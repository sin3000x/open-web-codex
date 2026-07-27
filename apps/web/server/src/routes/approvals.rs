use std::sync::Arc;

use axum::{extract::Path, http::StatusCode, Extension, Json};
use open_web_codex_adapter::CodexAdapter;
use open_web_codex_approval_service::{ApprovalActor, ApprovalService, ApprovalServiceError};
use open_web_codex_platform_contracts::error::PlatformError;
use open_web_codex_platform_contracts::{
    ApprovalSummary, DecideApprovalRequest, RespondUserInputRequest,
};
use uuid::Uuid;

use crate::middleware::auth::AuthenticatedUser;

type ApiError = (StatusCode, Json<PlatformError>);

pub async fn list_pending(
    auth: AuthenticatedUser,
    Extension(approvals): Extension<Arc<ApprovalService>>,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
) -> Result<Json<Vec<ApprovalSummary>>, ApiError> {
    let runtime_instance_id = adapter.runtime_instance_id().await;
    approvals
        .list_pending(actor(&auth), runtime_instance_id)
        .await
        .map(Json)
        .map_err(approval_error)
}

pub async fn decide(
    auth: AuthenticatedUser,
    Path(id): Path<Uuid>,
    Extension(approvals): Extension<Arc<ApprovalService>>,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
    Json(request): Json<DecideApprovalRequest>,
) -> Result<StatusCode, ApiError> {
    let actor = actor(&auth);
    let runtime_instance_id = adapter.runtime_instance_id().await;
    let dispatch = approvals
        .begin_decision(actor, id, runtime_instance_id, request)
        .await
        .map_err(approval_error)?;
    if adapter
        .respond_to_server_request(
            dispatch.runtime_instance_id,
            dispatch.runtime_request_id.clone(),
            dispatch.response.clone(),
        )
        .await
        .is_err()
    {
        approvals
            .mark_delivery_unknown(actor, &dispatch)
            .await
            .map_err(approval_error)?;
        return Err((
            StatusCode::BAD_GATEWAY,
            Json(PlatformError::internal(
                "Approval delivery status is unknown; inspect before retrying",
            )),
        ));
    }
    approvals
        .complete_decision(actor, &dispatch)
        .await
        .map_err(approval_error)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn respond_user_input(
    auth: AuthenticatedUser,
    Path(id): Path<Uuid>,
    Extension(approvals): Extension<Arc<ApprovalService>>,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
    Json(request): Json<RespondUserInputRequest>,
) -> Result<StatusCode, ApiError> {
    let actor = actor(&auth);
    let runtime_instance_id = adapter.runtime_instance_id().await;
    let dispatch = approvals
        .begin_user_input_response(actor, id, runtime_instance_id, request)
        .await
        .map_err(approval_error)?;
    if adapter
        .respond_to_server_request(
            dispatch.runtime_instance_id,
            dispatch.runtime_request_id.clone(),
            dispatch.response.clone(),
        )
        .await
        .is_err()
    {
        approvals
            .mark_delivery_unknown(actor, &dispatch)
            .await
            .map_err(approval_error)?;
        return Err((
            StatusCode::BAD_GATEWAY,
            Json(PlatformError::internal(
                "User input delivery status is unknown; inspect before retrying",
            )),
        ));
    }
    approvals
        .complete_decision(actor, &dispatch)
        .await
        .map_err(approval_error)?;
    Ok(StatusCode::NO_CONTENT)
}

fn actor(auth: &AuthenticatedUser) -> ApprovalActor {
    ApprovalActor {
        user_id: auth.user_id,
        organization_id: auth.organization_id,
    }
}

fn approval_error(error: ApprovalServiceError) -> ApiError {
    match error {
        ApprovalServiceError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(PlatformError::not_found("Approval was not found")),
        ),
        ApprovalServiceError::Conflict => (
            StatusCode::CONFLICT,
            Json(PlatformError::bad_request(
                "Approval was already decided or changed",
            )),
        ),
        ApprovalServiceError::Invalid => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(PlatformError::bad_request("Approval request is invalid")),
        ),
        ApprovalServiceError::Database(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(PlatformError::internal(
                "Approval service is temporarily unavailable",
            )),
        ),
    }
}
