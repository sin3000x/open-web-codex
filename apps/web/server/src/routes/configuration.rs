use std::sync::Arc;
use std::time::Duration;

use axum::{extract::State, http::StatusCode, Extension, Json};
use open_web_codex_platform_contracts::error::PlatformError;
use open_web_codex_platform_contracts::{
    MapsConfiguration, MapsProvider, UpdateMapsConfigurationRequest, UseMapsConfigurationRequest,
};
use open_web_codex_platform_store::{configuration, AppState};
use open_web_codex_secret_store::{
    PostgresSecretStore, SecretStoreError, SecretValue, StoredConfigurationSecret,
};
use serde::{Deserialize, Serialize};

use crate::middleware::auth::AuthenticatedUser;

const ACTIVE_MAPS_CREDENTIAL_KEY: &str = "maps.active_credential";
const LEGACY_MAPBOX_ACCESS_TOKEN_KEY: &str = "maps.mapbox_public_access_token";
const LEGACY_GOOGLE_MAPS_API_KEY: &str = "maps.google_api_key";
const MAX_MAPS_KEY_LENGTH: usize = 4096;

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<PlatformError>)>;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredMapsCredential {
    provider: MapsProvider,
    api_key: String,
}

struct LoadedMapsCredential {
    credential: StoredMapsCredential,
    updated_at: chrono::DateTime<chrono::Utc>,
}

/// GET /api/configuration/maps — return the single active provider status.
/// Google credentials remain server-only; an active Mapbox public token is
/// returned because Mapbox GL needs it in the browser.
pub async fn get_maps(
    State(state): State<AppState>,
    Extension(secrets): Extension<Arc<PostgresSecretStore>>,
    auth: AuthenticatedUser,
) -> ApiResult<MapsConfiguration> {
    let loaded = load_maps_credential(&state, &secrets).await?;
    Ok(Json(maps_configuration(loaded, can_configure(&auth))))
}

/// PUT /api/configuration/maps — replace the active provider and key.
pub async fn update_maps(
    State(state): State<AppState>,
    Extension(secrets): Extension<Arc<PostgresSecretStore>>,
    auth: AuthenticatedUser,
    Json(request): Json<UpdateMapsConfigurationRequest>,
) -> ApiResult<MapsConfiguration> {
    if !can_configure(&auth) {
        return Err((
            StatusCode::FORBIDDEN,
            Json(PlatformError::forbidden(
                "only Organization owners and admins can configure maps",
            )),
        ));
    }
    let api_key = validate_maps_key(request.provider, &request.api_key)?;
    let credential = StoredMapsCredential {
        provider: request.provider,
        api_key,
    };
    let encoded = serde_json::to_string(&credential).map_err(|_| maps_configuration_error())?;
    let secret = SecretValue::new(encoded).map_err(secret_store_error)?;
    let updated_at = secrets
        .put_global_configuration_secret(ACTIVE_MAPS_CREDENTIAL_KEY, &secret, auth.user_id)
        .await
        .map_err(secret_store_error)?;

    // The active credential is authoritative. Remove old provider-specific
    // rows after a successful replacement so later reads cannot revive them.
    cleanup_legacy_maps_configuration(&state).await;

    if let Some(url) = request.elicitation_url.as_deref() {
        submit_key_to_elicitation(url, request.provider, &credential.api_key).await?;
    }
    audit_configuration_update(&state, &auth).await;

    Ok(Json(maps_configuration(
        Some(LoadedMapsCredential {
            credential,
            updated_at,
        }),
        true,
    )))
}

/// POST /api/configuration/maps/use — deliver the selected provider/key to a
/// pending local MCP request without exposing the key to the browser.
pub async fn use_maps(
    State(state): State<AppState>,
    Extension(secrets): Extension<Arc<PostgresSecretStore>>,
    auth: AuthenticatedUser,
    Json(request): Json<UseMapsConfigurationRequest>,
) -> ApiResult<MapsConfiguration> {
    let loaded = load_maps_credential(&state, &secrets)
        .await?
        .ok_or_else(|| {
            (
                StatusCode::CONFLICT,
                Json(PlatformError::bad_request(
                    "Mapbox or Google Maps must be configured before running map tools",
                )),
            )
        })?;
    submit_key_to_elicitation(
        &request.elicitation_url,
        loaded.credential.provider,
        &loaded.credential.api_key,
    )
    .await?;
    Ok(Json(maps_configuration(Some(loaded), can_configure(&auth))))
}

async fn load_maps_credential(
    state: &AppState,
    secrets: &PostgresSecretStore,
) -> Result<Option<LoadedMapsCredential>, (StatusCode, Json<PlatformError>)> {
    if let Some(stored) = secrets
        .get_global_configuration_secret(ACTIVE_MAPS_CREDENTIAL_KEY)
        .await
        .map_err(secret_store_error)?
    {
        return decode_maps_credential(stored).map(Some);
    }

    // Compatibility with global rows written before the provider selection was
    // unified. The next successful save replaces and removes these rows.
    if let Some(stored) = configuration::get_global(&state.db, LEGACY_MAPBOX_ACCESS_TOKEN_KEY)
        .await
        .map_err(configuration_database_error)?
    {
        if let Some(api_key) = stored
            .value
            .get("accessToken")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.trim().is_empty())
        {
            return Ok(Some(LoadedMapsCredential {
                credential: StoredMapsCredential {
                    provider: MapsProvider::Mapbox,
                    api_key: api_key.trim().to_string(),
                },
                updated_at: stored.updated_at,
            }));
        }
    }

    let legacy_google = secrets
        .get_global_configuration_secret(LEGACY_GOOGLE_MAPS_API_KEY)
        .await
        .map_err(secret_store_error)?;
    Ok(legacy_google.map(|stored| LoadedMapsCredential {
        credential: StoredMapsCredential {
            provider: MapsProvider::Google,
            api_key: stored.value.expose().to_string(),
        },
        updated_at: stored.updated_at,
    }))
}

fn decode_maps_credential(
    stored: StoredConfigurationSecret,
) -> Result<LoadedMapsCredential, (StatusCode, Json<PlatformError>)> {
    let credential = serde_json::from_str::<StoredMapsCredential>(stored.value.expose())
        .map_err(|_| maps_configuration_error())?;
    validate_maps_key(credential.provider, &credential.api_key)?;
    Ok(LoadedMapsCredential {
        credential,
        updated_at: stored.updated_at,
    })
}

fn maps_configuration(
    loaded: Option<LoadedMapsCredential>,
    can_configure: bool,
) -> MapsConfiguration {
    let Some(loaded) = loaded else {
        return MapsConfiguration {
            configured: false,
            provider: None,
            mapbox_access_token: None,
            can_configure,
            updated_at: None,
        };
    };
    let mapbox_access_token = (loaded.credential.provider == MapsProvider::Mapbox)
        .then(|| loaded.credential.api_key.clone());
    MapsConfiguration {
        configured: true,
        provider: Some(loaded.credential.provider),
        mapbox_access_token,
        can_configure,
        updated_at: Some(loaded.updated_at),
    }
}

fn validate_maps_key(
    provider: MapsProvider,
    candidate: &str,
) -> Result<String, (StatusCode, Json<PlatformError>)> {
    let key = candidate.trim();
    let invalid_common = key.is_empty()
        || key.len() > MAX_MAPS_KEY_LENGTH
        || key.chars().any(char::is_whitespace)
        || key.chars().any(char::is_control);
    let invalid_mapbox = provider == MapsProvider::Mapbox && !key.starts_with("pk.");
    if invalid_common || invalid_mapbox {
        let message = match provider {
            MapsProvider::Mapbox => {
                "Mapbox access token must be a public browser token beginning with 'pk.'"
            }
            MapsProvider::Google => {
                "Google Maps API key must be non-empty and contain no whitespace"
            }
        };
        return Err((
            StatusCode::BAD_REQUEST,
            Json(PlatformError::bad_request(message)),
        ));
    }
    Ok(key.to_string())
}

pub(crate) fn safe_maps_credential_url(value: &str) -> Option<&str> {
    let parsed = url::Url::parse(value).ok()?;
    if parsed.scheme() != "http"
        || parsed.host_str() != Some("127.0.0.1")
        || parsed.port().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || parsed.path().len() <= 1
    {
        return None;
    }
    Some(value)
}

async fn submit_key_to_elicitation(
    candidate_url: &str,
    provider: MapsProvider,
    api_key: &str,
) -> Result<(), (StatusCode, Json<PlatformError>)> {
    let url = safe_maps_credential_url(candidate_url).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(PlatformError::bad_request(
                "Maps credential request URL is invalid",
            )),
        )
    })?;
    let provider = match provider {
        MapsProvider::Mapbox => "mapbox",
        MapsProvider::Google => "google",
    };
    let body = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("provider", provider)
        .append_pair("api_key", api_key)
        .append_pair("remember", "yes")
        .finish();
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(3))
        .build()
        .map_err(|_| credential_delivery_error())?;
    let response = client
        .post(url)
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(body)
        .send()
        .await
        .map_err(|_| credential_delivery_error())?;
    if !response.status().is_success() {
        return Err(credential_delivery_error());
    }
    Ok(())
}

async fn cleanup_legacy_maps_configuration(state: &AppState) {
    let _ = sqlx::query(
        "DELETE FROM platform_configuration \
         WHERE scope_kind = $1 AND scope_id = $2 AND config_key = $3",
    )
    .bind(configuration::GLOBAL_SCOPE_KIND)
    .bind(configuration::GLOBAL_SCOPE_ID)
    .bind(LEGACY_MAPBOX_ACCESS_TOKEN_KEY)
    .execute(&state.db)
    .await;
    let _ = sqlx::query(
        "DELETE FROM platform_configuration_secrets \
         WHERE scope_kind = $1 AND scope_id = $2 AND config_key = $3",
    )
    .bind(configuration::GLOBAL_SCOPE_KIND)
    .bind(configuration::GLOBAL_SCOPE_ID)
    .bind(LEGACY_GOOGLE_MAPS_API_KEY)
    .execute(&state.db)
    .await;
}

async fn audit_configuration_update(state: &AppState, auth: &AuthenticatedUser) {
    let _ = sqlx::query(
        "INSERT INTO audit_log \
         (actor_id, organization_id, action, target_type, metadata) \
         VALUES ($1, $2, 'platform_configuration.updated', 'platform_configuration', $3)",
    )
    .bind(auth.user_id)
    .bind(auth.organization_id)
    .bind(serde_json::json!({
        "scopeKind": configuration::GLOBAL_SCOPE_KIND,
        "scopeId": configuration::GLOBAL_SCOPE_ID,
        "configKey": ACTIVE_MAPS_CREDENTIAL_KEY,
        "secret": true,
    }))
    .execute(&state.db)
    .await;
}

fn can_configure(auth: &AuthenticatedUser) -> bool {
    matches!(auth.organization_role.as_str(), "owner" | "admin")
}

fn configuration_database_error(_error: sqlx::Error) -> (StatusCode, Json<PlatformError>) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(PlatformError::internal(
            "platform configuration is temporarily unavailable",
        )),
    )
}

fn secret_store_error(_error: SecretStoreError) -> (StatusCode, Json<PlatformError>) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(PlatformError::internal(
            "encrypted platform configuration is temporarily unavailable",
        )),
    )
}

fn maps_configuration_error() -> (StatusCode, Json<PlatformError>) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(PlatformError::internal(
            "stored maps configuration is invalid; save it again",
        )),
    )
}

fn credential_delivery_error() -> (StatusCode, Json<PlatformError>) {
    (
        StatusCode::BAD_GATEWAY,
        Json(PlatformError::internal(
            "Maps configuration could not be delivered to the pending request; retry it",
        )),
    )
}

#[cfg(test)]
mod tests {
    use super::{safe_maps_credential_url, validate_maps_key};
    use axum::http::StatusCode;
    use open_web_codex_platform_contracts::MapsProvider;

    #[test]
    fn validates_provider_specific_keys() {
        assert_eq!(
            validate_maps_key(MapsProvider::Mapbox, "  pk.public-token  ").unwrap(),
            "pk.public-token"
        );
        assert_eq!(
            validate_maps_key(MapsProvider::Mapbox, "sk.secret-token")
                .unwrap_err()
                .0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            validate_maps_key(MapsProvider::Google, "  AIza-test-key  ").unwrap(),
            "AIza-test-key"
        );
        assert_eq!(
            validate_maps_key(MapsProvider::Google, "key with space")
                .unwrap_err()
                .0,
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn accepts_only_tokenized_loopback_credential_urls() {
        assert_eq!(
            safe_maps_credential_url("http://127.0.0.1:43123/one-time-token"),
            Some("http://127.0.0.1:43123/one-time-token")
        );
        assert_eq!(
            safe_maps_credential_url("https://example.com/credential"),
            None
        );
        assert_eq!(
            safe_maps_credential_url("http://localhost:43123/credential"),
            None
        );
    }
}
