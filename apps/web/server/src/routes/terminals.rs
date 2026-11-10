use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use open_web_codex_adapter::{AuthorizedWorkspace, CodexAdapter};
use open_web_codex_platform_contracts::error::PlatformError;
use open_web_codex_platform_contracts::{
    OpenTerminalRequest, ResizeTerminalRequest, WriteTerminalRequest,
};
use open_web_codex_platform_store::AppState;
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

use crate::middleware::auth::AuthenticatedUser;

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<PlatformError>)>;

struct TerminalContext {
    workspace: AuthorizedWorkspace,
    run_id: Uuid,
}

pub async fn open(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(workspace_id): Path<Uuid>,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
    Json(request): Json<OpenTerminalRequest>,
) -> ApiResult<serde_json::Value> {
    let terminal_id = validate_terminal_id(&request.terminal_id)?;
    validate_size(request.cols, request.rows)?;
    let mut transaction = state.db.begin().await.map_err(database_error)?;
    let context = authorized_workspace_context(&mut transaction, &auth, workspace_id).await?;
    let process_id = Uuid::now_v7().to_string();
    sqlx::query(
        "INSERT INTO terminal_sessions \
         (organization_id, run_id, workspace_id, terminal_id, process_id, state) \
         VALUES ($1, $2, $3, $4, $5, 'starting')",
    )
    .bind(auth.organization_id)
    .bind(context.run_id)
    .bind(workspace_id)
    .bind(&terminal_id)
    .bind(&process_id)
    .execute(&mut *transaction)
    .await
    .map_err(|error| {
        if error
            .as_database_error()
            .is_some_and(|error| error.is_unique_violation())
        {
            (
                StatusCode::CONFLICT,
                Json(PlatformError::bad_request(
                    "terminal session is already open",
                )),
            )
        } else {
            database_error(error)
        }
    })?;
    transaction.commit().await.map_err(database_error)?;
    if let Err(error) = adapter
        .open_terminal(&context.workspace, &process_id, request.cols, request.rows)
        .await
    {
        sqlx::query(
            "UPDATE terminal_sessions SET state = 'failed', updated_at = now() \
             WHERE organization_id = $1 AND workspace_id = $2 AND terminal_id = $3",
        )
        .bind(auth.organization_id)
        .bind(workspace_id)
        .bind(&terminal_id)
        .execute(&state.db)
        .await
        .map_err(database_error)?;
        return Err(adapter_error("terminal.open", workspace_id, &error));
    }
    sqlx::query(
        "UPDATE terminal_sessions SET state = 'running', updated_at = now() \
         WHERE organization_id = $1 AND workspace_id = $2 AND terminal_id = $3 AND state = 'starting'",
    )
    .bind(auth.organization_id)
    .bind(workspace_id)
    .bind(&terminal_id)
    .execute(&state.db)
    .await
    .map_err(database_error)?;
    audit(&state, &auth, workspace_id, "terminal.open", &terminal_id).await?;
    Ok(Json(json!({ "id": terminal_id })))
}

pub async fn write(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path((workspace_id, terminal_id)): Path<(Uuid, String)>,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
    Json(request): Json<WriteTerminalRequest>,
) -> ApiResult<serde_json::Value> {
    if request.data.is_empty() || request.data.len() > 64 * 1024 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(PlatformError::bad_request(
                "terminal input must contain at most 64 KiB",
            )),
        ));
    }
    let (workspace, process_id) =
        authorized_terminal(&state, &auth, workspace_id, &terminal_id).await?;
    adapter
        .write_terminal(&workspace, &process_id, &request.data)
        .await
        .map_err(|error| adapter_error("terminal.write", workspace_id, &error))?;
    touch(&state, auth.organization_id, workspace_id, &terminal_id).await?;
    Ok(Json(json!({ "status": "written" })))
}

pub async fn resize(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path((workspace_id, terminal_id)): Path<(Uuid, String)>,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
    Json(request): Json<ResizeTerminalRequest>,
) -> ApiResult<serde_json::Value> {
    validate_size(request.cols, request.rows)?;
    let (workspace, process_id) =
        authorized_terminal(&state, &auth, workspace_id, &terminal_id).await?;
    adapter
        .resize_terminal(&workspace, &process_id, request.cols, request.rows)
        .await
        .map_err(|error| adapter_error("terminal.resize", workspace_id, &error))?;
    touch(&state, auth.organization_id, workspace_id, &terminal_id).await?;
    Ok(Json(json!({ "status": "resized" })))
}

pub async fn close(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path((workspace_id, terminal_id)): Path<(Uuid, String)>,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
) -> ApiResult<serde_json::Value> {
    let (workspace, process_id) =
        authorized_terminal(&state, &auth, workspace_id, &terminal_id).await?;
    adapter
        .close_terminal(&workspace, &process_id)
        .await
        .map_err(|error| adapter_error("terminal.close", workspace_id, &error))?;
    sqlx::query(
        "UPDATE terminal_sessions SET state = 'closing', updated_at = now() \
         WHERE organization_id = $1 AND workspace_id = $2 AND terminal_id = $3 \
           AND state IN ('starting', 'running')",
    )
    .bind(auth.organization_id)
    .bind(workspace_id)
    .bind(&terminal_id)
    .execute(&state.db)
    .await
    .map_err(database_error)?;
    audit(&state, &auth, workspace_id, "terminal.close", &terminal_id).await?;
    Ok(Json(json!({ "status": "closing" })))
}

async fn authorized_workspace_context(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    auth: &AuthenticatedUser,
    workspace_id: Uuid,
) -> Result<TerminalContext, (StatusCode, Json<PlatformError>)> {
    let row = sqlx::query(
        "SELECT workspace.id AS workspace_id, workspace.root_path, workspace.state, workspace_grant.role, \
                (SELECT run.id FROM runs run \
                 WHERE run.workspace_id = workspace.id AND run.requested_by = $3 \
                   AND run.codex_thread_id IS NOT NULL \
                 ORDER BY run.updated_at DESC, run.id DESC LIMIT 1) AS run_id \
         FROM workspaces workspace \
         LEFT JOIN workspace_grants workspace_grant ON workspace_grant.workspace_id = workspace.id \
           AND workspace_grant.organization_id = workspace.organization_id \
           AND workspace_grant.user_id = $3 AND workspace_grant.profile_id = workspace.profile_id \
         WHERE workspace.id = $1 AND workspace.organization_id = $2 \
         FOR KEY SHARE OF workspace",
    )
    .bind(workspace_id)
    .bind(auth.organization_id)
    .bind(auth.user_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(database_error)?
    .ok_or_else(not_found)?;
    if !row
        .get::<Option<String>, _>("role")
        .as_deref()
        .is_some_and(|role| matches!(role, "owner" | "write"))
        && !matches!(auth.organization_role.as_str(), "owner" | "admin")
    {
        return Err(not_found());
    }
    if !matches!(row.get::<String, _>("state").as_str(), "ready" | "retained") {
        return Err((
            StatusCode::CONFLICT,
            Json(PlatformError::bad_request("Workspace is not ready")),
        ));
    }
    let workspace_id: Uuid = row.get("workspace_id");
    Ok(TerminalContext {
        workspace: AuthorizedWorkspace {
            id: workspace_id.to_string(),
            root: row.get::<String, _>("root_path").into(),
        },
        run_id: row.get::<Option<Uuid>, _>("run_id").ok_or_else(|| {
            (
                StatusCode::CONFLICT,
                Json(PlatformError::bad_request(
                    "start a Thread in this Workspace before opening a terminal",
                )),
            )
        })?,
    })
}

async fn authorized_terminal(
    state: &AppState,
    auth: &AuthenticatedUser,
    workspace_id: Uuid,
    terminal_id: &str,
) -> Result<(AuthorizedWorkspace, String), (StatusCode, Json<PlatformError>)> {
    let row = sqlx::query(
        "SELECT session.process_id, workspace.id AS workspace_id, workspace.root_path, \
                workspace.state, workspace_grant.role \
         FROM terminal_sessions session \
         JOIN workspaces workspace ON workspace.id = session.workspace_id \
           AND workspace.organization_id = session.organization_id \
         LEFT JOIN workspace_grants workspace_grant ON workspace_grant.workspace_id = workspace.id \
           AND workspace_grant.organization_id = workspace.organization_id \
           AND workspace_grant.user_id = $4 AND workspace_grant.profile_id = workspace.profile_id \
         WHERE session.organization_id = $1 AND session.workspace_id = $2 \
           AND session.terminal_id = $3 AND session.state IN ('starting', 'running')",
    )
    .bind(auth.organization_id)
    .bind(workspace_id)
    .bind(terminal_id)
    .bind(auth.user_id)
    .fetch_optional(&state.db)
    .await
    .map_err(database_error)?
    .ok_or_else(not_found)?;
    if !row
        .get::<Option<String>, _>("role")
        .as_deref()
        .is_some_and(|role| matches!(role, "owner" | "write"))
        && !matches!(auth.organization_role.as_str(), "owner" | "admin")
    {
        return Err(not_found());
    }
    if !matches!(row.get::<String, _>("state").as_str(), "ready" | "retained") {
        return Err(not_found());
    }
    Ok((
        AuthorizedWorkspace {
            id: row.get::<Uuid, _>("workspace_id").to_string(),
            root: row.get::<String, _>("root_path").into(),
        },
        row.get("process_id"),
    ))
}

fn validate_terminal_id(value: &str) -> Result<String, (StatusCode, Json<PlatformError>)> {
    let value = value.trim();
    if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(PlatformError::bad_request("terminal id is invalid")),
        ));
    }
    Ok(value.to_string())
}

fn validate_size(cols: u16, rows: u16) -> Result<(), (StatusCode, Json<PlatformError>)> {
    if !(1..=500).contains(&cols) || !(1..=500).contains(&rows) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(PlatformError::bad_request(
                "terminal rows and columns must be between 1 and 500",
            )),
        ));
    }
    Ok(())
}

async fn touch(
    state: &AppState,
    organization_id: Uuid,
    workspace_id: Uuid,
    terminal_id: &str,
) -> Result<(), (StatusCode, Json<PlatformError>)> {
    sqlx::query(
        "UPDATE terminal_sessions SET updated_at = now() \
         WHERE organization_id = $1 AND workspace_id = $2 AND terminal_id = $3",
    )
    .bind(organization_id)
    .bind(workspace_id)
    .bind(terminal_id)
    .execute(&state.db)
    .await
    .map_err(database_error)?;
    Ok(())
}

async fn audit(
    state: &AppState,
    auth: &AuthenticatedUser,
    workspace_id: Uuid,
    action: &str,
    terminal_id: &str,
) -> Result<(), (StatusCode, Json<PlatformError>)> {
    sqlx::query(
        "INSERT INTO audit_log \
         (organization_id, actor_id, action, target_type, target_id, metadata, outcome) \
         VALUES ($1, $2, $3, 'workspace', $4, $5, 'success')",
    )
    .bind(auth.organization_id)
    .bind(auth.user_id)
    .bind(action)
    .bind(workspace_id)
    .bind(json!({ "terminalId": terminal_id }))
    .execute(&state.db)
    .await
    .map_err(database_error)?;
    Ok(())
}

fn not_found() -> (StatusCode, Json<PlatformError>) {
    (
        StatusCode::NOT_FOUND,
        Json(PlatformError::not_found("terminal session was not found")),
    )
}

fn adapter_error(
    operation: &'static str,
    workspace_id: Uuid,
    error: &open_web_codex_adapter::AdapterError,
) -> (StatusCode, Json<PlatformError>) {
    super::runtime_errors::runtime_api_error(
        operation,
        "terminal",
        super::runtime_errors::RuntimeErrorScope::Workspace(workspace_id),
        error,
    )
}

fn database_error(_error: sqlx::Error) -> (StatusCode, Json<PlatformError>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(PlatformError::internal("database operation failed")),
    )
}
