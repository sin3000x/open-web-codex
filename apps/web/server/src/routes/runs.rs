use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use open_web_codex_adapter::{
    AuthorizedWorkspace, CodexAdapter, ReviewTarget as AdapterReviewTarget,
};
use open_web_codex_platform_contracts::error::PlatformError;
use open_web_codex_platform_contracts::{
    InterruptRunRequest, ReviewTarget as PlatformReviewTarget, Run, StartReviewRequest,
    StartRunRequest, StartRunResponse, SteerRunRequest,
};
use open_web_codex_platform_store::AppState;
use open_web_codex_run_orchestrator::{
    CancelRunRequest, EnqueueRunRequest, RunOrchestrator, RunOrchestratorError, RunRecord,
};
use sqlx::Row;
use uuid::Uuid;

use crate::middleware::auth::{require_runtime_profile, AuthenticatedUser};
use crate::routes::RuntimeProfileBinding;

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<PlatformError>)>;

/// Queue a Run. A worker owns all Git and Runtime side effects after this
/// transaction, so retries are safe when the caller reuses its idempotency key.
pub async fn start_run(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(task_id): Path<Uuid>,
    Extension(orchestrator): Extension<Arc<RunOrchestrator>>,
    Extension(profile): Extension<RuntimeProfileBinding>,
    Json(req): Json<StartRunRequest>,
) -> ApiResult<StartRunResponse> {
    require_runtime_profile(&state.db, &auth, &profile.runtime_key).await?;
    let run = orchestrator
        .enqueue_run(EnqueueRunRequest {
            organization_id: auth.organization_id,
            actor_id: auth.user_id,
            task_id,
            idempotency_key: req.idempotency_key,
            git_ref: req.git_ref,
            workspace_kind: req.workspace_kind.unwrap_or_else(|| "main".to_string()),
            workspace_name: req.workspace_name,
            workspace_parent_run_id: req.workspace_parent_run_id,
            workspace_group_run_id: req.workspace_group_run_id,
            copy_agents_md: req.copy_agents_md,
            fork_thread_id: req.fork_thread_id,
            fork_source_run_id: req.fork_source_run_id,
        })
        .await
        .map_err(orchestrator_error)?;
    Ok(Json(StartRunResponse {
        run: run_from_record(run),
    }))
}

/// GET /api/runs?task_id=... — list runs for a task.
pub async fn list_runs(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, Uuid>>,
) -> ApiResult<Vec<Run>> {
    let task_id = params.get("task_id").copied();
    let rows = if let Some(task_id) = task_id {
        sqlx::query(
            "SELECT id, task_id, status, codex_thread_id, active_turn_id, workspace_id, \
                    source_ref, workspace_kind, workspace_name, workspace_parent_run_id, \
                    workspace_group_run_id, \
                    attempt, created_at, updated_at FROM runs \
             WHERE task_id = $1 AND organization_id = $2 ORDER BY created_at DESC",
        )
        .bind(task_id)
        .bind(auth.organization_id)
        .fetch_all(&state.db)
        .await
        .map_err(database_error)?
    } else {
        sqlx::query(
            "SELECT id, task_id, status, codex_thread_id, active_turn_id, workspace_id, \
                    source_ref, workspace_kind, workspace_name, workspace_parent_run_id, \
                    workspace_group_run_id, \
                    attempt, created_at, updated_at FROM runs \
             WHERE organization_id = $1 ORDER BY created_at DESC",
        )
        .bind(auth.organization_id)
        .fetch_all(&state.db)
        .await
        .map_err(database_error)?
    };
    Ok(Json(rows.iter().map(run_from_row).collect()))
}

/// GET /api/runs/:id — get a single run.
pub async fn get_run(
    auth: AuthenticatedUser,
    Path(id): Path<Uuid>,
    Extension(orchestrator): Extension<Arc<RunOrchestrator>>,
) -> ApiResult<Run> {
    let run = orchestrator
        .get_run(auth.organization_id, id)
        .await
        .map_err(orchestrator_error)?;
    Ok(Json(run_from_record(run)))
}

/// Cancel a Run and interrupt its projected active Turn when one exists.
pub async fn cancel_run(
    auth: AuthenticatedUser,
    Path(id): Path<Uuid>,
    Extension(orchestrator): Extension<Arc<RunOrchestrator>>,
) -> ApiResult<Run> {
    let run = orchestrator
        .cancel_run(CancelRunRequest {
            organization_id: auth.organization_id,
            actor_id: auth.user_id,
            allow_organization_admin: matches!(auth.organization_role.as_str(), "owner" | "admin"),
            run_id: id,
        })
        .await
        .map_err(orchestrator_error)?;
    Ok(Json(run_from_record(run)))
}

/// Interrupt the active Turn without cancelling its durable Run.
pub async fn interrupt_run(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(id): Path<Uuid>,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
    Json(request): Json<InterruptRunRequest>,
) -> ApiResult<serde_json::Value> {
    let context = authorized_turn_context(&state, &auth, id).await?;
    if context.turn_id != request.turn_id {
        return Err((
            StatusCode::CONFLICT,
            Json(PlatformError::bad_request(
                "active Turn changed; reload before interrupting",
            )),
        ));
    }
    adapter
        .interrupt_turn(&context.workspace, &context.thread_id, &request.turn_id)
        .await
        .map_err(adapter_control_error)?;
    sqlx::query(
        "UPDATE runs SET active_turn_id = NULL, updated_at = now() \
         WHERE id = $1 AND organization_id = $2 AND active_turn_id = $3",
    )
    .bind(id)
    .bind(auth.organization_id)
    .bind(&request.turn_id)
    .execute(&state.db)
    .await
    .map_err(database_error)?;
    Ok(Json(serde_json::json!({ "status": "interrupted" })))
}

/// Add a follow-up to the active Turn while enforcing the projected Turn id.
pub async fn steer_run(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(id): Path<Uuid>,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
    Json(request): Json<SteerRunRequest>,
) -> ApiResult<serde_json::Value> {
    if request.text.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(PlatformError::bad_request("message text must not be empty")),
        ));
    }
    let context = authorized_turn_context(&state, &auth, id).await?;
    if context.turn_id != request.turn_id {
        return Err((
            StatusCode::CONFLICT,
            Json(PlatformError::bad_request(
                "active Turn changed; reload before steering",
            )),
        ));
    }
    let result = adapter
        .steer_turn(
            &context.workspace,
            &context.thread_id,
            &request.turn_id,
            &request.text,
            &request.images,
        )
        .await
        .map_err(adapter_control_error)?;
    Ok(Json(
        serde_json::json!({ "status": "steered", "result": result }),
    ))
}

pub async fn compact_run_thread(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(id): Path<Uuid>,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
) -> ApiResult<serde_json::Value> {
    let context = authorized_thread_context(&state, &auth, id).await?;
    if context.turn_id.is_some() {
        return Err((
            StatusCode::CONFLICT,
            Json(PlatformError::bad_request(
                "interrupt the active Turn before compacting",
            )),
        ));
    }
    adapter
        .compact_thread(&context.workspace, &context.thread_id)
        .await
        .map_err(adapter_control_error)?;
    Ok(Json(serde_json::json!({ "status": "compacting" })))
}

pub async fn start_review(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(id): Path<Uuid>,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
    Json(request): Json<StartReviewRequest>,
) -> ApiResult<serde_json::Value> {
    if request
        .delivery
        .as_deref()
        .is_some_and(|value| value != "inline")
    {
        return Err((
            StatusCode::CONFLICT,
            Json(PlatformError::bad_request(
                "detached reviews require a durable child Run and are not enabled",
            )),
        ));
    }
    let context = authorized_thread_context(&state, &auth, id).await?;
    if context.turn_id.is_some() {
        return Err((
            StatusCode::CONFLICT,
            Json(PlatformError::bad_request(
                "interrupt the active Turn before starting review",
            )),
        ));
    }
    let target = review_target(request.target)?;
    let result = adapter
        .start_review(&context.workspace, &context.thread_id, target)
        .await
        .map_err(adapter_control_error)?;
    if let Some(turn_id) = result
        .pointer("/turn/id")
        .and_then(serde_json::Value::as_str)
    {
        sqlx::query(
            "UPDATE runs SET active_turn_id = $1, updated_at = now() \
             WHERE id = $2 AND organization_id = $3 AND status = 'running'",
        )
        .bind(turn_id)
        .bind(id)
        .bind(auth.organization_id)
        .execute(&state.db)
        .await
        .map_err(database_error)?;
    }
    Ok(Json(result))
}

struct AuthorizedTurnContext {
    workspace: AuthorizedWorkspace,
    thread_id: String,
    turn_id: String,
}

struct AuthorizedThreadContext {
    workspace: AuthorizedWorkspace,
    thread_id: String,
    turn_id: Option<String>,
}

async fn authorized_turn_context(
    state: &AppState,
    auth: &AuthenticatedUser,
    run_id: Uuid,
) -> Result<AuthorizedTurnContext, (StatusCode, Json<PlatformError>)> {
    let context = authorized_thread_context(state, auth, run_id).await?;
    Ok(AuthorizedTurnContext {
        workspace: context.workspace,
        thread_id: context.thread_id,
        turn_id: context.turn_id.ok_or_else(|| {
            (
                StatusCode::CONFLICT,
                Json(PlatformError::bad_request("Run has no active Turn")),
            )
        })?,
    })
}

async fn authorized_thread_context(
    state: &AppState,
    auth: &AuthenticatedUser,
    run_id: Uuid,
) -> Result<AuthorizedThreadContext, (StatusCode, Json<PlatformError>)> {
    let row = sqlx::query(
        "SELECT r.codex_thread_id, r.active_turn_id, r.workspace_id, r.requested_by, w.root_path \
         FROM runs r JOIN workspaces w ON w.id = r.workspace_id \
         WHERE r.id = $1 AND r.organization_id = $2 AND w.organization_id = $2 \
           AND r.status = 'running' AND w.state <> 'retired'",
    )
    .bind(run_id)
    .bind(auth.organization_id)
    .fetch_optional(&state.db)
    .await
    .map_err(database_error)?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(PlatformError::not_found("active Run was not found")),
        )
    })?;
    let requested_by: Option<Uuid> = row.get("requested_by");
    if requested_by != Some(auth.user_id)
        && !matches!(auth.organization_role.as_str(), "owner" | "admin")
    {
        return Err((
            StatusCode::NOT_FOUND,
            Json(PlatformError::not_found("active Run was not found")),
        ));
    }
    let workspace_id: Option<Uuid> = row.get("workspace_id");
    let root_path: String = row.get("root_path");
    let thread_id: Option<String> = row.get("codex_thread_id");
    let turn_id: Option<String> = row.get("active_turn_id");
    Ok(AuthorizedThreadContext {
        workspace: AuthorizedWorkspace {
            id: workspace_id
                .ok_or_else(|| {
                    (
                        StatusCode::CONFLICT,
                        Json(PlatformError::bad_request("Run workspace is not ready")),
                    )
                })?
                .to_string(),
            root: root_path.into(),
        },
        thread_id: thread_id.ok_or_else(|| {
            (
                StatusCode::CONFLICT,
                Json(PlatformError::bad_request("Run Thread is not ready")),
            )
        })?,
        turn_id,
    })
}

fn review_target(
    target: PlatformReviewTarget,
) -> Result<AdapterReviewTarget, (StatusCode, Json<PlatformError>)> {
    let invalid = || {
        (
            StatusCode::BAD_REQUEST,
            Json(PlatformError::bad_request("review target is invalid")),
        )
    };
    Ok(match target {
        PlatformReviewTarget::UncommittedChanges => AdapterReviewTarget::UncommittedChanges,
        PlatformReviewTarget::BaseBranch { branch } => {
            let branch = branch.trim();
            if branch.is_empty() || branch.len() > 255 || branch.starts_with('-') {
                return Err(invalid());
            }
            AdapterReviewTarget::BaseBranch {
                branch: branch.to_string(),
            }
        }
        PlatformReviewTarget::Commit { sha, title } => {
            let sha = sha.trim();
            if !(7..=64).contains(&sha.len()) || !sha.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(invalid());
            }
            if title.as_ref().is_some_and(|value| value.len() > 500) {
                return Err(invalid());
            }
            AdapterReviewTarget::Commit {
                sha: sha.to_string(),
                title,
            }
        }
        PlatformReviewTarget::Custom { instructions } => {
            let instructions = instructions.trim();
            if instructions.is_empty() || instructions.len() > 10_000 {
                return Err(invalid());
            }
            AdapterReviewTarget::Custom {
                instructions: instructions.to_string(),
            }
        }
    })
}

fn adapter_control_error(
    _error: open_web_codex_adapter::AdapterError,
) -> (StatusCode, Json<PlatformError>) {
    (
        StatusCode::BAD_GATEWAY,
        Json(PlatformError::internal("Codex Turn control failed")),
    )
}

fn run_from_record(run: RunRecord) -> Run {
    Run {
        id: run.id,
        task_id: run.task_id,
        status: run.status,
        codex_thread_id: run.codex_thread_id,
        active_turn_id: run.active_turn_id,
        workspace_id: run.workspace_id,
        source_ref: run.source_ref,
        workspace_kind: run.workspace_kind,
        workspace_name: run.workspace_name,
        workspace_parent_run_id: run.workspace_parent_run_id,
        workspace_group_run_id: run.workspace_group_run_id,
        attempt: run.attempt,
        created_at: run.created_at,
        updated_at: run.updated_at,
    }
}

fn run_from_row(row: &sqlx::postgres::PgRow) -> Run {
    Run {
        id: row.get("id"),
        task_id: row.get("task_id"),
        status: row.get("status"),
        codex_thread_id: row.get("codex_thread_id"),
        active_turn_id: row.get("active_turn_id"),
        workspace_id: row.get("workspace_id"),
        source_ref: row.get("source_ref"),
        workspace_kind: row.get("workspace_kind"),
        workspace_name: row.get("workspace_name"),
        workspace_parent_run_id: row.get("workspace_parent_run_id"),
        workspace_group_run_id: row.get("workspace_group_run_id"),
        attempt: row.get("attempt"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

pub(crate) fn orchestrator_error(error: RunOrchestratorError) -> (StatusCode, Json<PlatformError>) {
    let (status, platform) = match error {
        RunOrchestratorError::Invalid(message) => {
            (StatusCode::BAD_REQUEST, PlatformError::bad_request(message))
        }
        RunOrchestratorError::NotFound => (
            StatusCode::NOT_FOUND,
            PlatformError::not_found("Run resource was not found"),
        ),
        RunOrchestratorError::Conflict(message) => {
            (StatusCode::CONFLICT, PlatformError::bad_request(message))
        }
        RunOrchestratorError::Adapter(_) => (
            StatusCode::BAD_GATEWAY,
            PlatformError::internal("Codex Runtime operation failed"),
        ),
        RunOrchestratorError::LeaseLost => (
            StatusCode::CONFLICT,
            PlatformError::bad_request("Run ownership changed; reload its current state"),
        ),
        RunOrchestratorError::Database(_) | RunOrchestratorError::Git(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            PlatformError::internal("Run operation failed"),
        ),
    };
    (status, Json(platform))
}

fn database_error(_error: sqlx::Error) -> (StatusCode, Json<PlatformError>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(PlatformError::internal("database operation failed")),
    )
}
