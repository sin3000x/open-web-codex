use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use open_web_codex_platform_contracts::{
    ProviderCatalog, ProviderCredentialInput, UpdateProviderModelRequest, UpsertProviderRequest,
};
use open_web_codex_profile_registry::{ProfileRegistry, ProfileRegistryError};
use open_web_codex_secret_store::{PostgresSecretStore, SecretStoreError, SecretValue};
use serde_json::Value;
use sqlx::{PgPool, Row};
use thiserror::Error;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::{
    InMemoryProviderService, ProviderOperations, ProviderService, ProviderServiceError,
    ProviderTransport,
};

#[derive(Debug, Error)]
pub enum AuthorizedProviderError {
    #[error("Provider access is not authorized")]
    Forbidden,
    #[error(transparent)]
    Provider(#[from] ProviderServiceError),
    #[error("Profile Registry operation failed: {0}")]
    Registry(#[from] ProfileRegistryError),
    #[error("Provider Secret operation failed: {0}")]
    Secret(#[from] SecretStoreError),
    #[error("Provider authorization database operation failed: {0}")]
    Database(#[from] sqlx::Error),
}

#[async_trait]
pub trait AuthorizedProviderOperations: Send + Sync {
    async fn list(&self, actor: ProviderActor) -> Result<ProviderCatalog, AuthorizedProviderError>;
    async fn upsert(
        &self,
        actor: ProviderActor,
        id: &str,
        request: UpsertProviderRequest,
    ) -> Result<ProviderCatalog, AuthorizedProviderError>;
    async fn select(
        &self,
        actor: ProviderActor,
        id: &str,
    ) -> Result<ProviderCatalog, AuthorizedProviderError>;
    async fn select_model(
        &self,
        actor: ProviderActor,
        provider_id: &str,
        model_id: &str,
    ) -> Result<ProviderCatalog, AuthorizedProviderError>;
    async fn delete(
        &self,
        actor: ProviderActor,
        id: &str,
    ) -> Result<ProviderCatalog, AuthorizedProviderError>;
    async fn refresh_models(
        &self,
        actor: ProviderActor,
        id: &str,
    ) -> Result<ProviderCatalog, AuthorizedProviderError>;
    async fn update_model(
        &self,
        actor: ProviderActor,
        provider_id: &str,
        model_id: &str,
        request: UpdateProviderModelRequest,
    ) -> Result<ProviderCatalog, AuthorizedProviderError>;
}

#[derive(Clone, Copy, Debug)]
pub struct ProviderActor {
    pub user_id: Uuid,
    pub organization_id: Uuid,
}

struct RegistryProviderTransport {
    registry: ProfileRegistry,
    runtime_key: String,
}

#[async_trait]
impl ProviderTransport for RegistryProviderTransport {
    async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        let host = self
            .registry
            .host(&self.runtime_key)
            .await
            .map_err(|error| error.to_string())?;
        host.request(method, params)
            .await
            .map_err(|error| error.to_string())
    }
}

#[derive(Clone)]
pub struct SecuredProviderService {
    db: PgPool,
    runtime_key: String,
    registry: ProfileRegistry,
    secrets: PostgresSecretStore,
    runtime: ProviderService,
    operation: Arc<Mutex<()>>,
}

#[derive(Clone, Copy)]
struct AuthorizedProfile {
    id: Uuid,
    organization_id: Uuid,
}

impl SecuredProviderService {
    pub fn new(
        db: PgPool,
        runtime_key: impl Into<String>,
        registry: ProfileRegistry,
        secrets: PostgresSecretStore,
    ) -> Self {
        let runtime_key = runtime_key.into();
        let runtime = ProviderService::new(Arc::new(RegistryProviderTransport {
            registry: registry.clone(),
            runtime_key: runtime_key.clone(),
        }));
        Self {
            db,
            runtime_key,
            registry,
            secrets,
            runtime,
            operation: Arc::new(Mutex::new(())),
        }
    }

    /// Load persisted Provider Secrets before initially spawning the Profile.
    /// Values are returned only to the Profile Registry and must never be logged.
    pub async fn startup_secret_environment(
        &self,
    ) -> Result<BTreeMap<String, String>, AuthorizedProviderError> {
        let Some(profile) = self.profile_identity().await? else {
            return Ok(BTreeMap::new());
        };
        let values = self
            .secrets
            .list_provider_environment(profile.organization_id, profile.id)
            .await?;
        Ok(values
            .into_iter()
            .map(|value| (value.environment_key, value.value.expose().to_string()))
            .collect())
    }

    async fn authorize(
        &self,
        actor: ProviderActor,
    ) -> Result<AuthorizedProfile, AuthorizedProviderError> {
        let row = sqlx::query(
            "SELECT p.id, p.organization_id FROM profiles p \
             JOIN memberships m ON m.organization_id = p.organization_id AND m.user_id = $1 \
             WHERE p.owner_user_id = $1 AND p.organization_id = $2 \
               AND p.runtime_key = $3 AND p.status = 'active'",
        )
        .bind(actor.user_id)
        .bind(actor.organization_id)
        .bind(&self.runtime_key)
        .fetch_optional(&self.db)
        .await?
        .ok_or(AuthorizedProviderError::Forbidden)?;
        Ok(AuthorizedProfile {
            id: row.get("id"),
            organization_id: row.get("organization_id"),
        })
    }

    async fn profile_identity(&self) -> Result<Option<AuthorizedProfile>, AuthorizedProviderError> {
        let row = sqlx::query(
            "SELECT id, organization_id FROM profiles WHERE runtime_key = $1 AND status = 'active'",
        )
        .bind(&self.runtime_key)
        .fetch_optional(&self.db)
        .await?;
        Ok(row.map(|row| AuthorizedProfile {
            id: row.get("id"),
            organization_id: row.get("organization_id"),
        }))
    }

    async fn restore_secret(
        &self,
        profile: AuthorizedProfile,
        provider_id: &str,
        previous: Option<SecretValue>,
    ) -> Result<(), AuthorizedProviderError> {
        match previous {
            Some(previous) => {
                let environment_key = self
                    .secrets
                    .put_provider_key(profile.organization_id, profile.id, provider_id, &previous)
                    .await?;
                self.registry
                    .set_secret_environment(
                        &self.runtime_key,
                        &environment_key,
                        Some(previous.expose().to_string()),
                    )
                    .await?;
            }
            None => {
                let environment_key = self
                    .secrets
                    .delete_provider_key(profile.organization_id, profile.id, provider_id)
                    .await?;
                self.registry
                    .set_secret_environment(&self.runtime_key, &environment_key, None)
                    .await?;
            }
        }
        Ok(())
    }
}

#[async_trait]
impl AuthorizedProviderOperations for SecuredProviderService {
    async fn list(&self, actor: ProviderActor) -> Result<ProviderCatalog, AuthorizedProviderError> {
        self.authorize(actor).await?;
        Ok(self.runtime.list().await?)
    }

    async fn upsert(
        &self,
        actor: ProviderActor,
        id: &str,
        mut request: UpsertProviderRequest,
    ) -> Result<ProviderCatalog, AuthorizedProviderError> {
        let _operation = self.operation.lock().await;
        let profile = self.authorize(actor).await?;
        let credentials =
            std::mem::replace(&mut request.credentials, ProviderCredentialInput::Preserve);
        match credentials {
            ProviderCredentialInput::Direct { api_key } => {
                let previous = self
                    .secrets
                    .get_provider_key(profile.organization_id, profile.id, id)
                    .await?;
                let value = SecretValue::new(api_key)?;
                let environment_key = self
                    .secrets
                    .put_provider_key(profile.organization_id, profile.id, id, &value)
                    .await?;
                if let Err(error) = self
                    .registry
                    .set_secret_environment(
                        &self.runtime_key,
                        &environment_key,
                        Some(value.expose().to_string()),
                    )
                    .await
                {
                    self.restore_secret(profile, id, previous).await?;
                    return Err(error.into());
                }
                request.credentials = ProviderCredentialInput::Environment {
                    env_key: environment_key,
                };
                match self.runtime.upsert(id, request).await {
                    Ok(catalog) => Ok(catalog),
                    Err(error) => {
                        self.restore_secret(profile, id, previous).await?;
                        Err(error.into())
                    }
                }
            }
            credentials @ (ProviderCredentialInput::Environment { .. }
            | ProviderCredentialInput::NoCredential) => {
                request.credentials = credentials;
                let previous = self
                    .secrets
                    .get_provider_key(profile.organization_id, profile.id, id)
                    .await?;
                if previous.is_some() {
                    let environment_key = self
                        .secrets
                        .delete_provider_key(profile.organization_id, profile.id, id)
                        .await?;
                    if let Err(error) = self
                        .registry
                        .set_secret_environment(&self.runtime_key, &environment_key, None)
                        .await
                    {
                        self.restore_secret(profile, id, previous).await?;
                        return Err(error.into());
                    }
                }
                match self.runtime.upsert(id, request).await {
                    Ok(catalog) => Ok(catalog),
                    Err(error) => {
                        self.restore_secret(profile, id, previous).await?;
                        Err(error.into())
                    }
                }
            }
            ProviderCredentialInput::Preserve => Ok(self.runtime.upsert(id, request).await?),
        }
    }

    async fn select(
        &self,
        actor: ProviderActor,
        id: &str,
    ) -> Result<ProviderCatalog, AuthorizedProviderError> {
        self.authorize(actor).await?;
        Ok(self.runtime.select(id).await?)
    }

    async fn select_model(
        &self,
        actor: ProviderActor,
        provider_id: &str,
        model_id: &str,
    ) -> Result<ProviderCatalog, AuthorizedProviderError> {
        self.authorize(actor).await?;
        Ok(self.runtime.select_model(provider_id, model_id).await?)
    }

    async fn delete(
        &self,
        actor: ProviderActor,
        id: &str,
    ) -> Result<ProviderCatalog, AuthorizedProviderError> {
        let _operation = self.operation.lock().await;
        let profile = self.authorize(actor).await?;
        let previous = self
            .secrets
            .get_provider_key(profile.organization_id, profile.id, id)
            .await?;
        if previous.is_some() {
            let environment_key = self
                .secrets
                .delete_provider_key(profile.organization_id, profile.id, id)
                .await?;
            if let Err(error) = self
                .registry
                .set_secret_environment(&self.runtime_key, &environment_key, None)
                .await
            {
                self.restore_secret(profile, id, previous).await?;
                return Err(error.into());
            }
        }
        match self.runtime.delete(id).await {
            Ok(catalog) => Ok(catalog),
            Err(error) => {
                self.restore_secret(profile, id, previous).await?;
                Err(error.into())
            }
        }
    }

    async fn refresh_models(
        &self,
        actor: ProviderActor,
        id: &str,
    ) -> Result<ProviderCatalog, AuthorizedProviderError> {
        let _operation = self.operation.lock().await;
        self.authorize(actor).await?;
        let catalog = self.runtime.refresh_models(id).await?;
        self.registry
            .schedule_runtime_refresh(&self.runtime_key)
            .await?;
        Ok(catalog)
    }

    async fn update_model(
        &self,
        actor: ProviderActor,
        provider_id: &str,
        model_id: &str,
        request: UpdateProviderModelRequest,
    ) -> Result<ProviderCatalog, AuthorizedProviderError> {
        let _operation = self.operation.lock().await;
        self.authorize(actor).await?;
        let catalog = self
            .runtime
            .update_model(provider_id, model_id, request)
            .await?;
        self.registry
            .schedule_runtime_refresh(&self.runtime_key)
            .await?;
        Ok(catalog)
    }
}

#[derive(Clone, Default)]
pub struct InMemoryAuthorizedProviderService {
    inner: InMemoryProviderService,
}

#[async_trait]
impl AuthorizedProviderOperations for InMemoryAuthorizedProviderService {
    async fn list(
        &self,
        _actor: ProviderActor,
    ) -> Result<ProviderCatalog, AuthorizedProviderError> {
        Ok(self.inner.list().await?)
    }

    async fn upsert(
        &self,
        _actor: ProviderActor,
        id: &str,
        request: UpsertProviderRequest,
    ) -> Result<ProviderCatalog, AuthorizedProviderError> {
        Ok(self.inner.upsert(id, request).await?)
    }

    async fn select(
        &self,
        _actor: ProviderActor,
        id: &str,
    ) -> Result<ProviderCatalog, AuthorizedProviderError> {
        Ok(self.inner.select(id).await?)
    }

    async fn select_model(
        &self,
        _actor: ProviderActor,
        provider_id: &str,
        model_id: &str,
    ) -> Result<ProviderCatalog, AuthorizedProviderError> {
        Ok(self.inner.select_model(provider_id, model_id).await?)
    }

    async fn delete(
        &self,
        _actor: ProviderActor,
        id: &str,
    ) -> Result<ProviderCatalog, AuthorizedProviderError> {
        Ok(self.inner.delete(id).await?)
    }

    async fn refresh_models(
        &self,
        _actor: ProviderActor,
        id: &str,
    ) -> Result<ProviderCatalog, AuthorizedProviderError> {
        Ok(self.inner.refresh_models(id).await?)
    }

    async fn update_model(
        &self,
        _actor: ProviderActor,
        provider_id: &str,
        model_id: &str,
        request: UpdateProviderModelRequest,
    ) -> Result<ProviderCatalog, AuthorizedProviderError> {
        Ok(self
            .inner
            .update_model(provider_id, model_id, request)
            .await?)
    }
}
