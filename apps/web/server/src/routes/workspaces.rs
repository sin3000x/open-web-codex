use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, HeaderName, HeaderValue, Response, StatusCode},
    Extension, Json,
};
use open_web_codex_git_runtime::{CommitAuthor, GitRuntime, GitRuntimeError};
use open_web_codex_platform_contracts::error::PlatformError;
use open_web_codex_platform_contracts::{
    CommitWorkspaceRequest, CommitWorkspaceResponse, RenameWorkspaceUpstreamRequest,
    RunWorkspaceStatus, SetWorkspaceGitRootRequest, WorkspaceBranch, WorkspaceBranchRequest,
    WorkspaceCommitDiff, WorkspaceFileChange, WorkspaceFileContent, WorkspaceFileDiff,
    WorkspaceGitRootsQuery, WorkspaceLog, WorkspaceLogEntry, WorkspaceLogQuery, WorkspacePathQuery,
    WorkspacePathsRequest, WriteProfileTextFileRequest,
};
use open_web_codex_platform_store::AppState;
use open_web_codex_run_orchestrator::{
    RetireWorkspaceRequest, RunOrchestrator, RunOrchestratorError,
};
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

use crate::middleware::auth::AuthenticatedUser;

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<PlatformError>)>;
type AssetResult = Result<Response<Body>, (StatusCode, Json<PlatformError>)>;

pub async fn list_git_roots(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(run_id): Path<Uuid>,
    Query(query): Query<WorkspaceGitRootsQuery>,
    Extension(git): Extension<Arc<GitRuntime>>,
) -> ApiResult<Vec<String>> {
    let workspace_id = authorized_workspace(&state, &auth, run_id, false).await?;
    Ok(Json(
        git.list_git_roots(workspace_id, query.depth.unwrap_or(2))
            .await
            .map_err(git_error)?,
    ))
}

pub async fn set_git_root(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(run_id): Path<Uuid>,
    Extension(git): Extension<Arc<GitRuntime>>,
    Json(request): Json<SetWorkspaceGitRootRequest>,
) -> ApiResult<serde_json::Value> {
    let workspace_id = authorized_workspace(&state, &auth, run_id, true).await?;
    git.set_workspace_git_root(workspace_id, request.git_root.as_deref())
        .await
        .map_err(git_error)?;
    Ok(Json(json!({ "status": "selected" })))
}

pub async fn list_files(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(run_id): Path<Uuid>,
    Extension(git): Extension<Arc<GitRuntime>>,
) -> ApiResult<Vec<String>> {
    let workspace_id = authorized_workspace(&state, &auth, run_id, false).await?;
    Ok(Json(git.list_files(workspace_id).await.map_err(git_error)?))
}

pub async fn read_file(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(run_id): Path<Uuid>,
    Query(query): Query<WorkspacePathQuery>,
    Extension(git): Extension<Arc<GitRuntime>>,
) -> ApiResult<WorkspaceFileContent> {
    let workspace_id = authorized_workspace(&state, &auth, run_id, false).await?;
    let content = git
        .read_file(workspace_id, &query.path)
        .await
        .map_err(git_error)?;
    Ok(Json(WorkspaceFileContent {
        content: content.content,
        truncated: content.truncated,
    }))
}

pub async fn read_image_asset(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(run_id): Path<Uuid>,
    Query(query): Query<WorkspacePathQuery>,
    Extension(git): Extension<Arc<GitRuntime>>,
) -> AssetResult {
    let workspace_id = authorized_workspace(&state, &auth, run_id, false).await?;
    let asset = git
        .read_image_asset(workspace_id, &query.path)
        .await
        .map_err(git_error)?;
    let content_length = HeaderValue::from_str(&asset.bytes.len().to_string()).map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(PlatformError::internal(
                "workspace image response could not be created",
            )),
        )
    })?;
    let mut response = Response::new(Body::from(asset.bytes));
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(asset.media_type),
    );
    headers.insert(header::CONTENT_LENGTH, content_length);
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        HeaderName::from_static("cross-origin-resource-policy"),
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(
        HeaderName::from_static("referrer-policy"),
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        HeaderName::from_static("content-security-policy"),
        HeaderValue::from_static(
            "sandbox; default-src 'none'; style-src 'unsafe-inline'; img-src data:",
        ),
    );
    Ok(response)
}

pub async fn write_agents_file(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(run_id): Path<Uuid>,
    Extension(git): Extension<Arc<GitRuntime>>,
    Json(request): Json<WriteProfileTextFileRequest>,
) -> ApiResult<serde_json::Value> {
    let workspace_id = authorized_workspace(&state, &auth, run_id, true).await?;
    git.write_agents_md(workspace_id, &request.content)
        .await
        .map_err(git_error)?;
    audit_workspace_mutation(
        &state,
        &auth,
        run_id,
        workspace_id,
        "workspace.agents_write",
        &["AGENTS.md".to_string()],
    )
    .await?;
    Ok(Json(json!({ "status": "ok" })))
}

pub async fn diffs(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(run_id): Path<Uuid>,
    Extension(git): Extension<Arc<GitRuntime>>,
) -> ApiResult<Vec<WorkspaceFileDiff>> {
    let workspace_id = authorized_workspace(&state, &auth, run_id, false).await?;
    Ok(Json(
        git.diffs(workspace_id)
            .await
            .map_err(git_error)?
            .into_iter()
            .map(|diff| WorkspaceFileDiff {
                path: diff.path,
                diff: diff.diff,
                is_binary: diff.is_binary,
                truncated: diff.truncated,
            })
            .collect(),
    ))
}

pub async fn stage(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(run_id): Path<Uuid>,
    Extension(git): Extension<Arc<GitRuntime>>,
    Json(request): Json<WorkspacePathsRequest>,
) -> ApiResult<serde_json::Value> {
    let workspace_id = authorized_workspace(&state, &auth, run_id, true).await?;
    git.stage_paths(workspace_id, &request.paths)
        .await
        .map_err(git_error)?;
    audit_workspace_mutation(
        &state,
        &auth,
        run_id,
        workspace_id,
        "workspace.stage",
        &request.paths,
    )
    .await?;
    Ok(Json(json!({ "status": "staged" })))
}

pub async fn stage_all(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(run_id): Path<Uuid>,
    Extension(git): Extension<Arc<GitRuntime>>,
) -> ApiResult<serde_json::Value> {
    let workspace_id = authorized_workspace(&state, &auth, run_id, true).await?;
    git.stage_all(workspace_id).await.map_err(git_error)?;
    audit_workspace_mutation(
        &state,
        &auth,
        run_id,
        workspace_id,
        "workspace.stage_all",
        &[],
    )
    .await?;
    Ok(Json(json!({ "status": "staged" })))
}

pub async fn unstage(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(run_id): Path<Uuid>,
    Extension(git): Extension<Arc<GitRuntime>>,
    Json(request): Json<WorkspacePathsRequest>,
) -> ApiResult<serde_json::Value> {
    let workspace_id = authorized_workspace(&state, &auth, run_id, true).await?;
    git.unstage_paths(workspace_id, &request.paths)
        .await
        .map_err(git_error)?;
    audit_workspace_mutation(
        &state,
        &auth,
        run_id,
        workspace_id,
        "workspace.unstage",
        &request.paths,
    )
    .await?;
    Ok(Json(json!({ "status": "unstaged" })))
}

pub async fn revert(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(run_id): Path<Uuid>,
    Extension(git): Extension<Arc<GitRuntime>>,
    Json(request): Json<WorkspacePathsRequest>,
) -> ApiResult<serde_json::Value> {
    let workspace_id = authorized_workspace(&state, &auth, run_id, true).await?;
    git.revert_paths(workspace_id, &request.paths)
        .await
        .map_err(git_error)?;
    audit_workspace_mutation(
        &state,
        &auth,
        run_id,
        workspace_id,
        "workspace.revert",
        &request.paths,
    )
    .await?;
    Ok(Json(json!({ "status": "reverted" })))
}

pub async fn revert_all(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(run_id): Path<Uuid>,
    Extension(git): Extension<Arc<GitRuntime>>,
) -> ApiResult<serde_json::Value> {
    let workspace_id = authorized_workspace(&state, &auth, run_id, true).await?;
    git.revert_all(workspace_id).await.map_err(git_error)?;
    audit_workspace_mutation(
        &state,
        &auth,
        run_id,
        workspace_id,
        "workspace.revert_all",
        &[],
    )
    .await?;
    Ok(Json(json!({ "status": "reverted" })))
}

pub async fn list_branches(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(run_id): Path<Uuid>,
    Extension(git): Extension<Arc<GitRuntime>>,
) -> ApiResult<Vec<WorkspaceBranch>> {
    let workspace_id = authorized_workspace(&state, &auth, run_id, false).await?;
    Ok(Json(
        git.list_branches(workspace_id)
            .await
            .map_err(git_error)?
            .into_iter()
            .map(|branch| WorkspaceBranch {
                name: branch.name,
                last_commit: branch.last_commit,
            })
            .collect(),
    ))
}

pub async fn checkout_branch(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(run_id): Path<Uuid>,
    Extension(git): Extension<Arc<GitRuntime>>,
    Json(request): Json<WorkspaceBranchRequest>,
) -> ApiResult<serde_json::Value> {
    let workspace_id = authorized_workspace(&state, &auth, run_id, true).await?;
    git.checkout_branch(workspace_id, &request.name)
        .await
        .map_err(git_error)?;
    audit_workspace_mutation(
        &state,
        &auth,
        run_id,
        workspace_id,
        "workspace.branch_checkout",
        &[],
    )
    .await?;
    Ok(Json(json!({ "status": "checkedOut" })))
}

pub async fn create_branch(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(run_id): Path<Uuid>,
    Extension(git): Extension<Arc<GitRuntime>>,
    Json(request): Json<WorkspaceBranchRequest>,
) -> ApiResult<serde_json::Value> {
    let workspace_id = authorized_workspace(&state, &auth, run_id, true).await?;
    git.create_branch(workspace_id, &request.name)
        .await
        .map_err(git_error)?;
    audit_workspace_mutation(
        &state,
        &auth,
        run_id,
        workspace_id,
        "workspace.branch_create",
        &[],
    )
    .await?;
    Ok(Json(json!({ "status": "created" })))
}

pub async fn rename_branch(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(run_id): Path<Uuid>,
    Extension(git): Extension<Arc<GitRuntime>>,
    Json(request): Json<WorkspaceBranchRequest>,
) -> ApiResult<serde_json::Value> {
    let workspace_id = authorized_workspace(&state, &auth, run_id, true).await?;
    let run_metadata = sqlx::query(
        "SELECT workspace_kind, source_ref, workspace_name FROM runs \
         WHERE id = $1 AND organization_id = $2",
    )
    .bind(run_id)
    .bind(auth.organization_id)
    .fetch_one(&state.db)
    .await
    .map_err(database_error)?;
    if run_metadata.get::<String, _>("workspace_kind") == "main" {
        return Err((
            StatusCode::CONFLICT,
            Json(PlatformError::bad_request(
                "the main Run workspace cannot be renamed",
            )),
        ));
    }
    let final_name = git
        .rename_branch(workspace_id, &request.name)
        .await
        .map_err(git_error)?;
    let name = final_name.as_str();
    let mut transaction = state.db.begin().await.map_err(database_error)?;
    let workspace_name: Option<String> = sqlx::query_scalar(
        "UPDATE runs SET workspace_name = CASE \
             WHEN workspace_name IS NULL OR btrim(workspace_name) = source_ref THEN $1 \
             ELSE workspace_name END, \
             source_ref = $1, updated_at = now() \
         WHERE id = $2 AND organization_id = $3 AND workspace_kind <> 'main' \
         RETURNING workspace_name",
    )
    .bind(name)
    .bind(run_id)
    .bind(auth.organization_id)
    .fetch_one(&mut *transaction)
    .await
    .map_err(database_error)?;
    sqlx::query(
        "UPDATE workspaces SET branch_name = $1, updated_at = now() \
         WHERE id = $2 AND organization_id = $3",
    )
    .bind(name)
    .bind(workspace_id)
    .bind(auth.organization_id)
    .execute(&mut *transaction)
    .await
    .map_err(database_error)?;
    transaction.commit().await.map_err(database_error)?;
    audit_workspace_mutation(
        &state,
        &auth,
        run_id,
        workspace_id,
        "workspace.branch_rename",
        &[],
    )
    .await?;
    Ok(Json(
        json!({ "status": "renamed", "name": workspace_name.unwrap_or_else(|| name.to_string()) }),
    ))
}

pub async fn remove_derived_workspace(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(run_id): Path<Uuid>,
    Extension(orchestrator): Extension<Arc<RunOrchestrator>>,
) -> ApiResult<serde_json::Value> {
    let child_run_ids = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM runs WHERE organization_id = $1 AND workspace_group_run_id = $2",
    )
    .bind(auth.organization_id)
    .bind(run_id)
    .fetch_all(&state.db)
    .await
    .map_err(database_error)?;
    for child_run_id in child_run_ids {
        orchestrator
            .retire_workspace(RetireWorkspaceRequest {
                organization_id: auth.organization_id,
                actor_id: auth.user_id,
                allow_organization_admin: matches!(
                    auth.organization_role.as_str(),
                    "owner" | "admin"
                ),
                run_id: child_run_id,
            })
            .await
            .map_err(orchestrator_error)?;
    }
    let run = orchestrator
        .retire_workspace(RetireWorkspaceRequest {
            organization_id: auth.organization_id,
            actor_id: auth.user_id,
            allow_organization_admin: matches!(auth.organization_role.as_str(), "owner" | "admin"),
            run_id,
        })
        .await
        .map_err(orchestrator_error)?;
    if let Some(workspace_id) = run.workspace_id {
        audit_workspace_mutation(&state, &auth, run_id, workspace_id, "workspace.remove", &[])
            .await?;
    }
    Ok(Json(json!({ "status": "cleanupQueued" })))
}

pub async fn apply_derived_workspace(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(run_id): Path<Uuid>,
    Extension(git): Extension<Arc<GitRuntime>>,
) -> ApiResult<serde_json::Value> {
    let source_workspace_id = authorized_workspace(&state, &auth, run_id, true).await?;
    let row = sqlx::query(
        "SELECT source.workspace_kind, parent.workspace_id AS parent_workspace_id, \
                parent.requested_by AS parent_requested_by, parent_workspace.state AS parent_state \
         FROM runs source \
         JOIN runs parent ON parent.id = source.workspace_parent_run_id \
           AND parent.organization_id = source.organization_id \
         JOIN workspaces parent_workspace ON parent_workspace.id = parent.workspace_id \
           AND parent_workspace.organization_id = source.organization_id \
         WHERE source.id = $1 AND source.organization_id = $2",
    )
    .bind(run_id)
    .bind(auth.organization_id)
    .fetch_optional(&state.db)
    .await
    .map_err(database_error)?
    .ok_or_else(|| {
        (
            StatusCode::CONFLICT,
            Json(PlatformError::bad_request(
                "worktree parent Run is no longer available",
            )),
        )
    })?;
    if row.get::<String, _>("workspace_kind") != "worktree" {
        return Err((
            StatusCode::CONFLICT,
            Json(PlatformError::bad_request(
                "only worktree changes can be applied to a parent Run",
            )),
        ));
    }
    if row.get::<String, _>("parent_state") == "retired" {
        return Err((
            StatusCode::CONFLICT,
            Json(PlatformError::bad_request(
                "parent Run workspace has been retired",
            )),
        ));
    }
    let parent_requested_by: Option<Uuid> = row.get("parent_requested_by");
    if parent_requested_by != Some(auth.user_id)
        && !matches!(auth.organization_role.as_str(), "owner" | "admin")
    {
        return Err((
            StatusCode::NOT_FOUND,
            Json(PlatformError::not_found(
                "parent Run workspace was not found",
            )),
        ));
    }
    let parent_workspace_id: Uuid = row.get("parent_workspace_id");
    git.apply_workspace_changes(source_workspace_id, parent_workspace_id)
        .await
        .map_err(git_error)?;
    audit_workspace_mutation(
        &state,
        &auth,
        run_id,
        source_workspace_id,
        "workspace.apply_to_parent",
        &[],
    )
    .await?;
    Ok(Json(json!({ "status": "applied" })))
}

pub async fn rename_upstream_branch(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(run_id): Path<Uuid>,
    Extension(git): Extension<Arc<GitRuntime>>,
    Json(request): Json<RenameWorkspaceUpstreamRequest>,
) -> ApiResult<serde_json::Value> {
    let workspace_id = authorized_workspace(&state, &auth, run_id, true).await?;
    let workspace_kind: String = sqlx::query_scalar(
        "SELECT workspace_kind FROM runs WHERE id = $1 AND organization_id = $2",
    )
    .bind(run_id)
    .bind(auth.organization_id)
    .fetch_one(&state.db)
    .await
    .map_err(database_error)?;
    if workspace_kind != "worktree" {
        return Err((
            StatusCode::CONFLICT,
            Json(PlatformError::bad_request(
                "only worktree branches can rename their upstream",
            )),
        ));
    }
    git.rename_upstream_branch(workspace_id, &request.old_branch, &request.new_branch)
        .await
        .map_err(git_error)?;
    audit_workspace_mutation(
        &state,
        &auth,
        run_id,
        workspace_id,
        "workspace.upstream_rename",
        &[],
    )
    .await?;
    Ok(Json(json!({ "status": "renamed" })))
}

pub async fn log(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(run_id): Path<Uuid>,
    Query(query): Query<WorkspaceLogQuery>,
    Extension(git): Extension<Arc<GitRuntime>>,
) -> ApiResult<WorkspaceLog> {
    let workspace_id = authorized_workspace(&state, &auth, run_id, false).await?;
    let log = git
        .log(workspace_id, query.limit.unwrap_or(40))
        .await
        .map_err(git_error)?;
    Ok(Json(WorkspaceLog {
        total: log.total,
        entries: log.entries.into_iter().map(log_entry).collect(),
        ahead: log.ahead,
        behind: log.behind,
        ahead_entries: log.ahead_entries.into_iter().map(log_entry).collect(),
        behind_entries: log.behind_entries.into_iter().map(log_entry).collect(),
        upstream: log.upstream,
    }))
}

pub async fn commit_diffs(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path((run_id, sha)): Path<(Uuid, String)>,
    Extension(git): Extension<Arc<GitRuntime>>,
) -> ApiResult<Vec<WorkspaceCommitDiff>> {
    let workspace_id = authorized_workspace(&state, &auth, run_id, false).await?;
    Ok(Json(
        git.commit_diffs(workspace_id, &sha)
            .await
            .map_err(git_error)?
            .into_iter()
            .map(|diff| WorkspaceCommitDiff {
                path: diff.path,
                status: diff.status,
                diff: diff.diff,
                is_binary: diff.is_binary,
            })
            .collect(),
    ))
}

pub async fn remote(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(run_id): Path<Uuid>,
    Extension(git): Extension<Arc<GitRuntime>>,
) -> ApiResult<Option<String>> {
    let workspace_id = authorized_workspace(&state, &auth, run_id, false).await?;
    Ok(Json(git.remote(workspace_id).await.map_err(git_error)?))
}

pub async fn fetch(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(run_id): Path<Uuid>,
    Extension(git): Extension<Arc<GitRuntime>>,
) -> ApiResult<serde_json::Value> {
    mutate_remote(&state, &auth, run_id, &git, "fetch").await
}

pub async fn pull(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(run_id): Path<Uuid>,
    Extension(git): Extension<Arc<GitRuntime>>,
) -> ApiResult<serde_json::Value> {
    mutate_remote(&state, &auth, run_id, &git, "pull").await
}

pub async fn push(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(run_id): Path<Uuid>,
    Extension(git): Extension<Arc<GitRuntime>>,
) -> ApiResult<serde_json::Value> {
    mutate_remote(&state, &auth, run_id, &git, "push").await
}

pub async fn sync(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(run_id): Path<Uuid>,
    Extension(git): Extension<Arc<GitRuntime>>,
) -> ApiResult<serde_json::Value> {
    mutate_remote(&state, &auth, run_id, &git, "sync").await
}

pub async fn status(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(run_id): Path<Uuid>,
    Extension(git): Extension<Arc<GitRuntime>>,
) -> ApiResult<RunWorkspaceStatus> {
    let workspace_id = authorized_workspace(&state, &auth, run_id, false).await?;
    let status = git.status(workspace_id).await.map_err(git_error)?;
    Ok(Json(RunWorkspaceStatus {
        workspace_id,
        branch: status.branch,
        head_commit: status.head_commit,
        changes: status
            .changes
            .into_iter()
            .map(|change| WorkspaceFileChange {
                path: change.path,
                status: change.status,
                additions: change.additions,
                deletions: change.deletions,
                binary: change.binary,
                size_bytes: change.size_bytes,
                large: change.large,
            })
            .collect(),
    }))
}

pub async fn commit(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(run_id): Path<Uuid>,
    Extension(git): Extension<Arc<GitRuntime>>,
    Json(request): Json<CommitWorkspaceRequest>,
) -> ApiResult<CommitWorkspaceResponse> {
    let workspace_id = authorized_workspace(&state, &auth, run_id, true).await?;
    let author = CommitAuthor {
        name: auth.name.clone(),
        email: auth.email.clone(),
    };
    let commit = git
        .commit_selected(
            workspace_id,
            &request.selected_paths,
            &request.message,
            &author,
        )
        .await
        .map_err(git_error)?;
    let mut transaction = state.db.begin().await.map_err(database_error)?;
    sqlx::query(
        "UPDATE workspaces SET head_commit = $1, updated_at = now() \
         WHERE id = $2 AND organization_id = $3",
    )
    .bind(&commit)
    .bind(workspace_id)
    .bind(auth.organization_id)
    .execute(&mut *transaction)
    .await
    .map_err(database_error)?;
    sqlx::query(
        "INSERT INTO audit_log \
         (organization_id, actor_id, action, target_type, target_id, metadata, outcome) \
         VALUES ($1, $2, 'workspace.commit', 'workspace', $3, $4, 'success')",
    )
    .bind(auth.organization_id)
    .bind(auth.user_id)
    .bind(workspace_id)
    .bind(json!({
        "runId": run_id,
        "commit": commit,
        "selectedPathCount": request.selected_paths.len(),
    }))
    .execute(&mut *transaction)
    .await
    .map_err(database_error)?;
    transaction.commit().await.map_err(database_error)?;
    Ok(Json(CommitWorkspaceResponse {
        workspace_id,
        commit,
    }))
}

pub(super) async fn authorized_workspace(
    state: &AppState,
    auth: &AuthenticatedUser,
    run_id: Uuid,
    require_owner: bool,
) -> Result<Uuid, (StatusCode, Json<PlatformError>)> {
    let row = sqlx::query(
        "SELECT r.workspace_id, r.requested_by, w.state \
         FROM runs r JOIN workspaces w ON w.id = r.workspace_id \
         WHERE r.id = $1 AND r.organization_id = $2 AND w.organization_id = $2",
    )
    .bind(run_id)
    .bind(auth.organization_id)
    .fetch_optional(&state.db)
    .await
    .map_err(database_error)?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(PlatformError::not_found("Run workspace was not found")),
        )
    })?;
    if row.get::<String, _>("state") == "retired" {
        return Err((
            StatusCode::CONFLICT,
            Json(PlatformError::bad_request("Run workspace has been retired")),
        ));
    }
    let requested_by: Option<Uuid> = row.get("requested_by");
    let can_manage = requested_by == Some(auth.user_id)
        || matches!(auth.organization_role.as_str(), "owner" | "admin");
    if require_owner && !can_manage {
        return Err((
            StatusCode::NOT_FOUND,
            Json(PlatformError::not_found("Run workspace was not found")),
        ));
    }
    Ok(row.get("workspace_id"))
}

async fn audit_workspace_mutation(
    state: &AppState,
    auth: &AuthenticatedUser,
    run_id: Uuid,
    workspace_id: Uuid,
    action: &str,
    paths: &[String],
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
    .bind(json!({ "runId": run_id, "paths": paths }))
    .execute(&state.db)
    .await
    .map_err(database_error)?;
    Ok(())
}

async fn mutate_remote(
    state: &AppState,
    auth: &AuthenticatedUser,
    run_id: Uuid,
    git: &GitRuntime,
    operation: &str,
) -> ApiResult<serde_json::Value> {
    let workspace_id = authorized_workspace(state, auth, run_id, true).await?;
    match operation {
        "fetch" => git.fetch(workspace_id).await,
        "pull" => git.pull(workspace_id).await,
        "push" => git.push(workspace_id).await,
        "sync" => git.sync(workspace_id).await,
        _ => unreachable!("remote operation is route-owned"),
    }
    .map_err(git_error)?;
    audit_workspace_mutation(
        state,
        auth,
        run_id,
        workspace_id,
        &format!("workspace.{operation}"),
        &[],
    )
    .await?;
    Ok(Json(json!({ "status": operation })))
}

fn log_entry(entry: open_web_codex_git_runtime::GitLogEntry) -> WorkspaceLogEntry {
    WorkspaceLogEntry {
        sha: entry.sha,
        summary: entry.summary,
        author: entry.author,
        timestamp: entry.timestamp,
    }
}

fn git_error(error: GitRuntimeError) -> (StatusCode, Json<PlatformError>) {
    match error {
        GitRuntimeError::InvalidSource(_)
        | GitRuntimeError::InvalidRef(_)
        | GitRuntimeError::UnsafePath(_)
        | GitRuntimeError::Conflict(_)
        | GitRuntimeError::NoChanges => (
            StatusCode::CONFLICT,
            Json(PlatformError::bad_request(
                "Git workspace request was rejected",
            )),
        ),
        GitRuntimeError::UnsupportedImage(_) => (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Json(PlatformError::bad_request(
                "Workspace image type was rejected",
            )),
        ),
        GitRuntimeError::ImageTooLarge => (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(PlatformError::bad_request(
                "Workspace image exceeds the maximum supported size",
            )),
        ),
        GitRuntimeError::Git { .. } | GitRuntimeError::Io { .. } => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(PlatformError::internal("Git workspace operation failed")),
        ),
    }
}

fn database_error(_error: sqlx::Error) -> (StatusCode, Json<PlatformError>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(PlatformError::internal("database operation failed")),
    )
}

fn orchestrator_error(error: RunOrchestratorError) -> (StatusCode, Json<PlatformError>) {
    let (status, platform) = match error {
        RunOrchestratorError::Invalid(message) => {
            (StatusCode::BAD_REQUEST, PlatformError::bad_request(message))
        }
        RunOrchestratorError::NotFound => (
            StatusCode::NOT_FOUND,
            PlatformError::not_found("Run workspace was not found"),
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
            PlatformError::internal("Run workspace operation failed"),
        ),
    };
    (status, Json(platform))
}
