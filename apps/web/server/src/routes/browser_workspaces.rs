use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use open_web_codex_platform_contracts::error::PlatformError;
use open_web_codex_platform_contracts::{
    BrowserWorkspacePreference, SetWorkspaceRuntimeCodexArgsRequest,
    SetWorkspaceRuntimeCodexArgsResponse, UpdateBrowserWorkspaceSettingsRequest,
    WorktreeSetupStatus,
};
use open_web_codex_platform_store::AppState;
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

use crate::middleware::auth::AuthenticatedUser;

type ApiError = (StatusCode, Json<PlatformError>);
type ApiResult<T> = Result<Json<T>, ApiError>;

const MAX_SETTINGS_BYTES: usize = 256 * 1024;
const MAX_CODEX_ARGS_BYTES: usize = 4096;
const MAX_SETUP_SCRIPT_BYTES: usize = 64 * 1024;

pub async fn list(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
) -> ApiResult<Vec<BrowserWorkspacePreference>> {
    let rows = sqlx::query(
        "SELECT browser_workspace_id, settings, runtime_codex_args \
         FROM browser_workspace_preferences \
         WHERE organization_id = $1 AND user_id = $2",
    )
    .bind(auth.organization_id)
    .bind(auth.user_id)
    .fetch_all(&state.db)
    .await
    .map_err(database_error)?;
    Ok(Json(
        rows.into_iter()
            .map(|row| BrowserWorkspacePreference {
                workspace_id: row.get("browser_workspace_id"),
                settings: row.get("settings"),
                runtime_codex_args: row.get("runtime_codex_args"),
            })
            .collect(),
    ))
}

pub async fn update_settings(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(workspace_id): Path<Uuid>,
    Json(request): Json<UpdateBrowserWorkspaceSettingsRequest>,
) -> ApiResult<BrowserWorkspacePreference> {
    authorize_browser_workspace(&state, &auth, workspace_id, true).await?;
    if !request.settings.is_object()
        || serde_json::to_vec(&request.settings)
            .map_err(|_| bad_request("Workspace settings are invalid"))?
            .len()
            > MAX_SETTINGS_BYTES
    {
        return Err(bad_request(
            "Workspace settings must be a bounded JSON object",
        ));
    }
    let row = sqlx::query(
        "INSERT INTO browser_workspace_preferences \
         (organization_id, user_id, browser_workspace_id, settings) \
         VALUES ($1, $2, $3, $4) \
         ON CONFLICT (organization_id, user_id, browser_workspace_id) DO UPDATE \
         SET settings = EXCLUDED.settings, updated_at = now() \
         RETURNING settings, runtime_codex_args",
    )
    .bind(auth.organization_id)
    .bind(auth.user_id)
    .bind(workspace_id)
    .bind(request.settings)
    .fetch_one(&state.db)
    .await
    .map_err(database_error)?;
    Ok(Json(BrowserWorkspacePreference {
        workspace_id,
        settings: row.get("settings"),
        runtime_codex_args: row.get("runtime_codex_args"),
    }))
}

pub async fn set_runtime_codex_args(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(workspace_id): Path<Uuid>,
    Json(request): Json<SetWorkspaceRuntimeCodexArgsRequest>,
) -> ApiResult<SetWorkspaceRuntimeCodexArgsResponse> {
    authorize_browser_workspace(&state, &auth, workspace_id, true).await?;
    let codex_args = request
        .codex_args
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if codex_args.as_ref().is_some_and(|value| {
        value.len() > MAX_CODEX_ARGS_BYTES || value.chars().any(|character| character.is_control())
    }) {
        return Err(bad_request("Codex arguments are invalid"));
    }
    sqlx::query(
        "INSERT INTO browser_workspace_preferences \
         (organization_id, user_id, browser_workspace_id, runtime_codex_args) \
         VALUES ($1, $2, $3, $4) \
         ON CONFLICT (organization_id, user_id, browser_workspace_id) DO UPDATE \
         SET runtime_codex_args = EXCLUDED.runtime_codex_args, updated_at = now()",
    )
    .bind(auth.organization_id)
    .bind(auth.user_id)
    .bind(workspace_id)
    .bind(&codex_args)
    .execute(&state.db)
    .await
    .map_err(database_error)?;

    // Arbitrary client process arguments are not applied to a shared Profile
    // Host. Typed per-Turn options continue to be applied by the message API.
    Ok(Json(SetWorkspaceRuntimeCodexArgsResponse {
        applied_codex_args: codex_args,
        respawned: false,
    }))
}

pub async fn worktree_setup_status(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(workspace_id): Path<Uuid>,
) -> ApiResult<WorktreeSetupStatus> {
    let kind = authorize_browser_workspace(&state, &auth, workspace_id, false).await?;
    if kind != "worktree" {
        return Ok(Json(WorktreeSetupStatus {
            should_run: false,
            script: None,
        }));
    }
    let row = sqlx::query(
        "SELECT settings, setup_completed_script FROM browser_workspace_preferences \
         WHERE organization_id = $1 AND user_id = $2 AND browser_workspace_id = $3",
    )
    .bind(auth.organization_id)
    .bind(auth.user_id)
    .bind(workspace_id)
    .fetch_optional(&state.db)
    .await
    .map_err(database_error)?;
    let Some(row) = row else {
        return Ok(Json(WorktreeSetupStatus {
            should_run: false,
            script: None,
        }));
    };
    let settings: Value = row.get("settings");
    let script = settings
        .get("worktreeSetupScript")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if script
        .as_ref()
        .is_some_and(|value| value.len() > MAX_SETUP_SCRIPT_BYTES)
    {
        return Err(bad_request(
            "Worktree setup script exceeds the supported size",
        ));
    }
    let completed: Option<String> = row.get("setup_completed_script");
    Ok(Json(WorktreeSetupStatus {
        should_run: script.is_some() && script != completed,
        script,
    }))
}

pub async fn mark_worktree_setup_ran(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(workspace_id): Path<Uuid>,
) -> ApiResult<Value> {
    if authorize_browser_workspace(&state, &auth, workspace_id, true).await? != "worktree" {
        return Err(bad_request("Workspace is not a worktree"));
    }
    sqlx::query(
        "UPDATE browser_workspace_preferences \
         SET setup_completed_script = NULLIF(btrim(settings->>'worktreeSetupScript'), ''), \
             updated_at = now() \
         WHERE organization_id = $1 AND user_id = $2 AND browser_workspace_id = $3",
    )
    .bind(auth.organization_id)
    .bind(auth.user_id)
    .bind(workspace_id)
    .execute(&state.db)
    .await
    .map_err(database_error)?;
    Ok(Json(serde_json::json!({ "status": "recorded" })))
}

async fn authorize_browser_workspace(
    state: &AppState,
    auth: &AuthenticatedUser,
    workspace_id: Uuid,
    require_owner: bool,
) -> Result<String, ApiError> {
    if let Some(created_by) = sqlx::query_scalar::<_, Option<Uuid>>(
        "SELECT created_by FROM projects WHERE id = $1 AND organization_id = $2",
    )
    .bind(workspace_id)
    .bind(auth.organization_id)
    .fetch_optional(&state.db)
    .await
    .map_err(database_error)?
    {
        if require_owner
            && created_by != Some(auth.user_id)
            && !matches!(auth.organization_role.as_str(), "owner" | "admin")
        {
            return Err(not_found());
        }
        return Ok("main".to_string());
    }
    let row = sqlx::query(
        "SELECT workspace_kind, requested_by FROM runs \
         WHERE id = $1 AND organization_id = $2 AND workspace_kind <> 'main'",
    )
    .bind(workspace_id)
    .bind(auth.organization_id)
    .fetch_optional(&state.db)
    .await
    .map_err(database_error)?
    .ok_or_else(not_found)?;
    let requested_by: Option<Uuid> = row.get("requested_by");
    if require_owner
        && requested_by != Some(auth.user_id)
        && !matches!(auth.organization_role.as_str(), "owner" | "admin")
    {
        return Err(not_found());
    }
    Ok(row.get("workspace_kind"))
}

fn bad_request(message: &str) -> ApiError {
    (
        StatusCode::BAD_REQUEST,
        Json(PlatformError::bad_request(message)),
    )
}

fn not_found() -> ApiError {
    (
        StatusCode::NOT_FOUND,
        Json(PlatformError::not_found("Browser workspace was not found")),
    )
}

fn database_error(_: sqlx::Error) -> ApiError {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(PlatformError::internal("Database operation failed")),
    )
}
