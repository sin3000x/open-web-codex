use std::sync::Arc;

use axum::extract::State;
use axum::{extract::Path, http::StatusCode, Extension, Json};
use open_web_codex_platform_contracts::error::{ErrorKind, PlatformError};
use open_web_codex_platform_contracts::{
    ModelSelection, ProviderCatalog, UpdateProviderModelRequest, UpsertProviderRequest,
};
use open_web_codex_platform_store::configuration::{
    get_global, put_global, MODEL_SELECTION_CONFIG_KEY,
};
use open_web_codex_platform_store::AppState;
use open_web_codex_provider_service::secured::{
    AuthorizedProviderError, AuthorizedProviderOperations, ProviderActor,
};
use open_web_codex_provider_service::ProviderServiceError;

use crate::middleware::auth::AuthenticatedUser;

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<PlatformError>)>;

/// GET /api/providers — return a credential-safe Provider catalog.
pub async fn list_providers(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Extension(providers): Extension<Arc<dyn AuthorizedProviderOperations>>,
) -> ApiResult<ProviderCatalog> {
    let mut catalog = providers
        .list(provider_actor(&auth))
        .await
        .map_err(provider_error)?;
    apply_persisted_selection(&state, &mut catalog).await?;
    Ok(Json(catalog))
}

/// PUT /api/providers/:id — create or update a custom Provider.
pub async fn upsert_provider(
    auth: AuthenticatedUser,
    Path(id): Path<String>,
    Extension(providers): Extension<Arc<dyn AuthorizedProviderOperations>>,
    Json(request): Json<UpsertProviderRequest>,
) -> ApiResult<ProviderCatalog> {
    providers
        .upsert(provider_actor(&auth), &id, request)
        .await
        .map(Json)
        .map_err(provider_error)
}

/// POST /api/providers/:id/select — select a configured Provider.
pub async fn select_provider(
    auth: AuthenticatedUser,
    Path(id): Path<String>,
    Extension(providers): Extension<Arc<dyn AuthorizedProviderOperations>>,
) -> ApiResult<ProviderCatalog> {
    providers
        .select(provider_actor(&auth), &id)
        .await
        .map(Json)
        .map_err(provider_error)
}

/// POST /api/providers/:provider_id/models/:model_id/select — persist the
/// platform-wide default Provider/model pair and update the active Profile.
pub async fn select_provider_model(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path((provider_id, model_id)): Path<(String, String)>,
    Extension(providers): Extension<Arc<dyn AuthorizedProviderOperations>>,
) -> ApiResult<ProviderCatalog> {
    let mut catalog = providers
        .select_model(provider_actor(&auth), &provider_id, &model_id)
        .await
        .map_err(provider_error)?;
    put_global(
        &state.db,
        MODEL_SELECTION_CONFIG_KEY,
        serde_json::to_value(ModelSelection {
            provider_id: provider_id.clone(),
            model_id: model_id.clone(),
        })
        .map_err(|_| internal_error("Model selection could not be encoded"))?,
        auth.user_id,
    )
    .await
    .map_err(|_| internal_error("Model selection could not be persisted"))?;
    catalog.current_provider_id = provider_id;
    catalog.current_model_id = Some(model_id);
    Ok(Json(catalog))
}

/// DELETE /api/providers/:id — remove a non-current custom Provider.
pub async fn delete_provider(
    auth: AuthenticatedUser,
    Path(id): Path<String>,
    Extension(providers): Extension<Arc<dyn AuthorizedProviderOperations>>,
) -> ApiResult<ProviderCatalog> {
    providers
        .delete(provider_actor(&auth), &id)
        .await
        .map(Json)
        .map_err(provider_error)
}

/// POST /api/providers/:id/models/refresh — refresh the Provider-scoped catalog.
pub async fn refresh_provider_models(
    auth: AuthenticatedUser,
    Path(id): Path<String>,
    Extension(providers): Extension<Arc<dyn AuthorizedProviderOperations>>,
) -> ApiResult<ProviderCatalog> {
    providers
        .refresh_models(provider_actor(&auth), &id)
        .await
        .map(Json)
        .map_err(provider_error)
}

/// PATCH /api/providers/:provider_id/models/:model_id — update platform-editable
/// model metadata without exposing Codex configuration paths.
pub async fn update_provider_model(
    auth: AuthenticatedUser,
    Path((provider_id, model_id)): Path<(String, String)>,
    Extension(providers): Extension<Arc<dyn AuthorizedProviderOperations>>,
    Json(request): Json<UpdateProviderModelRequest>,
) -> ApiResult<ProviderCatalog> {
    providers
        .update_model(provider_actor(&auth), &provider_id, &model_id, request)
        .await
        .map(Json)
        .map_err(provider_error)
}

fn provider_error(error: AuthorizedProviderError) -> (StatusCode, Json<PlatformError>) {
    match error {
        AuthorizedProviderError::Forbidden => (
            StatusCode::NOT_FOUND,
            Json(PlatformError::not_found("Provider Profile was not found")),
        ),
        AuthorizedProviderError::Provider(error) => provider_service_error(error),
        AuthorizedProviderError::Registry(_)
        | AuthorizedProviderError::Secret(_)
        | AuthorizedProviderError::Database(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(PlatformError::internal(
                "Provider service is temporarily unavailable",
            )),
        ),
    }
}

fn provider_actor(auth: &AuthenticatedUser) -> ProviderActor {
    ProviderActor {
        user_id: auth.user_id,
        organization_id: auth.organization_id,
    }
}

async fn apply_persisted_selection(
    state: &AppState,
    catalog: &mut ProviderCatalog,
) -> Result<(), (StatusCode, Json<PlatformError>)> {
    let stored = get_global(&state.db, MODEL_SELECTION_CONFIG_KEY)
        .await
        .map_err(|_| internal_error("Model selection could not be loaded"))?;
    let Some(stored) = stored else {
        return Ok(());
    };
    let Ok(selection) = serde_json::from_value::<ModelSelection>(stored.value) else {
        return Ok(());
    };
    if selection.provider_id == catalog.current_provider_id
        && catalog.data.iter().any(|provider| {
            provider.id == selection.provider_id
                && (provider.models.is_empty()
                    || provider
                        .models
                        .iter()
                        .any(|model| model.model_id == selection.model_id && model.show_in_picker))
        })
    {
        catalog.current_model_id = Some(selection.model_id);
    }
    Ok(())
}

fn internal_error(message: &str) -> (StatusCode, Json<PlatformError>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(PlatformError::internal(message)),
    )
}

fn provider_service_error(error: ProviderServiceError) -> (StatusCode, Json<PlatformError>) {
    let (status, error) = match error {
        ProviderServiceError::InvalidInput(message) => {
            (StatusCode::BAD_REQUEST, PlatformError::bad_request(message))
        }
        ProviderServiceError::NotFound(message) => {
            (StatusCode::NOT_FOUND, PlatformError::not_found(message))
        }
        ProviderServiceError::Forbidden(message) => {
            (StatusCode::FORBIDDEN, PlatformError::forbidden(message))
        }
        ProviderServiceError::Runtime(_) => (
            StatusCode::BAD_GATEWAY,
            PlatformError {
                kind: ErrorKind::CodexRejected,
                message: "Codex rejected the Provider operation".to_string(),
                request_id: None,
                retry_after_ms: None,
            },
        ),
        ProviderServiceError::InvalidResponse(_) => (
            StatusCode::BAD_GATEWAY,
            PlatformError {
                kind: ErrorKind::CodexRejected,
                message: "Codex returned an invalid Provider response".to_string(),
                request_id: None,
                retry_after_ms: None,
            },
        ),
    };
    (status, Json(error))
}

#[cfg(test)]
mod tests {
    use super::provider_error;
    use axum::http::StatusCode;
    use open_web_codex_platform_contracts::error::ErrorKind;
    use open_web_codex_provider_service::{secured::AuthorizedProviderError, ProviderServiceError};

    #[test]
    fn runtime_failures_do_not_expose_runtime_details() {
        let secret = "secret-value-must-not-leak";
        let (status, error) = provider_error(AuthorizedProviderError::Provider(
            ProviderServiceError::Runtime(secret.to_string()),
        ));

        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert_eq!(error.kind, ErrorKind::CodexRejected);
        assert!(!error.message.contains(secret));
    }
}
