use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use open_web_codex_adapter::{AuthorizedWorkspace, CodexAdapter, TurnOptions};
use open_web_codex_platform_contracts::error::PlatformError;
use open_web_codex_platform_contracts::{
    CreateTaskRequest, ListTaskEventsParams, ModelSelection, RunEvent, SendMessageRequest,
    SendMessageResponse, Task, UpdateTaskModelSelectionRequest,
};
use open_web_codex_platform_store::configuration::{
    get_global, put_global, MODEL_SELECTION_CONFIG_KEY,
};
use open_web_codex_platform_store::AppState;
use open_web_codex_provider_service::secured::{AuthorizedProviderOperations, ProviderActor};
use open_web_codex_run_orchestrator::{RecoverRunRequest, RunOrchestrator};
use serde::Deserialize;
use sqlx::Row;
use std::sync::Arc;
use uuid::Uuid;

use crate::middleware::auth::{require_runtime_profile, AuthenticatedUser};
use crate::routes::RuntimeProfileBinding;

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<PlatformError>)>;

#[derive(Deserialize)]
pub struct ListTasksParams {
    pub project_id: Uuid,
}

/// GET /api/tasks?project_id=...
pub async fn list_tasks(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
    Query(params): Query<ListTasksParams>,
) -> ApiResult<Vec<Task>> {
    let rows = sqlx::query(
        "SELECT id, project_id, title, status, model_provider, model, created_at, updated_at \
         FROM tasks WHERE project_id = $1 AND organization_id = $2 ORDER BY created_at DESC",
    )
    .bind(params.project_id)
    .bind(auth.organization_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(PlatformError::internal(format!("{e}"))),
        )
    })?;

    let tasks: Vec<Task> = rows
        .iter()
        .map(|row| Task {
            id: row.get("id"),
            project_id: row.get("project_id"),
            title: row.get("title"),
            status: row.get("status"),
            model_provider: row.get("model_provider"),
            model: row.get("model"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
        })
        .collect();

    Ok(Json(tasks))
}

/// POST /api/tasks
pub async fn create_task(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
    Json(req): Json<CreateTaskRequest>,
) -> ApiResult<Task> {
    if req.title.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(PlatformError::bad_request("title must not be empty")),
        ));
    }
    let requested_selection = normalize_model_selection(req.model_provider, req.model)?;
    let selection = match requested_selection {
        Some(selection) => Some(selection),
        None => load_default_model_selection(&state).await?,
    };

    let row = sqlx::query(
        "INSERT INTO tasks \
         (organization_id, project_id, created_by, title, model_provider, model) \
         SELECT organization_id, id, $4, $2, $5, $6 \
         FROM projects WHERE id = $1 AND organization_id = $3 \
         RETURNING id, project_id, title, status, model_provider, model, created_at, updated_at",
    )
    .bind(req.project_id)
    .bind(&req.title)
    .bind(auth.organization_id)
    .bind(auth.user_id)
    .bind(selection.as_ref().map(|value| value.provider_id.as_str()))
    .bind(selection.as_ref().map(|value| value.model_id.as_str()))
    .fetch_optional(&state.db)
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(PlatformError::internal(format!("{e}"))),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(PlatformError::not_found("project not found")),
        )
    })?;

    Ok(Json(Task {
        id: row.get("id"),
        project_id: row.get("project_id"),
        title: row.get("title"),
        status: row.get("status"),
        model_provider: row.get("model_provider"),
        model: row.get("model"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }))
}

/// PUT /api/tasks/:id/model-selection — persist the Provider/model pair that
/// belongs to this Thread. This does not alter any other existing Thread.
pub async fn update_model_selection(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
    Path(task_id): Path<Uuid>,
    Extension(providers): Extension<Arc<dyn AuthorizedProviderOperations>>,
    Json(request): Json<UpdateTaskModelSelectionRequest>,
) -> ApiResult<ModelSelection> {
    let selection = normalize_model_selection(Some(request.provider_id), Some(request.model_id))?
        .expect("request contains both model selection fields");
    let catalog = providers
        .list(ProviderActor {
            user_id: auth.user_id,
            organization_id: auth.organization_id,
        })
        .await
        .map_err(|_| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(PlatformError::internal(
                    "Provider catalog is temporarily unavailable",
                )),
            )
        })?;
    let provider = catalog
        .data
        .iter()
        .find(|provider| provider.id == selection.provider_id)
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(PlatformError::bad_request(
                    "selected Provider does not exist",
                )),
            )
        })?;
    if !provider.models.is_empty()
        && !provider
            .models
            .iter()
            .any(|model| model.model_id == selection.model_id && model.show_in_picker)
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(PlatformError::bad_request(
                "selected model is not available for this Provider",
            )),
        ));
    }
    let updated = sqlx::query(
        "UPDATE tasks SET model_provider = $1, model = $2, updated_at = now() \
         WHERE id = $3 AND organization_id = $4 \
         RETURNING id",
    )
    .bind(&selection.provider_id)
    .bind(&selection.model_id)
    .bind(task_id)
    .bind(auth.organization_id)
    .fetch_optional(&state.db)
    .await
    .map_err(database_error)?
    .is_some();
    if !updated {
        return Err((
            StatusCode::NOT_FOUND,
            Json(PlatformError::not_found(format!(
                "task {task_id} not found"
            ))),
        ));
    }
    persist_default_model_selection(&state, auth.user_id, &selection).await?;
    Ok(Json(selection))
}

/// GET /api/tasks/:id/events — list persisted run events for a task.
pub async fn list_task_events(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
    Path(task_id): Path<Uuid>,
    Query(params): Query<ListTaskEventsParams>,
) -> ApiResult<Vec<RunEvent>> {
    let limit = params.limit.unwrap_or(50).min(200);
    let query = match params.after_sequence {
        Some(after) => sqlx::query(
            "SELECT e.id, e.sequence, e.run_id, e.event_type, e.projection_version, \
                    e.thread_id, e.turn_id, e.item_id, e.payload, e.created_at \
             FROM run_events e \
             JOIN runs r ON r.id = e.run_id \
             WHERE r.task_id = $1 AND r.organization_id = $2 AND e.sequence > $3 \
             ORDER BY e.sequence ASC LIMIT $4",
        )
        .bind(task_id)
        .bind(auth.organization_id)
        .bind(after)
        .bind(limit),
        None => sqlx::query(
            "SELECT * FROM ( \
                 SELECT e.id, e.sequence, e.run_id, e.event_type, e.projection_version, \
                        e.thread_id, e.turn_id, e.item_id, e.payload, e.created_at \
                 FROM run_events e \
                 JOIN runs r ON r.id = e.run_id \
                 WHERE r.task_id = $1 AND r.organization_id = $2 \
                 ORDER BY e.sequence DESC LIMIT $3 \
             ) recent ORDER BY sequence ASC",
        )
        .bind(task_id)
        .bind(auth.organization_id)
        .bind(limit),
    };

    let rows = query.fetch_all(&state.db).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(PlatformError::internal(format!("{e}"))),
        )
    })?;

    let events: Vec<RunEvent> = rows
        .iter()
        .map(|row| {
            let payload: serde_json::Value = row.get("payload");
            RunEvent {
                id: row.get("id"),
                sequence: row.get("sequence"),
                run_id: row.get("run_id"),
                event_type: row.get("event_type"),
                projection_version: row.get("projection_version"),
                thread_id: row.get("thread_id"),
                turn_id: row.get("turn_id"),
                item_id: row.get("item_id"),
                payload,
                created_at: row.get("created_at"),
            }
        })
        .collect();

    Ok(Json(events))
}

/// POST /api/tasks/:id/messages — send a user message to the task's active thread.
pub async fn send_message(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(task_id): Path<Uuid>,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
    Extension(orchestrator): Extension<Arc<RunOrchestrator>>,
    Extension(profile): Extension<RuntimeProfileBinding>,
    Json(req): Json<SendMessageRequest>,
) -> ApiResult<SendMessageResponse> {
    if req.text.trim().is_empty() && req.images.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(PlatformError::bad_request(
                "message text or at least one image is required",
            )),
        ));
    }
    require_runtime_profile(&state.db, &auth, &profile.runtime_key).await?;

    // Resolve the server-owned workspace; the browser never supplies a path.
    let active_run = sqlx::query(
        "SELECT r.id, r.status, r.codex_thread_id, r.workspace_id, w.root_path, \
                t.title, t.model_provider, t.model \
         FROM runs r JOIN tasks t ON t.id = r.task_id \
         LEFT JOIN workspaces w ON w.id = r.workspace_id \
         WHERE r.task_id = $1 AND r.organization_id = $2 \
           AND r.requested_by = $3 AND r.status IN ('running', 'recovery_pending') \
         ORDER BY r.created_at DESC LIMIT 1",
    )
    .bind(task_id)
    .bind(auth.organization_id)
    .bind(auth.user_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(PlatformError::internal(format!("{e}"))),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(PlatformError::bad_request(
                "no active owned Run for this Task; start a Run first",
            )),
        )
    })?;

    let thread_id: Option<String> = active_run.get("codex_thread_id");
    let thread_id = thread_id.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(PlatformError::bad_request(
                "active run has no thread yet; try again shortly",
            )),
        )
    })?;
    let workspace_id: Option<Uuid> = active_run.get("workspace_id");
    let root_path: Option<String> = active_run.get("root_path");
    let workspace = match (workspace_id, root_path) {
        (Some(workspace_id), Some(root)) => AuthorizedWorkspace {
            id: workspace_id.to_string(),
            root: root.into(),
        },
        _ => {
            return Err((
                StatusCode::CONFLICT,
                Json(PlatformError::bad_request(
                    "active Run workspace is not ready",
                )),
            ));
        }
    };
    if active_run.get::<String, _>("status") == "recovery_pending" {
        orchestrator
            .recover_run(RecoverRunRequest {
                organization_id: auth.organization_id,
                actor_id: auth.user_id,
                allow_organization_admin: matches!(
                    auth.organization_role.as_str(),
                    "owner" | "admin"
                ),
                run_id: active_run.get("id"),
            })
            .await
            .map_err(super::runs::orchestrator_error)?;
    }
    if req.model.is_some() != req.model_provider.is_some() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(PlatformError::bad_request(
                "model and model_provider must be selected together",
            )),
        ));
    }
    let selection = if req.model.is_some() {
        normalize_model_selection(req.model_provider.clone(), req.model.clone())?
    } else {
        normalize_model_selection(active_run.get("model_provider"), active_run.get("model"))?
    };
    let persist_requested_selection = req.model.is_some();
    let suggested_thread_name =
        suggested_thread_name(active_run.get::<String, _>("title").as_str(), &req.text);

    let result = adapter
        .send_user_message(
            &workspace,
            &thread_id,
            &req.text,
            &TurnOptions {
                model: selection.as_ref().map(|value| value.model_id.clone()),
                model_provider: selection.as_ref().map(|value| value.provider_id.clone()),
                effort: req.effort,
                service_tier: req.service_tier,
                access_mode: req.access_mode,
                images: req.images,
                collaboration_mode: req.collaboration_mode,
            },
        )
        .await
        .map_err(|error| {
            tracing::warn!(
                task_id = %task_id,
                thread_id = %thread_id,
                error = %error,
                "Codex Runtime rejected turn start"
            );
            (
                StatusCode::BAD_GATEWAY,
                Json(PlatformError::internal(
                    "Codex Runtime failed to start the Turn",
                )),
            )
        })?;

    let turn_id = result
        .get("turnId")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            (
                StatusCode::BAD_GATEWAY,
                Json(PlatformError::internal(
                    "Codex Runtime started a Turn without returning its id",
                )),
            )
        })?
        .to_string();
    if persist_requested_selection {
        let selection = selection
            .as_ref()
            .expect("validated request selection remains available");
        sqlx::query(
            "UPDATE tasks SET model_provider = $1, model = $2, updated_at = now() \
             WHERE id = $3 AND organization_id = $4",
        )
        .bind(&selection.provider_id)
        .bind(&selection.model_id)
        .bind(task_id)
        .bind(auth.organization_id)
        .execute(&state.db)
        .await
        .map_err(database_error)?;
        persist_default_model_selection(&state, auth.user_id, selection).await?;
    }
    if let Err(error) = sqlx::query(
        "UPDATE runs SET active_turn_id = $1, updated_at = now() \
         WHERE id = $2 AND organization_id = $3 AND status = 'running'",
    )
    .bind(&turn_id)
    .bind(active_run.get::<Uuid, _>("id"))
    .bind(auth.organization_id)
    .execute(&state.db)
    .await
    {
        tracing::warn!(%error, "active Turn delivery succeeded but projection update failed");
    }

    let status = result
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("sent")
        .to_string();
    let thread_name = if let Some(name) = suggested_thread_name {
        match sqlx::query(
            "UPDATE tasks SET title = $1, updated_at = now() \
             WHERE id = $2 AND organization_id = $3 AND title IN ('Thread', 'New Agent')",
        )
        .bind(&name)
        .bind(task_id)
        .bind(auth.organization_id)
        .execute(&state.db)
        .await
        {
            Ok(result) if result.rows_affected() > 0 => Some(name),
            Ok(_) => None,
            Err(error) => {
                tracing::warn!(%error, %task_id, "Turn started but Thread title projection failed");
                None
            }
        }
    } else {
        None
    };

    Ok(Json(SendMessageResponse {
        status,
        thread_id,
        turn_id,
        thread_name,
    }))
}

/// GET /api/tasks/:id
pub async fn get_task(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Task> {
    let row = sqlx::query(
        "SELECT id, project_id, title, status, model_provider, model, created_at, updated_at \
         FROM tasks WHERE id = $1 AND organization_id = $2",
    )
    .bind(id)
    .bind(auth.organization_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(PlatformError::internal(format!("{e}"))),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(PlatformError::not_found(format!("task {id} not found"))),
        )
    })?;

    Ok(Json(Task {
        id: row.get("id"),
        project_id: row.get("project_id"),
        title: row.get("title"),
        status: row.get("status"),
        model_provider: row.get("model_provider"),
        model: row.get("model"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }))
}

fn normalize_model_selection(
    provider_id: Option<String>,
    model_id: Option<String>,
) -> Result<Option<ModelSelection>, (StatusCode, Json<PlatformError>)> {
    match (provider_id, model_id) {
        (None, None) => Ok(None),
        (Some(provider_id), Some(model_id)) => {
            let provider_id = provider_id.trim();
            let model_id = model_id.trim();
            let invalid = provider_id.is_empty()
                || model_id.is_empty()
                || provider_id.len() > 200
                || model_id.len() > 300
                || provider_id
                    .chars()
                    .chain(model_id.chars())
                    .any(|character| matches!(character, '\0' | '\n' | '\r'));
            if invalid {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(PlatformError::bad_request("model selection is invalid")),
                ));
            }
            Ok(Some(ModelSelection {
                provider_id: provider_id.to_string(),
                model_id: model_id.to_string(),
            }))
        }
        _ => Err((
            StatusCode::BAD_REQUEST,
            Json(PlatformError::bad_request(
                "model and model_provider must be selected together",
            )),
        )),
    }
}

fn suggested_thread_name(current_name: &str, message: &str) -> Option<String> {
    if !matches!(current_name.trim(), "Thread" | "New Agent") {
        return None;
    }
    let normalized = message.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return None;
    }
    const MAX_CHARS: usize = 80;
    let mut characters = normalized.chars();
    let prefix = characters.by_ref().take(MAX_CHARS - 1).collect::<String>();
    if characters.next().is_some() {
        Some(format!("{prefix}…"))
    } else {
        Some(normalized)
    }
}

async fn load_default_model_selection(
    state: &AppState,
) -> Result<Option<ModelSelection>, (StatusCode, Json<PlatformError>)> {
    let stored = get_global(&state.db, MODEL_SELECTION_CONFIG_KEY)
        .await
        .map_err(database_error)?;
    Ok(stored.and_then(|stored| serde_json::from_value(stored.value).ok()))
}

async fn persist_default_model_selection(
    state: &AppState,
    user_id: Uuid,
    selection: &ModelSelection,
) -> Result<(), (StatusCode, Json<PlatformError>)> {
    put_global(
        &state.db,
        MODEL_SELECTION_CONFIG_KEY,
        serde_json::to_value(selection).map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(PlatformError::internal(
                    "model selection could not be encoded",
                )),
            )
        })?,
        user_id,
    )
    .await
    .map_err(database_error)?;
    Ok(())
}

fn database_error(_: sqlx::Error) -> (StatusCode, Json<PlatformError>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(PlatformError::internal("Database operation failed")),
    )
}

#[cfg(test)]
mod tests {
    use super::{normalize_model_selection, suggested_thread_name};

    #[test]
    fn model_selection_requires_a_provider_model_pair() {
        assert!(normalize_model_selection(None, None).unwrap().is_none());
        assert!(normalize_model_selection(
            Some("deepseek".to_string()),
            Some("deepseek-v4-flash".to_string()),
        )
        .unwrap()
        .is_some());
        assert!(normalize_model_selection(Some("deepseek".to_string()), None,).is_err());
        assert!(normalize_model_selection(None, Some("deepseek-v4-flash".to_string()),).is_err());
    }

    #[test]
    fn derives_a_readable_title_only_for_placeholder_threads() {
        assert_eq!(
            suggested_thread_name("Thread", "  查询上海\n  的地图  "),
            Some("查询上海 的地图".to_string()),
        );
        assert_eq!(
            suggested_thread_name("New Agent", "Show the route"),
            Some("Show the route".to_string()),
        );
        assert_eq!(suggested_thread_name("Custom title", "ignored"), None);
        assert_eq!(suggested_thread_name("Thread", " \n\t "), None);
    }

    #[test]
    fn truncates_long_titles_on_unicode_character_boundaries() {
        let title = suggested_thread_name("Thread", &"图".repeat(100)).unwrap();
        assert_eq!(title.chars().count(), 80);
        assert!(title.ends_with('…'));
    }
}
