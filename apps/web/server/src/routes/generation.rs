use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::{Extension, Json};
use open_web_codex_adapter::{AuthorizedWorkspace, CodexAdapter};
use open_web_codex_git_runtime::GitRuntime;
use open_web_codex_platform_contracts::error::PlatformError;
use open_web_codex_platform_contracts::{GenerateTextRequest, GenerateTextResponse};
use open_web_codex_platform_store::AppState;
use sqlx::Row;
use uuid::Uuid;

use crate::middleware::auth::AuthenticatedUser;

type ApiError = (StatusCode, Json<PlatformError>);
type ApiResult<T> = Result<Json<T>, ApiError>;

const MAX_GENERATION_INPUT_BYTES: usize = 64 * 1024;
const MAX_GENERATION_DIFF_BYTES: usize = 256 * 1024;

pub async fn generate(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(run_id): Path<Uuid>,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
    Extension(git): Extension<Arc<GitRuntime>>,
    Json(request): Json<GenerateTextRequest>,
) -> ApiResult<GenerateTextResponse> {
    if request.input.len() > MAX_GENERATION_INPUT_BYTES {
        return Err(bad_request("Generation input is too large"));
    }
    let workspace = authorized_workspace(&state, &auth, run_id).await?;
    let prompt = match request.kind.as_str() {
        "runMetadata" => run_metadata_prompt(require_input(&request.input)?),
        "agentDescription" => agent_description_prompt(require_input(&request.input)?),
        "commitMessage" => {
            let workspace_id = Uuid::parse_str(&workspace.id)
                .map_err(|_| internal("Run workspace identity is invalid"))?;
            let diffs = git
                .diffs(workspace_id)
                .await
                .map_err(|_| internal("Unable to read workspace changes for message generation"))?;
            let mut text = String::new();
            for diff in diffs {
                if diff.is_binary {
                    text.push_str(&format!("Binary file changed: {}\n", diff.path));
                } else {
                    text.push_str(&diff.diff);
                    text.push('\n');
                }
                if text.len() > MAX_GENERATION_DIFF_BYTES {
                    text.truncate(MAX_GENERATION_DIFF_BYTES);
                    text.push_str("\n[diff truncated]\n");
                    break;
                }
            }
            if text.trim().is_empty() {
                return Err(bad_request("No changes to generate a commit message for"));
            }
            commit_message_prompt(&text)
        }
        _ => return Err(bad_request("Unknown generation kind")),
    };
    let text = adapter
        .generate_text(&workspace, &prompt, request.model.as_deref())
        .await
        .map_err(|_| {
            (
                StatusCode::BAD_GATEWAY,
                Json(PlatformError::internal("Codex text generation failed")),
            )
        })?;
    Ok(Json(GenerateTextResponse { text }))
}

async fn authorized_workspace(
    state: &AppState,
    auth: &AuthenticatedUser,
    run_id: Uuid,
) -> Result<AuthorizedWorkspace, ApiError> {
    let row = sqlx::query(
        "SELECT r.workspace_id, r.requested_by, w.root_path, w.state \
         FROM runs r JOIN workspaces w ON w.id = r.workspace_id \
         WHERE r.id = $1 AND r.organization_id = $2 AND w.organization_id = $2",
    )
    .bind(run_id)
    .bind(auth.organization_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|_| internal("database operation failed"))?
    .ok_or_else(|| not_found("Run workspace was not found"))?;
    let requested_by: Option<Uuid> = row.get("requested_by");
    if requested_by != Some(auth.user_id)
        && !matches!(auth.organization_role.as_str(), "owner" | "admin")
    {
        return Err(not_found("Run workspace was not found"));
    }
    if row.get::<String, _>("state") == "retired" {
        return Err(bad_request("Run workspace has been retired"));
    }
    Ok(AuthorizedWorkspace {
        id: row.get::<Uuid, _>("workspace_id").to_string(),
        root: row.get::<String, _>("root_path").into(),
    })
}

fn require_input(value: &str) -> Result<&str, ApiError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(bad_request("Generation input is required"));
    }
    Ok(value)
}

fn commit_message_prompt(diff: &str) -> String {
    format!(
        "Generate a concise git commit message for the following changes. \
Follow conventional commit format (for example feat:, fix:, refactor:, docs:). \
Keep the summary line under 72 characters. Only output the commit message.\n\nChanges:\n{diff}"
    )
}

fn run_metadata_prompt(input: &str) -> String {
    format!(
        "You create concise run metadata for a coding task.\n\
Return ONLY a JSON object with keys title and worktreeName.\n\
title must be a clear 3-7 word Title Case phrase.\n\
worktreeName must be a lower-case kebab-case slug prefixed with one of \
feat/, fix/, chore/, test/, docs/, refactor/, perf/, build/, ci/, style/.\n\
Example: {{\"title\":\"Fix Login Redirect Loop\",\"worktreeName\":\"fix/login-redirect-loop\"}}\n\nTask:\n{input}"
    )
}

fn agent_description_prompt(input: &str) -> String {
    format!(
        "You generate custom coding-agent configuration text.\n\
Return ONLY a JSON object with exactly the keys description and developerInstructions.\n\
description must be a practical one-sentence role summary of 4-12 words.\n\
developerInstructions must contain 3-8 actionable, specific lines.\n\
Do not include Markdown fences.\n\nUser prompt:\n{input}"
    )
}

fn bad_request(message: &str) -> ApiError {
    (
        StatusCode::BAD_REQUEST,
        Json(PlatformError::bad_request(message)),
    )
}

fn not_found(message: &str) -> ApiError {
    (
        StatusCode::NOT_FOUND,
        Json(PlatformError::not_found(message)),
    )
}

fn internal(message: &str) -> ApiError {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(PlatformError::internal(message)),
    )
}
