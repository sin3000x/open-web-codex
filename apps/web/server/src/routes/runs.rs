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
    InterruptRunRequest, ReviewTarget as PlatformReviewTarget, Run, RunFailureCode,
    StartReviewRequest, StartRunRequest, StartRunResponse, SteerRunRequest,
};
use open_web_codex_platform_store::AppState;
use open_web_codex_run_orchestrator::{
    CancelRunRequest, EnqueueRunRequest, ReplayRunRequest, RunOrchestrator, RunOrchestratorError,
    RunRecord,
};
use sqlx::Row;
use uuid::Uuid;

use crate::middleware::auth::{require_runtime_profile, AuthenticatedUser};
use crate::routes::RuntimeProfileBinding;

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<PlatformError>)>;

struct TaskWorkspace {
    id: Uuid,
}

/// Queue a Run against the immutable authorized Workspace selected by its Task.
/// The worker owns only Run scheduling and Runtime delivery; Workspace
/// provisioning is independent.
pub async fn start_run(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(task_id): Path<Uuid>,
    Extension(orchestrator): Extension<Arc<RunOrchestrator>>,
    Extension(profile): Extension<RuntimeProfileBinding>,
    Json(req): Json<StartRunRequest>,
) -> ApiResult<StartRunResponse> {
    require_runtime_profile(&state.db, &auth, &profile.runtime_key).await?;
    let task = authorized_task_workspace(&state, &auth, &profile, task_id).await?;
    let workspace_id = task.id;
    if let Some(run) = orchestrator
        .replay_run(ReplayRunRequest {
            organization_id: auth.organization_id,
            actor_id: auth.user_id,
            task_id,
            idempotency_key: req.idempotency_key.clone(),
            workspace_id,
            fork_thread_id: req.fork_thread_id.clone(),
            fork_source_run_id: req.fork_source_run_id,
        })
        .await
        .map_err(orchestrator_error)?
    {
        return Ok(Json(StartRunResponse {
            run: run_from_record(run),
        }));
    }
    let run = orchestrator
        .enqueue_run(EnqueueRunRequest {
            organization_id: auth.organization_id,
            actor_id: auth.user_id,
            task_id,
            idempotency_key: req.idempotency_key,
            workspace_id,
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
            "SELECT id, task_id, status, failure_code, codex_thread_id, active_turn_id, workspace_id, \
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
            "SELECT id, task_id, status, failure_code, codex_thread_id, active_turn_id, workspace_id, \
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
        .interrupt_turn(
            &context.workspace,
            context.copilot_package_id.as_deref(),
            &context.thread_id,
            &request.turn_id,
        )
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
            context.copilot_package_id.as_deref(),
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
        .compact_thread(
            &context.workspace,
            context.copilot_package_id.as_deref(),
            &context.thread_id,
        )
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
        .start_review(
            &context.workspace,
            context.copilot_package_id.as_deref(),
            &context.thread_id,
            target,
        )
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
    copilot_package_id: Option<String>,
    thread_id: String,
    turn_id: String,
}

struct AuthorizedThreadContext {
    workspace: AuthorizedWorkspace,
    copilot_package_id: Option<String>,
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
        copilot_package_id: context.copilot_package_id,
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
        "SELECT run.codex_thread_id, run.active_turn_id, run.workspace_id, \
                run.requested_by, task.copilot_package_id, workspace.root_path \
         FROM runs run \
         JOIN tasks task ON task.id = run.task_id \
           AND task.organization_id = run.organization_id \
           AND task.workspace_id = run.workspace_id \
         JOIN workspaces workspace ON workspace.id = run.workspace_id \
           AND workspace.organization_id = run.organization_id \
         JOIN workspace_grants workspace_grant ON workspace_grant.workspace_id = workspace.id \
           AND workspace_grant.organization_id = workspace.organization_id \
           AND workspace_grant.user_id = run.requested_by \
           AND workspace_grant.profile_id = workspace.profile_id \
           AND workspace_grant.role IN ('owner', 'write') \
         WHERE run.id = $1 AND run.organization_id = $2 \
           AND run.status = 'running' AND workspace.state IN ('ready', 'retained')",
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
    let workspace_id: Uuid = row.get("workspace_id");
    let root_path: String = row.get("root_path");
    let thread_id: Option<String> = row.get("codex_thread_id");
    let turn_id: Option<String> = row.get("active_turn_id");
    Ok(AuthorizedThreadContext {
        copilot_package_id: row.get("copilot_package_id"),
        workspace: AuthorizedWorkspace {
            id: workspace_id.to_string(),
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
        failure_code: run
            .failure_code
            .as_deref()
            .map(RunFailureCode::from_persisted),
        codex_thread_id: run.codex_thread_id,
        active_turn_id: run.active_turn_id,
        workspace_id: run.workspace_id,
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
        failure_code: row
            .get::<Option<String>, _>("failure_code")
            .as_deref()
            .map(RunFailureCode::from_persisted),
        codex_thread_id: row.get("codex_thread_id"),
        active_turn_id: row.get("active_turn_id"),
        workspace_id: row.get("workspace_id"),
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
/// Resolve the immutable Task Workspace under the same active Profile and
/// write-grant conditions used by the Run scheduler. No browser-selected path
/// or Run-level Workspace identifier participates in this lookup.
async fn authorized_task_workspace(
    state: &AppState,
    auth: &AuthenticatedUser,
    profile: &RuntimeProfileBinding,
    task_id: Uuid,
) -> Result<TaskWorkspace, (StatusCode, Json<PlatformError>)> {
    let row = sqlx::query(
        "SELECT task.workspace_id \
         FROM tasks task \
         JOIN profiles runtime_profile ON runtime_profile.organization_id = task.organization_id \
           AND runtime_profile.owner_user_id = $2 AND runtime_profile.runtime_key = $3 \
           AND runtime_profile.status = 'active' \
         JOIN workspaces workspace ON workspace.id = task.workspace_id \
           AND workspace.organization_id = task.organization_id \
           AND workspace.project_id = task.project_id \
           AND workspace.profile_id = runtime_profile.id \
           AND workspace.state IN ('ready', 'retained') \
         JOIN workspace_grants workspace_grant \
           ON workspace_grant.workspace_id = workspace.id \
          AND workspace_grant.organization_id = workspace.organization_id \
          AND workspace_grant.user_id = $2 \
          AND workspace_grant.profile_id = runtime_profile.id \
          AND workspace_grant.role IN ('owner', 'write') \
         WHERE task.id = $1 AND task.organization_id = $4",
    )
    .bind(task_id)
    .bind(auth.user_id)
    .bind(&profile.runtime_key)
    .bind(auth.organization_id)
    .fetch_optional(&state.db)
    .await
    .map_err(database_error)?
    .ok_or_else(|| {
        (
            StatusCode::CONFLICT,
            Json(PlatformError::workspace_unavailable(
                "task_workspace_unavailable",
            )),
        )
    })?;
    Ok(TaskWorkspace {
        id: row.get("workspace_id"),
    })
}

#[cfg(test)]
mod tests {
    use super::run_from_record;
    use chrono::Utc;
    use open_web_codex_platform_contracts::RunFailureCode;
    use open_web_codex_run_orchestrator::RunRecord;
    use uuid::Uuid;

    #[test]
    fn run_projection_includes_the_safe_failure_code() {
        let projected = run_from_record(RunRecord {
            id: Uuid::now_v7(),
            task_id: Uuid::now_v7(),
            status: "failed".to_string(),
            failure_code: Some("codex_unavailable".to_string()),
            codex_thread_id: None,
            active_turn_id: None,
            workspace_id: Uuid::now_v7(),
            attempt: 1,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        });

        assert_eq!(
            projected.failure_code,
            Some(RunFailureCode::CodexUnavailable)
        );
        assert_eq!(
            serde_json::to_value(projected).unwrap()["failure_code"],
            "codex_unavailable"
        );
    }

    #[test]
    fn run_projection_bounds_unknown_persisted_failure_codes() {
        let projected = run_from_record(RunRecord {
            id: Uuid::now_v7(),
            task_id: Uuid::now_v7(),
            status: "failed".to_string(),
            failure_code: Some("raw internal failure detail".to_string()),
            codex_thread_id: None,
            active_turn_id: None,
            workspace_id: Uuid::now_v7(),
            attempt: 1,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        });

        assert_eq!(projected.failure_code, Some(RunFailureCode::UnknownFailure));
        assert_eq!(
            serde_json::to_value(projected).unwrap()["failure_code"],
            "unknown_failure"
        );
    }
}
