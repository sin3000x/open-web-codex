use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use open_web_codex_adapter::{AuthorizedWorkspace, CodexAdapter};
use open_web_codex_platform_contracts::error::PlatformError;
use open_web_codex_platform_contracts::{
    SetThreadNameRequest, ThreadHistory, ThreadHistoryError, ThreadHistoryResponse,
    ThreadHistoryStatus, ThreadHistoryTurn,
};
use open_web_codex_platform_store::AppState;
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

use crate::middleware::auth::AuthenticatedUser;

type ApiError = (StatusCode, Json<PlatformError>);
type ApiResult<T> = Result<Json<T>, ApiError>;

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

pub async fn archive(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(run_id): Path<Uuid>,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
) -> ApiResult<serde_json::Value> {
    let context = authorized_thread(&state, &auth, run_id).await?;
    adapter
        .archive_thread(&context.workspace, &context.thread_id)
        .await
        .map_err(runtime_error)?;
    let mut transaction = state.db.begin().await.map_err(database_error)?;
    sqlx::query("UPDATE tasks SET status = 'archived', updated_at = now() WHERE id = $1")
        .bind(context.task_id)
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
    workspace: AuthorizedWorkspace,
}

async fn authorized_thread(
    state: &AppState,
    auth: &AuthenticatedUser,
    run_id: Uuid,
) -> Result<ThreadContext, ApiError> {
    let row = sqlx::query(
        "SELECT r.task_id, r.workspace_id, r.codex_thread_id, r.requested_by, \
                w.root_path, w.state \
         FROM runs r JOIN workspaces w ON w.id = r.workspace_id \
         WHERE r.id = $1 AND r.organization_id = $2 AND w.organization_id = $2",
    )
    .bind(run_id)
    .bind(auth.organization_id)
    .fetch_optional(&state.db)
    .await
    .map_err(database_error)?
    .ok_or_else(not_found)?;
    let requested_by: Option<Uuid> = row.get("requested_by");
    if row.get::<String, _>("state") == "retired"
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
            .unwrap_or_default()
            .to_string(),
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
        .and_then(serde_json::Value::as_object)
        .map(|error| ThreadHistoryError {
            message: error
                .get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("Turn failed")
                .to_string(),
            additional_details: error
                .get("additionalDetails")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
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
    for item in &mut turn.items {
        if item.get("type").and_then(serde_json::Value::as_str) != Some("agentMessage") {
            continue;
        }
        let Some(text) = item
            .get("text")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
        else {
            continue;
        };
        let artifacts = crate::event_projection::resolve_inline_artifacts(&state.db, run_id, &text)
            .await
            .map_err(database_error)?;
        if !artifacts.is_empty() {
            item.as_object_mut()
                .expect("projected item must be an object")
                .insert(
                    "inlineArtifacts".to_string(),
                    serde_json::Value::Array(artifacts),
                );
        }
    }
    Ok(turn)
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
            ("serverName", "approvalServerName"),
        ] {
            if let Some(value) = request.and_then(|request| request.get(source)) {
                approval.insert(target.to_string(), value.clone());
            }
        }
        if approval_status == "pending" {
            if let Some(value) = request.and_then(|request| request.get("url")) {
                approval.insert("approvalUrl".to_string(), value.clone());
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

fn runtime_error(_: open_web_codex_adapter::AdapterError) -> ApiError {
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
    use super::{project_turn, public_approval_status, ApprovalOverlay, ThreadHistoryOverlay};
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
    fn rejects_turns_without_stable_identity() {
        assert!(project_turn(&json!({ "status": "completed", "items": [] })).is_err());
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
