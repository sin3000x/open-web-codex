//! Multi-Profile lifecycle registry for native Codex app-server processes.
//!
//! The registry owns mutable child-process secret environments and restarts a
//! Profile in place when those values change. Secret values are never returned
//! by its public APIs or included in Debug output.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use open_web_codex_profile_host::{ProfileHost, ProfileHostConfig, ProfileHostError};
use thiserror::Error;
use tokio::sync::{Mutex, RwLock};

#[derive(Debug, Error)]
pub enum ProfileRegistryError {
    #[error("Profile '{0}' is already registered")]
    AlreadyRegistered(String),
    #[error("Profile '{0}' is not registered")]
    NotFound(String),
    #[error("invalid Profile environment key")]
    InvalidEnvironmentKey,
    #[error("Profile Host failed: {0}")]
    Host(#[from] ProfileHostError),
    #[error(
        "Profile Host update failed and recovery also failed: update={update}; recovery={recovery}"
    )]
    HostRecovery {
        update: ProfileHostError,
        recovery: ProfileHostError,
    },
}

struct ManagedProfile {
    base_config: ProfileHostConfig,
    secret_environment: RwLock<BTreeMap<String, String>>,
    host: RwLock<Option<ProfileHost>>,
    operation: Mutex<()>,
}

impl std::fmt::Debug for ManagedProfile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ManagedProfile")
            .field("base_config", &self.base_config)
            .field("secret_environment", &"[redacted]")
            .finish_non_exhaustive()
    }
}

impl ManagedProfile {
    fn new(base_config: ProfileHostConfig, secret_environment: BTreeMap<String, String>) -> Self {
        Self {
            base_config,
            secret_environment: RwLock::new(secret_environment),
            host: RwLock::new(None),
            operation: Mutex::new(()),
        }
    }

    async fn effective_config(&self) -> ProfileHostConfig {
        let mut config = self.base_config.clone();
        for (key, value) in self.secret_environment.read().await.iter() {
            config = config.with_environment(key, value);
        }
        config
    }
}

#[derive(Clone, Default)]
pub struct ProfileRegistry {
    profiles: Arc<RwLock<HashMap<String, Arc<ManagedProfile>>>>,
}

impl ProfileRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn register(
        &self,
        config: ProfileHostConfig,
    ) -> Result<ProfileHost, ProfileRegistryError> {
        self.register_with_secret_environment(config, BTreeMap::new())
            .await
    }

    pub async fn register_with_secret_environment(
        &self,
        config: ProfileHostConfig,
        secret_environment: BTreeMap<String, String>,
    ) -> Result<ProfileHost, ProfileRegistryError> {
        for key in secret_environment.keys() {
            validate_environment_key(key)?;
        }
        let profile_id = config.profile_id.clone();
        let managed = Arc::new(ManagedProfile::new(config, secret_environment));
        {
            let mut profiles = self.profiles.write().await;
            if profiles.contains_key(&profile_id) {
                return Err(ProfileRegistryError::AlreadyRegistered(profile_id));
            }
            profiles.insert(profile_id.clone(), managed.clone());
        }

        let spawn_result = ProfileHost::spawn(managed.effective_config().await).await;
        match spawn_result {
            Ok(host) => {
                *managed.host.write().await = Some(host.clone());
                Ok(host)
            }
            Err(error) => {
                self.profiles.write().await.remove(&profile_id);
                Err(ProfileRegistryError::Host(error))
            }
        }
    }

    pub async fn host(&self, profile_id: &str) -> Result<ProfileHost, ProfileRegistryError> {
        let managed = self.managed(profile_id).await?;
        let host = managed
            .host
            .read()
            .await
            .clone()
            .ok_or_else(|| ProfileRegistryError::NotFound(profile_id.to_string()))?;
        Ok(host)
    }

    pub async fn set_secret_environment(
        &self,
        profile_id: &str,
        key: &str,
        value: Option<String>,
    ) -> Result<ProfileHost, ProfileRegistryError> {
        validate_environment_key(key)?;
        let managed = self.managed(profile_id).await?;
        let _operation = managed.operation.lock().await;
        let previous = {
            let mut environment = managed.secret_environment.write().await;
            let previous = environment.get(key).cloned();
            if previous == value {
                return managed
                    .host
                    .read()
                    .await
                    .clone()
                    .ok_or_else(|| ProfileRegistryError::NotFound(profile_id.to_string()));
            }
            match value {
                Some(value) => {
                    environment.insert(key.to_string(), value);
                }
                None => {
                    environment.remove(key);
                }
            }
            previous
        };
        let host = managed
            .host
            .read()
            .await
            .clone()
            .ok_or_else(|| ProfileRegistryError::NotFound(profile_id.to_string()))?;
        if let Err(update) = host.restart(managed.effective_config().await).await {
            {
                let mut environment = managed.secret_environment.write().await;
                match previous {
                    Some(previous) => {
                        environment.insert(key.to_string(), previous);
                    }
                    None => {
                        environment.remove(key);
                    }
                }
            }
            if let Err(recovery) = host.restart(managed.effective_config().await).await {
                return Err(ProfileRegistryError::HostRecovery { update, recovery });
            }
            return Err(ProfileRegistryError::Host(update));
        }
        Ok(host)
    }

    pub async fn secret_environment_keys(
        &self,
        profile_id: &str,
    ) -> Result<Vec<String>, ProfileRegistryError> {
        let managed = self.managed(profile_id).await?;
        let keys = managed
            .secret_environment
            .read()
            .await
            .keys()
            .cloned()
            .collect();
        Ok(keys)
    }

    /// Mark the owned app-server for replacement at the next safe
    /// server-controlled Turn boundary. Provider configuration is already
    /// persisted in the Profile home when this is called; the replacement
    /// process will rebuild its startup-scoped model catalog from that state.
    pub async fn schedule_runtime_refresh(
        &self,
        profile_id: &str,
    ) -> Result<ProfileHost, ProfileRegistryError> {
        let managed = self.managed(profile_id).await?;
        let _operation = managed.operation.lock().await;
        let host = managed
            .host
            .read()
            .await
            .clone()
            .ok_or_else(|| ProfileRegistryError::NotFound(profile_id.to_string()))?;
        host.schedule_restart(managed.effective_config().await)
            .await?;
        Ok(host)
    }

    pub async fn shutdown(&self, profile_id: &str) -> Result<(), ProfileRegistryError> {
        let managed = self.managed(profile_id).await?;
        let _operation = managed.operation.lock().await;
        if let Some(host) = managed.host.read().await.clone() {
            host.shutdown().await?;
        }
        Ok(())
    }

    async fn managed(&self, profile_id: &str) -> Result<Arc<ManagedProfile>, ProfileRegistryError> {
        self.profiles
            .read()
            .await
            .get(profile_id)
            .cloned()
            .ok_or_else(|| ProfileRegistryError::NotFound(profile_id.to_string()))
    }
}

fn validate_environment_key(key: &str) -> Result<(), ProfileRegistryError> {
    if key.is_empty()
        || !key.chars().all(|character| {
            character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
        })
    {
        return Err(ProfileRegistryError::InvalidEnvironmentKey);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{validate_environment_key, ManagedProfile, ProfileRegistryError};
    use open_web_codex_profile_host::ProfileHostConfig;

    #[test]
    fn validates_child_environment_keys() {
        assert!(validate_environment_key("OPEN_WEB_CODEX_PROVIDER_AB12").is_ok());
        assert!(matches!(
            validate_environment_key("bad-key"),
            Err(ProfileRegistryError::InvalidEnvironmentKey)
        ));
    }

    #[test]
    fn debug_output_redacts_registry_secret_environment() {
        let managed = ManagedProfile::new(
            ProfileHostConfig::new("profile", "/tmp/profile", "/tmp"),
            BTreeMap::new(),
        );
        let debug = format!("{managed:?}");

        assert!(debug.contains("[redacted]"));
    }
}
