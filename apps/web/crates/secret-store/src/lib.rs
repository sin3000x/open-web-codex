//! Encrypted platform Secret storage.
//!
//! Ciphertext is bound to Organization/Profile/Provider identity with AEAD
//! additional authenticated data. Plaintext values are intentionally neither
//! serializable nor printable and are zeroized when dropped.

use std::sync::Arc;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
use ring::digest::{digest, SHA256};
use ring::rand::{SecureRandom, SystemRandom};
use sqlx::{PgPool, Row};
use thiserror::Error;
use uuid::Uuid;
use zeroize::Zeroize;

const KEY_LENGTH: usize = 32;
const NONCE_LENGTH: usize = 12;
const PROVIDER_SECRET_PURPOSE: &str = "provider_api_key";
const CONFIGURATION_SECRET_PURPOSE: &str = "platform_configuration";

#[derive(Debug, Error)]
pub enum SecretStoreError {
    #[error("invalid platform master key")]
    InvalidMasterKey,
    #[error("invalid Provider secret identity")]
    InvalidIdentity,
    #[error("failed to encrypt platform Secret")]
    Encrypt,
    #[error("failed to decrypt platform Secret")]
    Decrypt,
    #[error("stored Secret uses unsupported key version '{0}'")]
    UnsupportedKeyVersion(String),
    #[error("platform Secret database operation failed: {0}")]
    Database(#[from] sqlx::Error),
}

pub struct SecretValue(String);

impl SecretValue {
    pub fn new(value: impl Into<String>) -> Result<Self, SecretStoreError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(SecretStoreError::InvalidIdentity);
        }
        Ok(Self(value))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for SecretValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SecretValue([redacted])")
    }
}

impl Drop for SecretValue {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

pub struct MasterKey([u8; KEY_LENGTH]);

impl MasterKey {
    pub fn from_base64(value: &str) -> Result<Self, SecretStoreError> {
        let decoded = STANDARD
            .decode(value.trim())
            .map_err(|_| SecretStoreError::InvalidMasterKey)?;
        let bytes: [u8; KEY_LENGTH] = decoded
            .try_into()
            .map_err(|_| SecretStoreError::InvalidMasterKey)?;
        Ok(Self(bytes))
    }

    pub fn generate() -> Result<Self, SecretStoreError> {
        let mut bytes = [0_u8; KEY_LENGTH];
        SystemRandom::new()
            .fill(&mut bytes)
            .map_err(|_| SecretStoreError::Encrypt)?;
        Ok(Self(bytes))
    }
}

impl std::fmt::Debug for MasterKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("MasterKey([redacted])")
    }
}

impl Drop for MasterKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[derive(Debug)]
pub struct EncryptedSecret {
    pub key_version: String,
    pub nonce: [u8; NONCE_LENGTH],
    pub ciphertext: Vec<u8>,
}

#[derive(Clone)]
pub struct SecretCipher {
    key: Arc<LessSafeKey>,
    key_version: Arc<str>,
}

impl std::fmt::Debug for SecretCipher {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SecretCipher")
            .field("key", &"[redacted]")
            .field("key_version", &self.key_version)
            .finish()
    }
}

impl SecretCipher {
    pub fn new(
        master_key: MasterKey,
        key_version: impl Into<String>,
    ) -> Result<Self, SecretStoreError> {
        let key = UnboundKey::new(&AES_256_GCM, &master_key.0)
            .map_err(|_| SecretStoreError::InvalidMasterKey)?;
        let key_version = key_version.into();
        if key_version.trim().is_empty() {
            return Err(SecretStoreError::InvalidMasterKey);
        }
        Ok(Self {
            key: Arc::new(LessSafeKey::new(key)),
            key_version: Arc::from(key_version),
        })
    }

    pub fn generate(key_version: impl Into<String>) -> Result<Self, SecretStoreError> {
        Self::new(MasterKey::generate()?, key_version)
    }

    pub fn seal(
        &self,
        authenticated_context: &[u8],
        value: &SecretValue,
    ) -> Result<EncryptedSecret, SecretStoreError> {
        let mut nonce = [0_u8; NONCE_LENGTH];
        SystemRandom::new()
            .fill(&mut nonce)
            .map_err(|_| SecretStoreError::Encrypt)?;
        let mut ciphertext = value.expose().as_bytes().to_vec();
        self.key
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(authenticated_context),
                &mut ciphertext,
            )
            .map_err(|_| SecretStoreError::Encrypt)?;
        Ok(EncryptedSecret {
            key_version: self.key_version.to_string(),
            nonce,
            ciphertext,
        })
    }

    pub fn open(
        &self,
        authenticated_context: &[u8],
        encrypted: EncryptedSecret,
    ) -> Result<SecretValue, SecretStoreError> {
        if encrypted.key_version != self.key_version.as_ref() {
            return Err(SecretStoreError::UnsupportedKeyVersion(
                encrypted.key_version,
            ));
        }
        let mut ciphertext = encrypted.ciphertext;
        let plaintext = self
            .key
            .open_in_place(
                Nonce::assume_unique_for_key(encrypted.nonce),
                Aad::from(authenticated_context),
                &mut ciphertext,
            )
            .map_err(|_| SecretStoreError::Decrypt)?;
        let plaintext = std::str::from_utf8(plaintext).map_err(|_| SecretStoreError::Decrypt)?;
        SecretValue::new(plaintext.to_string()).map_err(|_| SecretStoreError::Decrypt)
    }
}

#[derive(Debug)]
pub struct ProviderEnvironmentSecret {
    pub provider_id: String,
    pub environment_key: String,
    pub value: SecretValue,
}

#[derive(Debug)]
pub struct StoredConfigurationSecret {
    pub value: SecretValue,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone)]
pub struct PostgresSecretStore {
    db: PgPool,
    cipher: SecretCipher,
}

impl PostgresSecretStore {
    pub fn new(db: PgPool, cipher: SecretCipher) -> Self {
        Self { db, cipher }
    }

    pub async fn put_provider_key(
        &self,
        organization_id: Uuid,
        profile_id: Uuid,
        provider_id: &str,
        value: &SecretValue,
    ) -> Result<String, SecretStoreError> {
        validate_provider_id(provider_id)?;
        let context = provider_context(organization_id, profile_id, provider_id);
        let encrypted = self.cipher.seal(context.as_bytes(), value)?;
        sqlx::query(
            "INSERT INTO profile_secrets \
             (organization_id, profile_id, provider_id, purpose, key_version, nonce, ciphertext) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             ON CONFLICT (profile_id, provider_id, purpose) DO UPDATE SET \
             organization_id = EXCLUDED.organization_id, key_version = EXCLUDED.key_version, \
             nonce = EXCLUDED.nonce, ciphertext = EXCLUDED.ciphertext, updated_at = now()",
        )
        .bind(organization_id)
        .bind(profile_id)
        .bind(provider_id)
        .bind(PROVIDER_SECRET_PURPOSE)
        .bind(encrypted.key_version)
        .bind(encrypted.nonce.as_slice())
        .bind(encrypted.ciphertext)
        .execute(&self.db)
        .await?;
        Ok(provider_environment_key(profile_id, provider_id)?)
    }

    pub async fn get_provider_key(
        &self,
        organization_id: Uuid,
        profile_id: Uuid,
        provider_id: &str,
    ) -> Result<Option<SecretValue>, SecretStoreError> {
        validate_provider_id(provider_id)?;
        let row = sqlx::query(
            "SELECT key_version, nonce, ciphertext FROM profile_secrets \
             WHERE organization_id = $1 AND profile_id = $2 AND provider_id = $3 AND purpose = $4",
        )
        .bind(organization_id)
        .bind(profile_id)
        .bind(provider_id)
        .bind(PROVIDER_SECRET_PURPOSE)
        .fetch_optional(&self.db)
        .await?;
        row.map(|row| {
            let encrypted = encrypted_from_row(&row)?;
            let context = provider_context(organization_id, profile_id, provider_id);
            self.cipher.open(context.as_bytes(), encrypted)
        })
        .transpose()
    }

    pub async fn delete_provider_key(
        &self,
        organization_id: Uuid,
        profile_id: Uuid,
        provider_id: &str,
    ) -> Result<String, SecretStoreError> {
        validate_provider_id(provider_id)?;
        sqlx::query(
            "DELETE FROM profile_secrets WHERE organization_id = $1 AND profile_id = $2 \
             AND provider_id = $3 AND purpose = $4",
        )
        .bind(organization_id)
        .bind(profile_id)
        .bind(provider_id)
        .bind(PROVIDER_SECRET_PURPOSE)
        .execute(&self.db)
        .await?;
        provider_environment_key(profile_id, provider_id)
    }

    pub async fn list_provider_environment(
        &self,
        organization_id: Uuid,
        profile_id: Uuid,
    ) -> Result<Vec<ProviderEnvironmentSecret>, SecretStoreError> {
        let rows = sqlx::query(
            "SELECT provider_id, key_version, nonce, ciphertext FROM profile_secrets \
             WHERE organization_id = $1 AND profile_id = $2 AND purpose = $3 \
             ORDER BY provider_id",
        )
        .bind(organization_id)
        .bind(profile_id)
        .bind(PROVIDER_SECRET_PURPOSE)
        .fetch_all(&self.db)
        .await?;
        rows.iter()
            .map(|row| {
                let provider_id: String = row.get("provider_id");
                validate_provider_id(&provider_id)?;
                let encrypted = encrypted_from_row(row)?;
                let context = provider_context(organization_id, profile_id, &provider_id);
                let value = self.cipher.open(context.as_bytes(), encrypted)?;
                Ok(ProviderEnvironmentSecret {
                    environment_key: provider_environment_key(profile_id, &provider_id)?,
                    provider_id,
                    value,
                })
            })
            .collect()
    }

    /// Store a server-only setting in the centralized configuration namespace.
    /// The plaintext is never serialized into `platform_configuration`.
    pub async fn put_global_configuration_secret(
        &self,
        config_key: &str,
        value: &SecretValue,
        updated_by: Uuid,
    ) -> Result<chrono::DateTime<chrono::Utc>, SecretStoreError> {
        validate_configuration_key(config_key)?;
        let context = configuration_context("global", "global", config_key);
        let encrypted = self.cipher.seal(context.as_bytes(), value)?;
        let row = sqlx::query(
            "INSERT INTO platform_configuration_secrets \
             (scope_kind, scope_id, config_key, key_version, nonce, ciphertext, updated_by) \
             VALUES ('global', 'global', $1, $2, $3, $4, $5) \
             ON CONFLICT (scope_kind, scope_id, config_key) DO UPDATE SET \
             key_version = EXCLUDED.key_version, nonce = EXCLUDED.nonce, \
             ciphertext = EXCLUDED.ciphertext, updated_by = EXCLUDED.updated_by, \
             updated_at = now() \
             RETURNING updated_at",
        )
        .bind(config_key)
        .bind(encrypted.key_version)
        .bind(encrypted.nonce.as_slice())
        .bind(encrypted.ciphertext)
        .bind(updated_by)
        .fetch_one(&self.db)
        .await?;
        Ok(row.get("updated_at"))
    }

    pub async fn get_global_configuration_secret(
        &self,
        config_key: &str,
    ) -> Result<Option<StoredConfigurationSecret>, SecretStoreError> {
        validate_configuration_key(config_key)?;
        let row = sqlx::query(
            "SELECT key_version, nonce, ciphertext, updated_at \
             FROM platform_configuration_secrets \
             WHERE scope_kind = 'global' AND scope_id = 'global' AND config_key = $1",
        )
        .bind(config_key)
        .fetch_optional(&self.db)
        .await?;
        row.map(|row| {
            let encrypted = encrypted_from_row(&row)?;
            let context = configuration_context("global", "global", config_key);
            Ok(StoredConfigurationSecret {
                value: self.cipher.open(context.as_bytes(), encrypted)?,
                updated_at: row.get("updated_at"),
            })
        })
        .transpose()
    }
}

fn encrypted_from_row(row: &sqlx::postgres::PgRow) -> Result<EncryptedSecret, SecretStoreError> {
    let nonce: Vec<u8> = row.get("nonce");
    let nonce: [u8; NONCE_LENGTH] = nonce.try_into().map_err(|_| SecretStoreError::Decrypt)?;
    Ok(EncryptedSecret {
        key_version: row.get("key_version"),
        nonce,
        ciphertext: row.get("ciphertext"),
    })
}

fn provider_context(organization_id: Uuid, profile_id: Uuid, provider_id: &str) -> String {
    format!("open-web-codex:{organization_id}:{profile_id}:{PROVIDER_SECRET_PURPOSE}:{provider_id}")
}

fn configuration_context(scope_kind: &str, scope_id: &str, config_key: &str) -> String {
    format!("open-web-codex:{CONFIGURATION_SECRET_PURPOSE}:{scope_kind}:{scope_id}:{config_key}")
}

pub fn provider_environment_key(
    profile_id: Uuid,
    provider_id: &str,
) -> Result<String, SecretStoreError> {
    validate_provider_id(provider_id)?;
    let identity = format!("{profile_id}:{provider_id}");
    let hash = digest(&SHA256, identity.as_bytes());
    let suffix = hash.as_ref()[..12]
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<String>();
    Ok(format!("OPEN_WEB_CODEX_PROVIDER_{suffix}"))
}

fn validate_provider_id(provider_id: &str) -> Result<(), SecretStoreError> {
    if provider_id.trim().is_empty()
        || !provider_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(SecretStoreError::InvalidIdentity);
    }
    Ok(())
}

fn validate_configuration_key(config_key: &str) -> Result<(), SecretStoreError> {
    if config_key.trim().is_empty()
        || config_key.len() > 160
        || !config_key.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_')
        })
    {
        return Err(SecretStoreError::InvalidIdentity);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        configuration_context, provider_environment_key, validate_configuration_key,
        EncryptedSecret, SecretCipher, SecretStoreError, SecretValue,
    };
    use uuid::Uuid;

    #[test]
    fn encrypts_with_identity_bound_authenticated_data() {
        let cipher = SecretCipher::generate("test-v1").expect("generate cipher");
        let value = SecretValue::new("provider-secret").expect("Secret value");
        let encrypted = cipher
            .seal(b"org:profile:provider-a", &value)
            .expect("encrypt Secret");
        let opened = cipher
            .open(b"org:profile:provider-a", encrypted)
            .expect("decrypt Secret");

        assert_eq!(opened.expose(), "provider-secret");
        assert!(!format!("{opened:?}").contains("provider-secret"));
    }

    #[test]
    fn rejects_tampering_and_cross_provider_decryption() {
        let cipher = SecretCipher::generate("test-v1").expect("generate cipher");
        let value = SecretValue::new("provider-secret").expect("Secret value");
        let encrypted = cipher
            .seal(b"org:profile:provider-a", &value)
            .expect("encrypt Secret");
        let copied = EncryptedSecret {
            key_version: encrypted.key_version,
            nonce: encrypted.nonce,
            ciphertext: encrypted.ciphertext,
        };

        assert!(matches!(
            cipher.open(b"org:profile:provider-b", copied),
            Err(SecretStoreError::Decrypt)
        ));
    }

    #[test]
    fn configuration_secrets_use_a_distinct_validated_encryption_context() {
        assert!(validate_configuration_key("maps.google_api_key").is_ok());
        assert!(validate_configuration_key("../secret").is_err());
        assert_ne!(
            configuration_context("global", "global", "maps.google_api_key"),
            configuration_context("user", "user-1", "maps.google_api_key")
        );
    }

    #[test]
    fn derives_stable_scoped_environment_keys() {
        let first_profile = Uuid::now_v7();
        let second_profile = Uuid::now_v7();
        let first = provider_environment_key(first_profile, "deepseek").unwrap();

        assert_eq!(
            first,
            provider_environment_key(first_profile, "deepseek").unwrap()
        );
        assert_ne!(
            first,
            provider_environment_key(first_profile, "other").unwrap()
        );
        assert_ne!(
            first,
            provider_environment_key(second_profile, "deepseek").unwrap()
        );
        assert!(first.chars().all(|character| character.is_ascii_uppercase()
            || character.is_ascii_digit()
            || character == '_'));
    }
}
