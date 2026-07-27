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
    browser_workspace_id: Uuid,
}

pub async fn open(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(run_id): Path<Uuid>,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
    Json(request): Json<OpenTerminalRequest>,
) -> ApiResult<serde_json::Value> {
    let terminal_id = validate_terminal_id(&request.terminal_id)?;
    validate_size(request.cols, request.rows)?;
    let context = authorized_run_workspace(&state, &auth, run_id).await?;
    let process_id = Uuid::now_v7().to_string();
    sqlx::query(
        "INSERT INTO terminal_sessions \
         (organization_id, run_id, browser_workspace_id, terminal_id, process_id, state) \
         VALUES ($1, $2, $3, $4, $5, 'starting')",
    )
    .bind(auth.organization_id)
    .bind(run_id)
    .bind(context.browser_workspace_id)
    .bind(&terminal_id)
    .bind(&process_id)
    .execute(&state.db)
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
    if let Err(error) = adapter
        .open_terminal(&context.workspace, &process_id, request.cols, request.rows)
        .await
    {
        sqlx::query(
            "UPDATE terminal_sessions SET state = 'failed', updated_at = now() \
             WHERE organization_id = $1 AND run_id = $2 AND terminal_id = $3",
        )
        .bind(auth.organization_id)
        .bind(run_id)
        .bind(&terminal_id)
        .execute(&state.db)
        .await
        .map_err(database_error)?;
        return Err(adapter_error(error));
    }
    sqlx::query(
        "UPDATE terminal_sessions SET state = 'running', updated_at = now() \
         WHERE organization_id = $1 AND run_id = $2 AND terminal_id = $3 AND state = 'starting'",
    )
    .bind(auth.organization_id)
    .bind(run_id)
    .bind(&terminal_id)
    .execute(&state.db)
    .await
    .map_err(database_error)?;
    audit(&state, &auth, run_id, "terminal.open", &terminal_id).await?;
    Ok(Json(json!({ "id": terminal_id })))
}

pub async fn write(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path((run_id, terminal_id)): Path<(Uuid, String)>,
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
    let (workspace, process_id) = authorized_terminal(&state, &auth, run_id, &terminal_id).await?;
    adapter
        .write_terminal(&workspace, &process_id, &request.data)
        .await
        .map_err(adapter_error)?;
    touch(&state, auth.organization_id, run_id, &terminal_id).await?;
    Ok(Json(json!({ "status": "written" })))
}

pub async fn resize(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path((run_id, terminal_id)): Path<(Uuid, String)>,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
    Json(request): Json<ResizeTerminalRequest>,
) -> ApiResult<serde_json::Value> {
    validate_size(request.cols, request.rows)?;
    let (workspace, process_id) = authorized_terminal(&state, &auth, run_id, &terminal_id).await?;
    adapter
        .resize_terminal(&workspace, &process_id, request.cols, request.rows)
        .await
        .map_err(adapter_error)?;
    touch(&state, auth.organization_id, run_id, &terminal_id).await?;
    Ok(Json(json!({ "status": "resized" })))
}

pub async fn close(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path((run_id, terminal_id)): Path<(Uuid, String)>,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
) -> ApiResult<serde_json::Value> {
    let (workspace, process_id) = authorized_terminal(&state, &auth, run_id, &terminal_id).await?;
    adapter
        .close_terminal(&workspace, &process_id)
        .await
        .map_err(adapter_error)?;
    sqlx::query(
        "UPDATE terminal_sessions SET state = 'closing', updated_at = now() \
         WHERE organization_id = $1 AND run_id = $2 AND terminal_id = $3 \
           AND state IN ('starting', 'running')",
    )
    .bind(auth.organization_id)
    .bind(run_id)
    .bind(&terminal_id)
    .execute(&state.db)
    .await
    .map_err(database_error)?;
    audit(&state, &auth, run_id, "terminal.close", &terminal_id).await?;
    Ok(Json(json!({ "status": "closing" })))
}

async fn authorized_run_workspace(
    state: &AppState,
    auth: &AuthenticatedUser,
    run_id: Uuid,
) -> Result<TerminalContext, (StatusCode, Json<PlatformError>)> {
    let row = sqlx::query(
        "SELECT r.workspace_id, r.requested_by, r.workspace_kind, w.root_path, w.state, \
                task.project_id \
         FROM runs r JOIN workspaces w ON w.id = r.workspace_id \
         JOIN tasks task ON task.id = r.task_id AND task.organization_id = r.organization_id \
         WHERE r.id = $1 AND r.organization_id = $2 AND w.organization_id = $2",
    )
    .bind(run_id)
    .bind(auth.organization_id)
    .fetch_optional(&state.db)
    .await
    .map_err(database_error)?
    .ok_or_else(not_found)?;
    let requested_by: Option<Uuid> = row.get("requested_by");
    if requested_by != Some(auth.user_id)
        && !matches!(auth.organization_role.as_str(), "owner" | "admin")
    {
        return Err(not_found());
    }
    if row.get::<String, _>("state") == "retired" {
        return Err((
            StatusCode::CONFLICT,
            Json(PlatformError::bad_request("Run workspace has been retired")),
        ));
    }
    let workspace_id: Uuid = row.get("workspace_id");
    let browser_workspace_id = if row.get::<String, _>("workspace_kind") == "main" {
        row.get("project_id")
    } else {
        run_id
    };
    Ok(TerminalContext {
        workspace: AuthorizedWorkspace {
            id: workspace_id.to_string(),
            root: row.get::<String, _>("root_path").into(),
        },
        browser_workspace_id,
    })
}

async fn authorized_terminal(
    state: &AppState,
    auth: &AuthenticatedUser,
    run_id: Uuid,
    terminal_id: &str,
) -> Result<(AuthorizedWorkspace, String), (StatusCode, Json<PlatformError>)> {
    let row = sqlx::query(
        "SELECT session.process_id, r.workspace_id, r.requested_by, w.root_path, w.state \
         FROM terminal_sessions session \
         JOIN runs r ON r.id = session.run_id AND r.organization_id = session.organization_id \
         JOIN workspaces w ON w.id = r.workspace_id AND w.organization_id = r.organization_id \
         WHERE session.organization_id = $1 AND session.run_id = $2 \
           AND session.terminal_id = $3 AND session.state IN ('starting', 'running')",
    )
    .bind(auth.organization_id)
    .bind(run_id)
    .bind(terminal_id)
    .fetch_optional(&state.db)
    .await
    .map_err(database_error)?
    .ok_or_else(not_found)?;
    if row.get::<Option<Uuid>, _>("requested_by") != Some(auth.user_id)
        && !matches!(auth.organization_role.as_str(), "owner" | "admin")
    {
        return Err(not_found());
    }
    if row.get::<String, _>("state") == "retired" {
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
    run_id: Uuid,
    terminal_id: &str,
) -> Result<(), (StatusCode, Json<PlatformError>)> {
    sqlx::query(
        "UPDATE terminal_sessions SET updated_at = now() \
         WHERE organization_id = $1 AND run_id = $2 AND terminal_id = $3",
    )
    .bind(organization_id)
    .bind(run_id)
    .bind(terminal_id)
    .execute(&state.db)
    .await
    .map_err(database_error)?;
    Ok(())
}

async fn audit(
    state: &AppState,
    auth: &AuthenticatedUser,
    run_id: Uuid,
    action: &str,
    terminal_id: &str,
) -> Result<(), (StatusCode, Json<PlatformError>)> {
    sqlx::query(
        "INSERT INTO audit_log \
         (organization_id, actor_id, action, target_type, target_id, metadata, outcome) \
         VALUES ($1, $2, $3, 'run', $4, $5, 'success')",
    )
    .bind(auth.organization_id)
    .bind(auth.user_id)
    .bind(action)
    .bind(run_id)
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
    _error: open_web_codex_adapter::AdapterError,
) -> (StatusCode, Json<PlatformError>) {
    (
        StatusCode::BAD_GATEWAY,
        Json(PlatformError::internal("Codex terminal operation failed")),
    )
}

fn database_error(_error: sqlx::Error) -> (StatusCode, Json<PlatformError>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(PlatformError::internal("database operation failed")),
    )
}
