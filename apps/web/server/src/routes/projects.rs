use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use open_web_codex_git_runtime::GitRuntime;
use open_web_codex_platform_contracts::error::PlatformError;
use open_web_codex_platform_contracts::{
    CreateManagedProjectRequest, CreateProjectRequest, Project, ProjectThreadContext, Run, Task,
};
use open_web_codex_platform_store::AppState;
use sqlx::Row;
use std::sync::Arc;
use uuid::Uuid;

use crate::middleware::auth::AuthenticatedUser;

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<PlatformError>)>;

/// GET /api/projects/:id/thread-contexts — one joined navigation projection.
pub async fn list_thread_contexts(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
) -> ApiResult<Vec<ProjectThreadContext>> {
    let rows = sqlx::query(
        "SELECT p.id AS project_id, p.name AS project_name, p.git_url, p.default_branch, \
                p.created_at AS project_created_at, p.updated_at AS project_updated_at, \
                t.id AS task_id, t.title, t.status AS task_status, \
                t.model_provider, t.model, \
                t.created_at AS task_created_at, t.updated_at AS task_updated_at, \
                r.id AS run_id, r.status AS run_status, r.codex_thread_id, r.active_turn_id, \
                r.workspace_id, r.source_ref, r.workspace_kind, r.workspace_name, \
                r.workspace_parent_run_id, r.workspace_group_run_id, r.attempt, \
                r.created_at AS run_created_at, r.updated_at AS run_updated_at \
         FROM projects p JOIN tasks t ON t.project_id = p.id \
         JOIN runs r ON r.task_id = t.id \
         WHERE p.id = $1 AND p.organization_id = $2 AND t.organization_id = $2 \
           AND r.organization_id = $2 AND r.codex_thread_id IS NOT NULL \
         ORDER BY r.created_at DESC",
    )
    .bind(project_id)
    .bind(auth.organization_id)
    .fetch_all(&state.db)
    .await
    .map_err(internal_database_error)?;
    Ok(Json(
        rows.iter()
            .map(|row| ProjectThreadContext {
                project: Project {
                    id: row.get("project_id"),
                    name: row.get("project_name"),
                    git_url: row.get("git_url"),
                    default_branch: row.get("default_branch"),
                    created_at: row.get("project_created_at"),
                    updated_at: row.get("project_updated_at"),
                },
                task: Task {
                    id: row.get("task_id"),
                    project_id,
                    title: row.get("title"),
                    status: row.get("task_status"),
                    model_provider: row.get("model_provider"),
                    model: row.get("model"),
                    created_at: row.get("task_created_at"),
                    updated_at: row.get("task_updated_at"),
                },
                run: Run {
                    id: row.get("run_id"),
                    task_id: row.get("task_id"),
                    status: row.get("run_status"),
                    codex_thread_id: row.get("codex_thread_id"),
                    active_turn_id: row.get("active_turn_id"),
                    workspace_id: row.get("workspace_id"),
                    source_ref: row.get("source_ref"),
                    workspace_kind: row.get("workspace_kind"),
                    workspace_name: row.get("workspace_name"),
                    workspace_parent_run_id: row.get("workspace_parent_run_id"),
                    workspace_group_run_id: row.get("workspace_group_run_id"),
                    attempt: row.get("attempt"),
                    created_at: row.get("run_created_at"),
                    updated_at: row.get("run_updated_at"),
                },
            })
            .collect(),
    ))
}

/// GET /api/projects
pub async fn list_projects(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
) -> ApiResult<Vec<Project>> {
    let rows = sqlx::query(
        "SELECT id, name, git_url, default_branch, created_at, updated_at \
         FROM projects WHERE organization_id = $1 ORDER BY created_at DESC",
    )
    .bind(auth.organization_id)
    .fetch_all(&state.db)
    .await
    .map_err(internal_database_error)?;

    let projects: Vec<Project> = rows
        .iter()
        .map(|row| Project {
            id: row.get("id"),
            name: row.get("name"),
            git_url: row.get("git_url"),
            default_branch: row.get("default_branch"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
        })
        .collect();

    Ok(Json(projects))
}

/// POST /api/projects
pub async fn create_project(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
    Extension(git): Extension<Arc<GitRuntime>>,
    Json(req): Json<CreateProjectRequest>,
) -> ApiResult<Project> {
    if req.name.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(PlatformError::bad_request("name must not be empty")),
        ));
    }
    if auth.organization_role != "owner" && auth.organization_role != "admin" {
        return Err((
            StatusCode::FORBIDDEN,
            Json(PlatformError::forbidden(
                "only Organization owners and admins can create projects",
            )),
        ));
    }
    let branch = req.default_branch.unwrap_or_else(|| "main".to_string());
    git.validate_external_source(&req.git_url).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(PlatformError::bad_request(
                "git_url is not an allowed Git source",
            )),
        )
    })?;
    git.validate_ref(&branch).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(PlatformError::bad_request(
                "default_branch is not a valid Git ref",
            )),
        )
    })?;
    let row = sqlx::query(
        "INSERT INTO projects (organization_id, created_by, name, git_url, default_branch) VALUES ($1, $2, $3, $4, $5) \
         RETURNING id, name, git_url, default_branch, created_at, updated_at",
    )
    .bind(auth.organization_id)
    .bind(auth.user_id)
    .bind(&req.name)
    .bind(&req.git_url)
    .bind(&branch)
    .fetch_one(&state.db)
    .await
    .map_err(internal_database_error)?;

    Ok(Json(Project {
        id: row.get("id"),
        name: row.get("name"),
        git_url: row.get("git_url"),
        default_branch: row.get("default_branch"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }))
}

/// POST /api/projects/managed — creates an empty server-owned Git project.
pub async fn create_managed_project(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
    Json(req): Json<CreateManagedProjectRequest>,
) -> ApiResult<Project> {
    let name = req.name.trim();
    if name.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(PlatformError::bad_request("name must not be empty")),
        ));
    }
    if auth.organization_role != "owner" && auth.organization_role != "admin" {
        return Err((
            StatusCode::FORBIDDEN,
            Json(PlatformError::forbidden(
                "only Organization owners and admins can create projects",
            )),
        ));
    }

    let id = Uuid::now_v7();
    let git_url = format!("managed://{id}");
    let row = sqlx::query(
        "INSERT INTO projects (id, organization_id, created_by, name, git_url, default_branch) VALUES ($1, $2, $3, $4, $5, 'main') \
         RETURNING id, name, git_url, default_branch, created_at, updated_at",
    )
    .bind(id)
    .bind(auth.organization_id)
    .bind(auth.user_id)
    .bind(name)
    .bind(&git_url)
    .fetch_one(&state.db)
    .await
    .map_err(internal_database_error)?;

    Ok(Json(Project {
        id: row.get("id"),
        name: row.get("name"),
        git_url: row.get("git_url"),
        default_branch: row.get("default_branch"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }))
}

/// GET /api/projects/:id
pub async fn get_project(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Project> {
    let row = sqlx::query(
        "SELECT id, name, git_url, default_branch, created_at, updated_at \
         FROM projects WHERE id = $1 AND organization_id = $2",
    )
    .bind(id)
    .bind(auth.organization_id)
    .fetch_optional(&state.db)
    .await
    .map_err(internal_database_error)?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(PlatformError::not_found(format!("project {id} not found"))),
        )
    })?;

    Ok(Json(Project {
        id: row.get("id"),
        name: row.get("name"),
        git_url: row.get("git_url"),
        default_branch: row.get("default_branch"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }))
}

/// DELETE /api/projects/:id
pub async fn delete_project(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<serde_json::Value> {
    if auth.organization_role != "owner" && auth.organization_role != "admin" {
        return Err((
            StatusCode::FORBIDDEN,
            Json(PlatformError::forbidden(
                "only Organization owners and admins can delete projects",
            )),
        ));
    }
    let deleted = sqlx::query("DELETE FROM projects WHERE id = $1 AND organization_id = $2")
        .bind(id)
        .bind(auth.organization_id)
        .execute(&state.db)
        .await
        .map_err(internal_database_error)?;
    if deleted.rows_affected() == 0 {
        return Err((
            StatusCode::NOT_FOUND,
            Json(PlatformError::not_found(format!("project {id} not found"))),
        ));
    }
    Ok(Json(serde_json::json!({ "deleted": true, "id": id })))
}

fn internal_database_error(_error: sqlx::Error) -> (StatusCode, Json<PlatformError>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(PlatformError::internal("database operation failed")),
    )
}
