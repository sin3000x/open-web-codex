use serde_json::Value;
use sqlx::{PgPool, Row};
use uuid::Uuid;

pub const GLOBAL_SCOPE_KIND: &str = "global";
pub const GLOBAL_SCOPE_ID: &str = "global";
pub const MODEL_SELECTION_CONFIG_KEY: &str = "models.default_selection";

#[derive(Debug, Clone)]
pub struct StoredConfiguration {
    pub value: Value,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Read one setting from the centralized configuration store.
pub async fn get_global(
    db: &PgPool,
    config_key: &str,
) -> Result<Option<StoredConfiguration>, sqlx::Error> {
    let row = sqlx::query(
        "SELECT config_value, updated_at FROM platform_configuration \
         WHERE scope_kind = $1 AND scope_id = $2 AND config_key = $3",
    )
    .bind(GLOBAL_SCOPE_KIND)
    .bind(GLOBAL_SCOPE_ID)
    .bind(config_key)
    .fetch_optional(db)
    .await?;

    Ok(row.map(|row| StoredConfiguration {
        value: row.get("config_value"),
        updated_at: row.get("updated_at"),
    }))
}

/// Upsert one setting in the global scope. Callers own key-specific validation
/// and must never place provider or other server-only Secrets in this store.
pub async fn put_global(
    db: &PgPool,
    config_key: &str,
    value: Value,
    updated_by: Uuid,
) -> Result<StoredConfiguration, sqlx::Error> {
    let row = sqlx::query(
        "INSERT INTO platform_configuration \
         (scope_kind, scope_id, config_key, config_value, updated_by) \
         VALUES ($1, $2, $3, $4, $5) \
         ON CONFLICT (scope_kind, scope_id, config_key) DO UPDATE SET \
         config_value = EXCLUDED.config_value, updated_by = EXCLUDED.updated_by, \
         updated_at = now() \
         RETURNING config_value, updated_at",
    )
    .bind(GLOBAL_SCOPE_KIND)
    .bind(GLOBAL_SCOPE_ID)
    .bind(config_key)
    .bind(value)
    .bind(updated_by)
    .fetch_one(db)
    .await?;

    Ok(StoredConfiguration {
        value: row.get("config_value"),
        updated_at: row.get("updated_at"),
    })
}
