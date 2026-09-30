use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, Response, StatusCode},
    Extension, Json,
};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use open_web_codex_adapter::{AuthorizedWorkspace, CodexAdapter};
use open_web_codex_platform_contracts::error::PlatformError;
use open_web_codex_platform_contracts::{
    SetThreadNameRequest, ThreadHistory, ThreadHistoryError, ThreadHistoryResponse,
    ThreadHistoryStatus, ThreadHistoryTurn,
};
use open_web_codex_platform_store::AppState;
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

use crate::middleware::auth::require_runtime_profile;
use crate::middleware::auth::AuthenticatedUser;
use crate::routes::RuntimeProfileBinding;

type ApiError = (StatusCode, Json<PlatformError>);
type ApiResult<T> = Result<Json<T>, ApiError>;
const MAX_INLINE_MAP_SOURCE_BYTES: usize = 32 * 1024 * 1024;

pub async fn read(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(run_id): Path<Uuid>,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
) -> ApiResult<ThreadHistoryResponse> {
    let context = authorized_thread(&state, &auth, run_id).await?;
    let value = adapter
        .read_thread(&context.workspace, &context.thread_id)
        .await
        .map_err(runtime_error)?;
    let thread = value
        .get("thread")
        .ok_or_else(|| bad_gateway("Runtime thread/read omitted thread"))?;
    Ok(Json(ThreadHistoryResponse {
        thread: project_thread(thread, &context.thread_id, &state, run_id).await?,
    }))
}

pub async fn read_inline_map_source(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path((run_id, card_ref, source_id)): Path<(Uuid, String, String)>,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
) -> ApiResult<Value> {
    let context = authorized_thread(&state, &auth, run_id).await?;
    let source = crate::inline_map_cards::source(
        &state.db,
        auth.organization_id,
        run_id,
        &card_ref,
        &source_id,
    )
    .await
    .map_err(database_error)?
    .ok_or_else(not_found)?;
    let response = adapter
        .read_mcp_resource(
            &context.workspace,
            context.copilot_package_id.as_deref(),
            &source.thread_id,
            &source.server,
            &source.uri,
        )
        .await
        .map_err(runtime_error)?;
    let bytes = mcp_resource_bytes(&response, &source.uri).map_err(bad_gateway)?;
    if bytes.len() > MAX_INLINE_MAP_SOURCE_BYTES {
        return Err(bad_gateway("Inline map source exceeds the size limit"));
    }
    let value = serde_json::from_slice::<Value>(&bytes)
        .map_err(|_| bad_gateway("Inline map source did not contain valid JSON"))?;
    if !valid_geojson_root(&value) {
        return Err(bad_gateway("Inline map source did not contain GeoJSON"));
    }
    Ok(Json(value))
}

pub async fn read_inline_visualization(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path((thread_id, file)): Path<(String, String)>,
    Extension(profile): Extension<RuntimeProfileBinding>,
) -> Result<Response<Body>, ApiError> {
    authorize_inline_visualization_thread(&state, &auth, &profile, &thread_id).await?;
    let codex_home = profile
        .codex_home
        .as_deref()
        .map(|path| path.as_path())
        .ok_or_else(|| bad_gateway("Inline visualization storage is not configured"))?;
    let payload = crate::inline_visualizations::read(codex_home, &thread_id, &file)
        .await
        .map_err(|_| not_found())?;
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, payload.content_type)
        .header(header::CACHE_CONTROL, "private, no-store")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .body(Body::from(payload.bytes))
        .map_err(|_| bad_gateway("Inline visualization response could not be created"))
}

async fn authorize_inline_visualization_thread(
    state: &AppState,
    auth: &AuthenticatedUser,
    profile: &RuntimeProfileBinding,
    thread_id: &str,
) -> Result<(), ApiError> {
    require_runtime_profile(&state.db, auth, &profile.runtime_key).await?;
    if thread_id.trim().is_empty() {
        return Err(not_found());
    }
    let row = sqlx::query(
        "SELECT run.requested_by, workspace.state \
         FROM runs run \
         JOIN tasks task ON task.id = run.task_id \
           AND task.organization_id = run.organization_id \
           AND task.workspace_id = run.workspace_id \
         JOIN workspaces workspace ON workspace.id = run.workspace_id \
           AND workspace.organization_id = run.organization_id \
         JOIN profiles profile ON profile.id = workspace.profile_id \
           AND profile.organization_id = run.organization_id \
           AND profile.runtime_key = $2 \
           AND profile.status = 'active' \
         JOIN workspace_grants workspace_grant ON workspace_grant.workspace_id = workspace.id \
           AND workspace_grant.organization_id = workspace.organization_id \
           AND workspace_grant.user_id = run.requested_by \
           AND workspace_grant.profile_id = workspace.profile_id \
         LEFT JOIN runtime_agent_projections agent \
           ON agent.root_run_id = run.id \
          AND agent.organization_id = run.organization_id \
           AND agent.thread_id = $3 \
         WHERE run.organization_id = $1 \
           AND (run.codex_thread_id = $3 OR agent.thread_id = $3) \
         ORDER BY run.updated_at DESC \
         LIMIT 1",
    )
    .bind(auth.organization_id)
    .bind(&profile.runtime_key)
    .bind(thread_id)
    .fetch_optional(&state.db)
    .await
    .map_err(database_error)?
    .ok_or_else(not_found)?;
    let requested_by: Option<Uuid> = row.get("requested_by");
    if !matches!(row.get::<String, _>("state").as_str(), "ready" | "retained")
        || (requested_by != Some(auth.user_id)
            && !matches!(auth.organization_role.as_str(), "owner" | "admin"))
    {
        return Err(not_found());
    }
    Ok(())
}

fn mcp_resource_bytes(response: &Value, expected_uri: &str) -> Result<Vec<u8>, &'static str> {
    let contents = response
        .get("contents")
        .and_then(Value::as_array)
        .ok_or("mcpServer/resource/read omitted contents")?;
    let content = contents
        .iter()
        .find(|content| content.get("uri").and_then(Value::as_str) == Some(expected_uri))
        .ok_or("MCP Resource response did not match the requested URI")?;
    if let Some(text) = content.get("text").and_then(Value::as_str) {
        return Ok(text.as_bytes().to_vec());
    }
    if let Some(blob) = content.get("blob").and_then(Value::as_str) {
        return BASE64
            .decode(blob)
            .map_err(|_| "MCP Resource blob was not valid base64");
    }
    Err("MCP Resource content was unsupported")
}

fn valid_geojson_root(value: &Value) -> bool {
    matches!(
        value.get("type").and_then(Value::as_str),
        Some(
            "FeatureCollection"
                | "Feature"
                | "GeometryCollection"
                | "Point"
                | "MultiPoint"
                | "LineString"
                | "MultiLineString"
                | "Polygon"
                | "MultiPolygon"
        )
    )
}

pub async fn list_turns(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(run_id): Path<Uuid>,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
) -> ApiResult<Vec<ThreadHistoryTurn>> {
    let context = authorized_thread(&state, &auth, run_id).await?;
    let turns = adapter
        .list_thread_turns(&context.workspace, &context.thread_id)
        .await
        .map_err(runtime_error)?;
    let overlay = load_history_overlay(&state, run_id).await?;
    let mut projected = Vec::with_capacity(turns.len());
    for turn in &turns {
        let mut turn = project_turn_with_refs(turn, &state, run_id).await?;
        overlay.apply(&mut turn);
        projected.push(turn);
    }
    Ok(Json(projected))
}

/// Return authoritative Codex history for one projected child Agent Thread.
///
/// The Run remains the authorization root. The platform projection proves that
/// the child belongs to that Run, while Codex remains the owner of Turn history.
pub async fn list_agent_turns(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path((run_id, agent_thread_id)): Path<(Uuid, String)>,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
) -> ApiResult<Vec<ThreadHistoryTurn>> {
    let context = authorized_agent_thread(&state, &auth, run_id, agent_thread_id.as_str()).await?;
    let turns = adapter
        .list_thread_turns(&context.workspace, &context.thread_id)
        .await
        .map_err(runtime_error)?;
    let overlay = load_history_overlay(&state, run_id).await?;
    let mut projected = Vec::with_capacity(turns.len());
    for turn in &turns {
        let mut turn = project_turn_with_refs(turn, &state, run_id).await?;
        overlay.apply(&mut turn);
        projected.push(turn);
    }
    Ok(Json(projected))
}

pub async fn archive(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(run_id): Path<Uuid>,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
) -> ApiResult<serde_json::Value> {
    let context = authorized_thread(&state, &auth, run_id).await?;
    let run_state = sqlx::query(
        "SELECT status, active_turn_id FROM runs WHERE id = $1 AND organization_id = $2",
    )
    .bind(run_id)
    .bind(auth.organization_id)
    .fetch_one(&state.db)
    .await
    .map_err(database_error)?;
    let recovery_without_active_turn = run_state.get::<String, _>("status") == "recovery_pending"
        && run_state
            .get::<Option<String>, _>("active_turn_id")
            .is_none();
    if !recovery_without_active_turn {
        adapter
            .archive_thread(&context.workspace, &context.thread_id)
            .await
            .map_err(runtime_error)?;
    }
    let mut transaction = state.db.begin().await.map_err(database_error)?;
    sqlx::query("UPDATE tasks SET status = 'archived', updated_at = now() WHERE id = $1")
        .bind(context.task_id)
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
    // Archiving the Runtime Thread also ends any platform Run that is still
    // schedulable. Otherwise the task disappears from the browser while its
    // stale `running`/`recovery_pending` Run still blocks Workspace removal.
    sqlx::query(
        "UPDATE runs SET status = 'cancelled', active_turn_id = NULL, failure_code = NULL, \
                         lease_owner = NULL, lease_token = NULL, lease_expires_at = NULL, \
                         updated_at = now() \
         WHERE id = $1 AND status IN ('pending', 'provisioning', 'running', 'cancelling', 'recovery_pending')",
    )
    .bind(run_id)
    .execute(&mut *transaction)
    .await
    .map_err(database_error)?;
    audit(
        &mut transaction,
        &auth,
        context.workspace_id,
        run_id,
        "thread.archive",
    )
    .await?;
    transaction.commit().await.map_err(database_error)?;
    Ok(Json(json!({ "status": "archived" })))
}

pub async fn set_name(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(run_id): Path<Uuid>,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
    Json(request): Json<SetThreadNameRequest>,
) -> ApiResult<serde_json::Value> {
    let name = request.name.trim();
    if name.is_empty()
        || name.len() > 200
        || name
            .chars()
            .any(|character| matches!(character, '\0' | '\n' | '\r'))
    {
        return Err(bad_request("Thread name is invalid"));
    }
    let context = authorized_thread(&state, &auth, run_id).await?;
    adapter
        .set_thread_name(&context.workspace, &context.thread_id, name)
        .await
        .map_err(runtime_error)?;
    let mut transaction = state.db.begin().await.map_err(database_error)?;
    sqlx::query("UPDATE tasks SET title = $1, updated_at = now() WHERE id = $2")
        .bind(name)
        .bind(context.task_id)
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
    audit(
        &mut transaction,
        &auth,
        context.workspace_id,
        run_id,
        "thread.name_set",
    )
    .await?;
    transaction.commit().await.map_err(database_error)?;
    Ok(Json(json!({ "status": "renamed", "name": name })))
}

struct ThreadContext {
    task_id: Uuid,
    workspace_id: Uuid,
    thread_id: String,
    copilot_package_id: Option<String>,
    workspace: AuthorizedWorkspace,
}

async fn authorized_thread(
    state: &AppState,
    auth: &AuthenticatedUser,
    run_id: Uuid,
) -> Result<ThreadContext, ApiError> {
    let row = sqlx::query(
        "SELECT run.task_id, run.workspace_id, run.codex_thread_id, run.requested_by, \
                task.copilot_package_id, \
                workspace.root_path, workspace.state \
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
         WHERE run.id = $1 AND run.organization_id = $2",
    )
    .bind(run_id)
    .bind(auth.organization_id)
    .fetch_optional(&state.db)
    .await
    .map_err(database_error)?
    .ok_or_else(not_found)?;
    let requested_by: Option<Uuid> = row.get("requested_by");
    if !matches!(row.get::<String, _>("state").as_str(), "ready" | "retained")
        || (requested_by != Some(auth.user_id)
            && !matches!(auth.organization_role.as_str(), "owner" | "admin"))
    {
        return Err(not_found());
    }
    let workspace_id: Uuid = row.get("workspace_id");
    let thread_id: Option<String> = row.get("codex_thread_id");
    let thread_id = thread_id.ok_or_else(|| bad_request("Run does not have a Codex Thread"))?;
    Ok(ThreadContext {
        task_id: row.get("task_id"),
        workspace_id,
        thread_id,
        copilot_package_id: row.get("copilot_package_id"),
        workspace: AuthorizedWorkspace {
            id: workspace_id.to_string(),
            root: row.get::<String, _>("root_path").into(),
        },
    })
}

async fn authorized_agent_thread(
    state: &AppState,
    auth: &AuthenticatedUser,
    run_id: Uuid,
    agent_thread_id: &str,
) -> Result<ThreadContext, ApiError> {
    if agent_thread_id.trim().is_empty() {
        return Err(not_found());
    }
    let row = sqlx::query(
        "SELECT run.task_id, run.workspace_id, run.requested_by, task.copilot_package_id,
                workspace.root_path, workspace.state, agent.thread_id
         FROM runs run
         JOIN tasks task ON task.id = run.task_id
           AND task.organization_id = run.organization_id
           AND task.workspace_id = run.workspace_id
         JOIN runtime_agent_projections agent
           ON agent.root_run_id = run.id
          AND agent.organization_id = run.organization_id
          AND agent.thread_id = $3
          AND agent.parent_thread_id IS NOT NULL
         JOIN workspaces workspace ON workspace.id = run.workspace_id
           AND workspace.organization_id = run.organization_id
         JOIN workspace_grants workspace_grant ON workspace_grant.workspace_id = workspace.id
           AND workspace_grant.organization_id = workspace.organization_id
           AND workspace_grant.user_id = run.requested_by
           AND workspace_grant.profile_id = workspace.profile_id
         WHERE run.id = $1 AND run.organization_id = $2",
    )
    .bind(run_id)
    .bind(auth.organization_id)
    .bind(agent_thread_id)
    .fetch_optional(&state.db)
    .await
    .map_err(database_error)?
    .ok_or_else(not_found)?;
    let requested_by: Option<Uuid> = row.get("requested_by");
    if !matches!(row.get::<String, _>("state").as_str(), "ready" | "retained")
        || (requested_by != Some(auth.user_id)
            && !matches!(auth.organization_role.as_str(), "owner" | "admin"))
    {
        return Err(not_found());
    }
    let workspace_id: Uuid = row.get("workspace_id");
    Ok(ThreadContext {
        task_id: row.get("task_id"),
        workspace_id,
        thread_id: row.get("thread_id"),
        copilot_package_id: row.get("copilot_package_id"),
        workspace: AuthorizedWorkspace {
            id: workspace_id.to_string(),
            root: row.get::<String, _>("root_path").into(),
        },
    })
}

async fn project_thread(
    value: &serde_json::Value,
    expected_id: &str,
    state: &AppState,
    run_id: Uuid,
) -> Result<ThreadHistory, ApiError> {
    let object = value
        .as_object()
        .ok_or_else(|| bad_gateway("Runtime Thread was invalid"))?;
    let id = object
        .get("id")
        .and_then(serde_json::Value::as_str)
        .filter(|id| *id == expected_id)
        .ok_or_else(|| bad_gateway("Runtime returned the wrong Thread"))?;
    let mut turns = Vec::new();
    if let Some(source_turns) = object.get("turns").and_then(serde_json::Value::as_array) {
        turns.reserve(source_turns.len());
        for turn in source_turns {
            turns.push(project_turn_with_refs(turn, state, run_id).await?);
        }
    }
    Ok(ThreadHistory {
        id: id.to_string(),
        name: object
            .get("name")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        preview: object
            .get("preview")
            .and_then(serde_json::Value::as_str)
            .and_then(|preview| crate::event_projection::bounded_runtime_text(preview, 500))
            .unwrap_or_default(),
        created_at: object
            .get("createdAt")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or_default(),
        updated_at: object
            .get("updatedAt")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or_default(),
        status: project_thread_status(object.get("status")),
        turns,
    })
}

fn project_thread_status(value: Option<&serde_json::Value>) -> ThreadHistoryStatus {
    let object = value.and_then(serde_json::Value::as_object);
    ThreadHistoryStatus {
        r#type: object
            .and_then(|value| value.get("type"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("notLoaded")
            .to_string(),
        active_flags: object
            .and_then(|value| value.get("activeFlags"))
            .and_then(serde_json::Value::as_array)
            .map(|flags| {
                flags
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn project_turn(value: &serde_json::Value) -> Result<ThreadHistoryTurn, ApiError> {
    let object = value
        .as_object()
        .ok_or_else(|| bad_gateway("Runtime Turn was invalid"))?;
    let id = object
        .get("id")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| bad_gateway("Runtime Turn omitted id"))?;
    let items = object
        .get("items")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let source = item.as_object()?;
                    let mut projected = crate::event_projection::project_item(source);
                    if let (Some(id), Some(target)) = (
                        source.get("id").and_then(serde_json::Value::as_str),
                        projected.as_object_mut(),
                    ) {
                        target.insert("id".to_string(), serde_json::Value::String(id.to_string()));
                    }
                    Some(projected)
                })
                .collect()
        })
        .unwrap_or_default();
    let error = object
        .get("error")
        .filter(|error| error.as_object().is_some())
        .map(|error| {
            let projected = crate::event_projection::project_public_runtime_error(error);
            ThreadHistoryError {
                message: projected
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("The Runtime reported an error.")
                    .to_string(),
                additional_details: None,
            }
        });
    Ok(ThreadHistoryTurn {
        id: id.to_string(),
        status: object
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("completed")
            .to_string(),
        items,
        error,
        started_at: object.get("startedAt").and_then(serde_json::Value::as_i64),
        completed_at: object
            .get("completedAt")
            .and_then(serde_json::Value::as_i64),
        duration_ms: object.get("durationMs").and_then(serde_json::Value::as_i64),
    })
}

async fn project_turn_with_refs(
    value: &serde_json::Value,
    state: &AppState,
    run_id: Uuid,
) -> Result<ThreadHistoryTurn, ApiError> {
    let mut turn = project_turn(value)?;
    let item_ids = turn
        .items
        .iter()
        .filter_map(|item| item.get("id").and_then(Value::as_str).map(str::to_string))
        .collect::<Vec<_>>();
    let artifacts_by_item = persisted_artifacts_by_item(state, run_id, &item_ids).await?;
    let agent_messages_by_item = persisted_agent_messages_by_item(state, run_id, &item_ids).await?;
    for item in &mut turn.items {
        if let Some(item_id) = item.get("id").and_then(Value::as_str) {
            if let Some(artifacts) = artifacts_by_item.get(item_id) {
                if let Some(object) = item.as_object_mut() {
                    object
                        .get_mut("result")
                        .and_then(Value::as_object_mut)
                        .and_then(|result| result.get_mut("structuredContent"))
                        .and_then(Value::as_object_mut)
                        .map(|structured| structured.remove("artifact"));
                    object.insert("artifacts".to_string(), Value::Array(artifacts.clone()));
                }
            }
        }
        if item.get("type").and_then(Value::as_str) != Some("agentMessage") {
            continue;
        }
        if let Some(text) = item
            .get("id")
            .and_then(Value::as_str)
            .and_then(|item_id| agent_messages_by_item.get(item_id))
        {
            item.as_object_mut()
                .expect("projected item must be an object")
                .insert("text".to_string(), Value::String(text.to_string()));
        }
        let Some(text) = item.get("text").and_then(Value::as_str) else {
            continue;
        };
        let cards = crate::inline_map_cards::resolve(&state.db, run_id, text)
            .await
            .map_err(database_error)?;
        if !cards.is_empty() {
            item.as_object_mut()
                .expect("projected item must be an object")
                .insert("inlineArtifacts".to_string(), Value::Array(cards));
        }
    }
    Ok(turn)
}

async fn persisted_agent_messages_by_item(
    state: &AppState,
    run_id: Uuid,
    item_ids: &[String],
) -> Result<HashMap<String, String>, ApiError> {
    if item_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query(
        "SELECT DISTINCT ON (item_id) item_id, payload #>> '{data,text}' AS text \
         FROM run_events \
         WHERE run_id = $1 \
           AND event_type = 'codex.item.completed' \
           AND item_id = ANY($2) \
           AND payload #>> '{itemType}' = 'agentMessage' \
         ORDER BY item_id, sequence DESC",
    )
    .bind(run_id)
    .bind(item_ids)
    .fetch_all(&state.db)
    .await
    .map_err(database_error)?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            Some((
                row.get::<Option<String>, _>("item_id")?,
                row.get::<Option<String>, _>("text")?,
            ))
        })
        .collect())
}

async fn persisted_artifacts_by_item(
    state: &AppState,
    run_id: Uuid,
    item_ids: &[String],
) -> Result<HashMap<String, Vec<Value>>, ApiError> {
    if item_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query(
        "SELECT provenance.producer_item_id, artifact.id, artifact.artifact_schema,
                artifact.display_name, artifact.mime_type, artifact.expected_size,
                artifact.byte_size, artifact.state, artifact.failure_code
         FROM artifact_provenance provenance
         JOIN artifacts artifact ON artifact.id = provenance.artifact_id
           AND artifact.organization_id = provenance.organization_id
         WHERE provenance.producer_run_id = $1
           AND provenance.producer_item_id = ANY($2)
         ORDER BY provenance.producer_item_id, artifact.created_at, artifact.id",
    )
    .bind(run_id)
    .bind(item_ids)
    .fetch_all(&state.db)
    .await
    .map_err(database_error)?;
    let mut by_item = HashMap::<String, Vec<Value>>::new();
    for row in rows {
        let artifact = crate::delivery_artifacts::artifact_delivery_projection(
            row.get("id"),
            &row.get::<String, _>("artifact_schema"),
            &row.get::<String, _>("display_name"),
            &row.get::<String, _>("mime_type"),
            row.get("expected_size"),
            row.get("byte_size"),
            &row.get::<String, _>("state"),
            row.get::<Option<String>, _>("failure_code").as_deref(),
        )
        .map_err(|_| bad_gateway("Artifact history projection was invalid"))?;
        by_item
            .entry(row.get("producer_item_id"))
            .or_default()
            .push(artifact);
    }
    Ok(by_item)
}

#[derive(Default)]
struct ThreadHistoryOverlay {
    item_sequences: HashMap<String, i64>,
    approvals_by_turn: HashMap<String, Vec<ApprovalOverlay>>,
}

struct ApprovalOverlay {
    sequence: i64,
    tool: Option<String>,
    item: serde_json::Value,
}

impl ThreadHistoryOverlay {
    fn apply(&self, turn: &mut ThreadHistoryTurn) {
        if let Some(approvals) = self.approvals_by_turn.get(&turn.id) {
            for approval in approvals {
                let insert_at = approval
                    .tool
                    .as_deref()
                    .and_then(|tool| {
                        turn.items
                            .iter()
                            .enumerate()
                            .filter(|(_, item)| {
                                item.get("type").and_then(serde_json::Value::as_str)
                                    == Some("mcpToolCall")
                                    && item.get("tool").and_then(serde_json::Value::as_str)
                                        == Some(tool)
                                    && item
                                        .get("id")
                                        .and_then(serde_json::Value::as_str)
                                        .and_then(|id| self.item_sequences.get(id))
                                        .is_some_and(|item_sequence| {
                                            *item_sequence <= approval.sequence
                                        })
                            })
                            .map(|(index, _)| index)
                            .last()
                            .map(|tool_index| {
                                let mut insert_at = tool_index + 1;
                                while turn.items.get(insert_at).is_some_and(|item| {
                                    item.get("type").and_then(serde_json::Value::as_str)
                                        == Some("platformApproval")
                                        && item
                                            .get("approvalTool")
                                            .and_then(serde_json::Value::as_str)
                                            == Some(tool)
                                }) {
                                    insert_at += 1;
                                }
                                insert_at
                            })
                    })
                    .unwrap_or_else(|| {
                        turn.items
                            .iter()
                            .position(|item| {
                                item.get("id")
                                    .and_then(serde_json::Value::as_str)
                                    .and_then(|id| self.item_sequences.get(id))
                                    .is_some_and(|item_sequence| *item_sequence > approval.sequence)
                            })
                            .unwrap_or(turn.items.len())
                    });
                turn.items.insert(insert_at, approval.item.clone());
            }
        }
    }
}

async fn load_history_overlay(
    state: &AppState,
    run_id: Uuid,
) -> Result<ThreadHistoryOverlay, ApiError> {
    let rows = sqlx::query(
        "SELECT events.sequence, events.event_type, events.turn_id, \
                events.item_id, events.payload, approvals.state AS approval_state \
         FROM run_events events \
         LEFT JOIN approvals ON approvals.run_id = events.run_id \
           AND approvals.id::text = events.payload #>> '{data,approvalId}' \
         WHERE events.run_id = $1 AND events.event_type IN ( \
           'codex.item.started', 'codex.item.completed', \
           'platform.approval.requested', 'platform.approval.resolved' \
         ) ORDER BY events.sequence",
    )
    .bind(run_id)
    .fetch_all(&state.db)
    .await
    .map_err(database_error)?;

    let resolved: HashSet<String> = rows
        .iter()
        .filter(|row| row.get::<String, _>("event_type") == "platform.approval.resolved")
        .filter_map(|row| {
            row.get::<serde_json::Value, _>("payload")
                .pointer("/data/requestId")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .collect();
    let mut overlay = ThreadHistoryOverlay::default();
    for row in rows {
        let sequence: i64 = row.get("sequence");
        let event_type: String = row.get("event_type");
        if matches!(
            event_type.as_str(),
            "codex.item.started" | "codex.item.completed"
        ) {
            if let Some(item_id) = row.get::<Option<String>, _>("item_id") {
                overlay.item_sequences.entry(item_id).or_insert(sequence);
            }
            continue;
        }
        if event_type != "platform.approval.requested" {
            continue;
        }
        let Some(turn_id) = row.get::<Option<String>, _>("turn_id") else {
            continue;
        };
        let payload: serde_json::Value = row.get("payload");
        if payload
            .pointer("/data/requestMethod")
            .and_then(serde_json::Value::as_str)
            == Some("item/tool/requestUserInput")
        {
            // User input is rendered from the run-scoped durable input queue.
            // Do not reinsert it as a generic approval item in the transcript.
            continue;
        }
        let Some(approval_id) = payload
            .pointer("/data/approvalId")
            .and_then(serde_json::Value::as_str)
        else {
            continue;
        };
        let request = payload
            .pointer("/data/requestParams")
            .and_then(serde_json::Value::as_object);
        let text = request
            .and_then(|request| request.get("message").or_else(|| request.get("reason")))
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("Approval requested");
        let requested_tool = requested_tool_name(request);
        let approval_status = row
            .get::<Option<String>, _>("approval_state")
            .as_deref()
            .and_then(public_approval_status)
            .unwrap_or_else(|| {
                if resolved.contains(approval_id) {
                    "resolved"
                } else {
                    "pending"
                }
            });
        let mut approval = serde_json::Map::from_iter([
            (
                "id".to_string(),
                serde_json::Value::String(format!("approval-{approval_id}")),
            ),
            (
                "type".to_string(),
                serde_json::Value::String("platformApproval".to_string()),
            ),
            (
                "text".to_string(),
                serde_json::Value::String(text.to_string()),
            ),
            (
                "approvalRequestId".to_string(),
                serde_json::Value::String(approval_id.to_string()),
            ),
            (
                "approvalStatus".to_string(),
                serde_json::Value::String(approval_status.to_string()),
            ),
        ]);
        if let Some(tool) = &requested_tool {
            approval.insert(
                "approvalTool".to_string(),
                serde_json::Value::String(tool.clone()),
            );
        }
        for (source, target) in [
            ("mode", "approvalMode"),
            ("credentialKind", "approvalCredentialKind"),
        ] {
            if let Some(value) = request.and_then(|request| request.get(source)) {
                approval.insert(target.to_string(), value.clone());
            }
        }
        overlay
            .approvals_by_turn
            .entry(turn_id)
            .or_default()
            .push(ApprovalOverlay {
                sequence,
                tool: requested_tool,
                item: serde_json::Value::Object(approval),
            });
    }
    for approvals in overlay.approvals_by_turn.values_mut() {
        approvals.sort_by_key(|approval| approval.sequence);
    }
    Ok(overlay)
}

fn public_approval_status(state: &str) -> Option<&'static str> {
    match state {
        "pending" | "dispatching" | "delivery_unknown" => Some("pending"),
        "approved" => Some("accepted"),
        "rejected" => Some("declined"),
        "answered" => Some("answered"),
        "cancelled" => Some("cancelled"),
        _ => None,
    }
}

fn requested_tool_name(
    request: Option<&serde_json::Map<String, serde_json::Value>>,
) -> Option<String> {
    let request = request?;
    for field in ["tool", "toolName"] {
        if let Some(value) = request.get(field).and_then(serde_json::Value::as_str) {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    let message = request.get("message").and_then(serde_json::Value::as_str)?;
    let marker = "tool \\\"";
    let start = message.find(marker)? + marker.len();
    let tool = message[start..].split('"').next()?.trim();
    (!tool.is_empty()).then(|| tool.to_string())
}

async fn audit(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    auth: &AuthenticatedUser,
    workspace_id: Uuid,
    run_id: Uuid,
    action: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO audit_log \
         (organization_id, actor_id, action, target_type, target_id, metadata, outcome) \
         VALUES ($1, $2, $3, 'workspace', $4, $5, 'success')",
    )
    .bind(auth.organization_id)
    .bind(auth.user_id)
    .bind(action)
    .bind(workspace_id)
    .bind(json!({ "runId": run_id }))
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    Ok(())
}

fn bad_request(message: &str) -> ApiError {
    (
        StatusCode::BAD_REQUEST,
        Json(PlatformError::bad_request(message)),
    )
}

fn bad_gateway(message: &str) -> ApiError {
    (
        StatusCode::BAD_GATEWAY,
        Json(PlatformError::internal(message)),
    )
}

fn not_found() -> ApiError {
    (
        StatusCode::NOT_FOUND,
        Json(PlatformError::not_found("Run Thread was not found")),
    )
}

fn runtime_error(error: open_web_codex_adapter::AdapterError) -> ApiError {
    tracing::warn!(%error, "Codex Runtime Thread operation failed");
    if error.is_thread_resume_rejected() {
        return (
            StatusCode::CONFLICT,
            Json(PlatformError::conflict(
                "Runtime rejected recovery of this Thread; start a new Run or inspect server logs",
            )),
        );
    }
    (
        StatusCode::BAD_GATEWAY,
        Json(PlatformError::internal("Codex Thread operation failed")),
    )
}

fn database_error(_: sqlx::Error) -> ApiError {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(PlatformError::internal("Database operation failed")),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        mcp_resource_bytes, project_turn, public_approval_status, valid_geojson_root,
        ApprovalOverlay, ThreadHistoryOverlay,
    };
    use open_web_codex_platform_contracts::ThreadHistoryTurn;
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn projects_authoritative_history_without_runtime_only_fields() {
        let projected = project_turn(&json!({
            "id": "turn-1",
            "status": "completed",
            "items": [{
                "id": "item-1",
                "type": "agentMessage",
                "text": "done",
                "phase": "final_answer",
                "cwd": "/private/server/workspace",
                "apiKey": "secret"
            }]
        }))
        .expect("valid Turn projection");
        let value = serde_json::to_value(projected).expect("serializable projection");

        assert_eq!(value["items"][0]["id"], "item-1");
        assert_eq!(value["items"][0]["text"], "done");
        assert_eq!(value["items"][0]["phase"], "final_answer");
        let encoded = value.to_string();
        assert!(!encoded.contains("/private/server/workspace"));
        assert!(!encoded.contains("secret"));
    }

    #[test]
    fn projects_root_and_child_history_agent_messages_with_one_safe_contract() {
        for (turn_id, phase) in [("root-turn", "commentary"), ("child-turn", "final_answer")] {
            let projected = project_turn(&json!({
                "id": turn_id,
                "status": "completed",
                "items": [{
                    "id": format!("item-{turn_id}"),
                    "type": "agentMessage",
                    "phase": phase,
                    "text": "[safe](normalized/file.csv) [web](https://example.com/report.csv) [unsafe](/Users/example/secret.csv) [entity](&#47;Users/example/entity.csv)",
                }]
            }))
            .expect("history turn projects");
            let item = &projected.items[0];
            let text = item["text"].as_str().expect("assistant text");
            assert!(text.contains("[safe](normalized/file.csv)"));
            assert!(text.contains("[web](https://example.com/report.csv)"));
            assert!(text.contains("unsafe"));
            assert!(text.contains("entity"));
            assert!(!text.contains("/Users/example"));
            assert!(!text.contains("[workspace-path]"));
            assert!(!text.contains("[internal-resource-uri]"));
            assert!(!text.contains("[unsafe]("));
            assert!(!text.contains("[entity]("));
            assert!(!text.contains("&#47;"));
        }
    }

    #[test]
    fn projects_markdown_destinations_and_natural_slashes_in_authoritative_history() {
        let projected = project_turn(
            &json!({
                "id": "turn-angle",
                "status": "completed",
                "items": [{
                    "id": "item-angle",
                    "type": "agentMessage",
                    "phase": "final_answer",
                    "text": "[angle](</Users/example/runner/workspaces/workspace-1/normalized/file.csv>) [relative](normalized/file.csv) [http](http://example.com/file.csv) [https](https://example.com/file.csv) 城市数/需求占比 1500/公里/需求单位 中间节点/上游关系 仓ID/名称/类型 `center`/`cross_docking` 字段映射/标准化"
                }]
            }),
        )
        .expect("history turn projects");
        let text = projected.items[0]["text"].as_str().expect("assistant text");
        assert!(!text.contains("[angle]("));
        assert!(!text.contains("/Users/example/runner/workspaces"));
        assert!(text.contains("[relative](normalized/file.csv)"));
        assert!(text.contains("[http](http://example.com/file.csv)"));
        assert!(text.contains("[https](https://example.com/file.csv)"));
        for phrase in [
            "城市数/需求占比",
            "1500/公里/需求单位",
            "中间节点/上游关系",
            "仓ID/名称/类型",
            "`center`/`cross_docking`",
            "字段映射/标准化",
        ] {
            assert!(text.contains(phrase), "missing preserved phrase: {phrase}");
        }
    }

    #[test]
    fn preserves_mcp_tool_error_in_authoritative_history() {
        let projected = project_turn(&json!({
            "id": "turn-1",
            "status": "failed",
            "items": [{
                "id": "item-1",
                "type": "mcpToolCall",
                "error": {
                    "message": "MCP failed at /private/profile/secret.json",
                    "credential": "<credential-fragment>"
                }
            }]
        }))
        .expect("valid failed Turn projection");
        let value = serde_json::to_value(projected).expect("serializable projection");
        assert_eq!(
            value["items"][0]["error"]["message"],
            "MCP failed at [workspace-path]/secret.json"
        );
        assert_eq!(value["items"][0]["error"]["credential"], "[redacted]");
        assert!(value["items"][0]["error"].get("code").is_none());
        assert!(!value.to_string().contains("<credential-fragment>"));
    }

    #[test]
    fn rejects_turns_without_stable_identity() {
        assert!(project_turn(&json!({ "status": "completed", "items": [] })).is_err());
    }

    #[test]
    fn projects_turn_errors_from_structured_status_without_provider_text() {
        let projected = project_turn(&json!({
            "id": "turn-1",
            "status": "failed",
            "error": {
                "message": "provider response <credential-fragment>",
                "additionalDetails": "https://provider.invalid/<credential-fragment>",
                "codexErrorInfo": "unauthorized"
            }
        }))
        .expect("valid failed Turn projection");
        let value = serde_json::to_value(projected).expect("serializable projection");

        assert_eq!(value["error"]["message"], "Provider authentication failed.");
        assert_eq!(value["error"]["additionalDetails"], serde_json::Value::Null);
        assert!(!value.to_string().contains("credential-fragment"));
        assert!(!value.to_string().contains("provider.invalid"));
    }

    #[test]
    fn projects_provider_function_tools_history_error_without_provider_text() {
        let projected = project_turn(&json!({
            "id": "turn-1",
            "status": "failed",
            "error": {
                "message": "provider response <credential-fragment>",
                "additionalDetails": "https://provider.invalid/<credential-fragment>",
                "codexErrorInfo": "providerFunctionToolsUnsupported"
            }
        }))
        .expect("valid failed Turn projection");
        let value = serde_json::to_value(projected).expect("serializable projection");

        assert_eq!(
            value["error"]["message"],
            "Function tools are disabled for this Provider. Enable Function tools in Provider settings."
        );
        assert_eq!(value["error"]["additionalDetails"], serde_json::Value::Null);
        assert!(!value.to_string().contains("credential-fragment"));
        assert!(!value.to_string().contains("provider.invalid"));
    }

    #[test]
    fn omits_explicit_null_turn_errors() {
        let projected = project_turn(&json!({
            "id": "turn-1",
            "status": "completed",
            "error": null
        }))
        .expect("valid completed Turn projection");
        assert!(projected.error.is_none());
    }

    #[test]
    fn reads_only_the_exact_requested_geojson_resource() {
        let uri = "supply-chain://resources/distribution";
        let bytes = mcp_resource_bytes(
            &json!({"contents": [
                {"uri": "supply-chain://resources/other", "text": "{}"},
                {"uri": uri, "mimeType": "application/geo+json", "text": "{\"type\":\"FeatureCollection\",\"features\":[]}"}
            ]}),
            uri,
        )
        .expect("exact Resource");
        let value: serde_json::Value = serde_json::from_slice(&bytes).expect("valid JSON");
        assert!(valid_geojson_root(&value));
        assert!(mcp_resource_bytes(&json!({"contents": []}), uri).is_err());
        assert!(!valid_geojson_root(&json!({"type": "Table"})));
    }

    #[test]
    fn maps_durable_approval_states_to_browser_outcomes() {
        assert_eq!(public_approval_status("approved"), Some("accepted"));
        assert_eq!(public_approval_status("rejected"), Some("declined"));
        assert_eq!(public_approval_status("answered"), Some("answered"));
        assert_eq!(public_approval_status("cancelled"), Some("cancelled"));
        assert_eq!(public_approval_status("dispatching"), Some("pending"));
    }

    #[test]
    fn inserts_platform_approvals_at_their_runtime_sequence() {
        let mut turn = ThreadHistoryTurn {
            id: "turn-1".to_string(),
            status: "completed".to_string(),
            items: vec![
                json!({ "id": "user", "type": "userMessage" }),
                json!({ "id": "tool-1", "type": "mcpToolCall", "tool": "batch_geocode" }),
                json!({ "id": "message", "type": "agentMessage" }),
                json!({ "id": "tool-2", "type": "mcpToolCall", "tool": "distance_matrix" }),
            ],
            error: None,
            started_at: None,
            completed_at: None,
            duration_ms: None,
        };
        let overlay = ThreadHistoryOverlay {
            item_sequences: HashMap::from([
                ("user".to_string(), 10),
                ("tool-1".to_string(), 20),
                ("message".to_string(), 40),
                ("tool-2".to_string(), 50),
            ]),
            approvals_by_turn: HashMap::from([(
                "turn-1".to_string(),
                vec![
                    ApprovalOverlay {
                        sequence: 30,
                        tool: Some("batch_geocode".to_string()),
                        item: json!({ "id": "approval-1", "type": "platformApproval", "approvalTool": "batch_geocode" }),
                    },
                    ApprovalOverlay {
                        sequence: 60,
                        tool: Some("distance_matrix".to_string()),
                        item: json!({ "id": "approval-2", "type": "platformApproval", "approvalTool": "distance_matrix" }),
                    },
                ],
            )]),
        };

        overlay.apply(&mut turn);

        let ids: Vec<_> = turn
            .items
            .iter()
            .filter_map(|item| item.get("id").and_then(serde_json::Value::as_str))
            .collect();
        assert_eq!(
            ids,
            [
                "user",
                "tool-1",
                "approval-1",
                "message",
                "tool-2",
                "approval-2"
            ]
        );
    }
}
