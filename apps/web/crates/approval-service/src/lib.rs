//! Durable, Profile-scoped approval orchestration.
//!
//! Runtime request ids and complete app-server payloads remain server-side.
//! Browser DTOs expose only the platform approval id, optimistic version and a
//! small authorized projection needed to make a decision.

use open_web_codex_platform_contracts::{
    ApprovalDecision, ApprovalSummary, DecideApprovalRequest, RespondUserInputRequest,
};
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use thiserror::Error;
use uuid::Uuid;

const COMMAND_APPROVAL: &str = "item/commandExecution/requestApproval";
const FILE_APPROVAL: &str = "item/fileChange/requestApproval";
const MCP_ELICITATION_REQUEST: &str = "mcpServer/elicitation/request";
const PERMISSIONS_APPROVAL: &str = "item/permissions/requestApproval";
const USER_INPUT_REQUEST: &str = "item/tool/requestUserInput";

#[derive(Clone, Copy, Debug)]
pub struct ApprovalActor {
    pub user_id: Uuid,
    pub organization_id: Uuid,
}

#[derive(Debug)]
pub struct ApprovalDispatch {
    pub approval_id: Uuid,
    pub runtime_instance_id: Uuid,
    pub runtime_request_id: Value,
    pub response: Value,
    pub dispatch_version: i64,
    terminal_state: &'static str,
    decision: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedApproval {
    pub approval_id: Uuid,
    pub thread_id: String,
    pub turn_id: Option<String>,
    pub item_id: Option<String>,
    pub outcome: ApprovalOutcome,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApprovalOutcome {
    Accepted,
    Declined,
    Answered,
    Cancelled,
}

impl ApprovalOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Declined => "declined",
            Self::Answered => "answered",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Error)]
pub enum ApprovalServiceError {
    #[error("Approval was not found")]
    NotFound,
    #[error("Approval was already decided or its version changed")]
    Conflict,
    #[error("Approval request is invalid")]
    Invalid,
    #[error("Approval database operation failed: {0}")]
    Database(#[from] sqlx::Error),
}

#[derive(Clone)]
pub struct ApprovalService {
    db: PgPool,
    runtime_key: String,
}

impl ApprovalService {
    pub fn new(db: PgPool, runtime_key: impl Into<String>) -> Self {
        Self {
            db,
            runtime_key: runtime_key.into(),
        }
    }

    /// Persist an approval server request before its projection is broadcast.
    pub async fn capture_event_frame(
        &self,
        frame: &[u8],
    ) -> Result<Option<Uuid>, ApprovalServiceError> {
        let payload = frame
            .strip_prefix(b"data: ")
            .and_then(|value| value.strip_suffix(b"\n\n"))
            .ok_or(ApprovalServiceError::Invalid)?;
        let envelope: Value =
            serde_json::from_slice(payload).map_err(|_| ApprovalServiceError::Invalid)?;
        let runtime_instance_id = envelope
            .pointer("/params/runtime_instance_id")
            .and_then(Value::as_str)
            .and_then(|value| Uuid::parse_str(value).ok())
            .ok_or(ApprovalServiceError::Invalid)?;
        let message = envelope
            .pointer("/params/message")
            .ok_or(ApprovalServiceError::Invalid)?;
        self.capture_message(runtime_instance_id, message).await
    }

    pub async fn capture_message(
        &self,
        runtime_instance_id: Uuid,
        message: &Value,
    ) -> Result<Option<Uuid>, ApprovalServiceError> {
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            return Ok(None);
        };
        if !matches!(
            method,
            COMMAND_APPROVAL
                | FILE_APPROVAL
                | MCP_ELICITATION_REQUEST
                | PERMISSIONS_APPROVAL
                | USER_INPUT_REQUEST
        ) {
            return Ok(None);
        }
        let request_id = message.get("id").ok_or(ApprovalServiceError::Invalid)?;
        if !(request_id.is_u64() || request_id.is_i64() || request_id.is_string()) {
            return Err(ApprovalServiceError::Invalid);
        }
        let params = message
            .get("params")
            .cloned()
            .ok_or(ApprovalServiceError::Invalid)?;
        let thread_id = params
            .get("threadId")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or(ApprovalServiceError::Invalid)?
            .to_string();
        let runtime_request_id =
            serde_json::to_string(request_id).map_err(|_| ApprovalServiceError::Invalid)?;
        let stored_payload = approval_payload(method, &params);

        let row = sqlx::query(
            "INSERT INTO approvals \
             (run_id, request_type, request_payload, organization_id, profile_id, thread_id, \
              runtime_instance_id, runtime_request_id, state, version) \
             SELECT r.id, $1, $2, p.organization_id, p.id, $3, $4, $5, 'pending', 0 \
             FROM profiles p JOIN runs r \
               ON r.organization_id = p.organization_id AND r.codex_thread_id = $3 \
             WHERE p.runtime_key = $6 AND p.status = 'active' \
             ORDER BY r.created_at DESC LIMIT 1 \
             ON CONFLICT (profile_id, runtime_instance_id, runtime_request_id) \
             WHERE runtime_instance_id IS NOT NULL AND runtime_request_id IS NOT NULL \
             DO NOTHING RETURNING id",
        )
        .bind(method)
        .bind(stored_payload)
        .bind(&thread_id)
        .bind(runtime_instance_id)
        .bind(&runtime_request_id)
        .bind(&self.runtime_key)
        .fetch_optional(&self.db)
        .await?;
        if let Some(row) = row {
            return Ok(Some(row.get("id")));
        }
        let existing = sqlx::query(
            "SELECT a.id FROM approvals a JOIN profiles p ON p.id = a.profile_id \
             WHERE p.runtime_key = $1 AND a.runtime_instance_id = $2 \
               AND a.runtime_request_id = $3",
        )
        .bind(&self.runtime_key)
        .bind(runtime_instance_id)
        .bind(runtime_request_id)
        .fetch_optional(&self.db)
        .await?;
        existing
            .map(|row| Some(row.get("id")))
            .ok_or(ApprovalServiceError::NotFound)
    }

    pub async fn list_pending(
        &self,
        actor: ApprovalActor,
        runtime_instance_id: Uuid,
    ) -> Result<Vec<ApprovalSummary>, ApprovalServiceError> {
        self.cancel_stale_runtime_requests(runtime_instance_id)
            .await?;
        let rows = sqlx::query(
            "SELECT a.id, a.run_id, a.thread_id, a.request_type, a.request_payload, \
                    a.state, a.version, a.created_at, a.decided_at \
             FROM approvals a JOIN profiles p ON p.id = a.profile_id \
             WHERE a.organization_id = $1 AND p.owner_user_id = $2 \
               AND p.runtime_key = $3 AND a.state IN ('pending', 'dispatching', 'delivery_unknown') \
               AND a.runtime_instance_id = $4 \
             ORDER BY a.created_at, a.id",
        )
        .bind(actor.organization_id)
        .bind(actor.user_id)
        .bind(&self.runtime_key)
        .bind(runtime_instance_id)
        .fetch_all(&self.db)
        .await?;
        Ok(rows.iter().map(summary_from_row).collect())
    }

    /// Resolve a Runtime request id to its browser-safe platform identity.
    ///
    /// Runtime resolution can race the HTTP response path or can happen
    /// without a browser decision (for example after interruption or Runtime
    /// auto-resolution). In either case, no active approval may remain visible
    /// after the Runtime says the request is resolved. Existing decisions are
    /// retained and determine the terminal state; an undecided request becomes
    /// cancelled.
    pub async fn resolve_runtime_request(
        &self,
        runtime_instance_id: Uuid,
        thread_id: &str,
        runtime_request_id: &Value,
    ) -> Result<Option<ResolvedApproval>, ApprovalServiceError> {
        if thread_id.trim().is_empty() {
            return Err(ApprovalServiceError::Invalid);
        }
        if !(runtime_request_id.is_u64()
            || runtime_request_id.is_i64()
            || runtime_request_id.is_string())
        {
            return Err(ApprovalServiceError::Invalid);
        }
        let runtime_request_id =
            serde_json::to_string(runtime_request_id).map_err(|_| ApprovalServiceError::Invalid)?;
        let mut transaction = self.db.begin().await?;
        let row = sqlx::query(
            "SELECT a.id, a.thread_id, a.request_payload, a.state, a.decision \
             FROM approvals a JOIN profiles p ON p.id = a.profile_id \
             WHERE p.runtime_key = $1 AND a.runtime_instance_id = $2 \
               AND a.thread_id = $3 AND a.runtime_request_id = $4 \
             FOR UPDATE OF a",
        )
        .bind(&self.runtime_key)
        .bind(runtime_instance_id)
        .bind(thread_id)
        .bind(&runtime_request_id)
        .fetch_optional(&mut *transaction)
        .await?;
        let Some(row) = row else {
            transaction.commit().await?;
            return Ok(None);
        };

        let approval_id: Uuid = row.get("id");
        let thread_id: Option<String> = row.get("thread_id");
        let thread_id = thread_id
            .filter(|value| !value.is_empty())
            .ok_or(ApprovalServiceError::Invalid)?;
        let request_payload: Value = row.get("request_payload");
        let turn_id = request_payload
            .get("turnId")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let item_id = request_payload
            .get("itemId")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let state: String = row.get("state");
        let decision: Option<String> = row.get("decision");
        let resolved_state =
            if let Some(terminal_state) = resolved_terminal_state(&state, decision.as_deref()) {
                sqlx::query(
                    "UPDATE approvals SET state = $1, decided_at = COALESCE(decided_at, now()), \
                 version = version + 1 WHERE id = $2",
                )
                .bind(terminal_state)
                .bind(approval_id)
                .execute(&mut *transaction)
                .await?;
                terminal_state
            } else {
                state.as_str()
            };
        let outcome = approval_outcome(resolved_state).ok_or(ApprovalServiceError::Invalid)?;
        transaction.commit().await?;

        Ok(Some(ResolvedApproval {
            approval_id,
            thread_id,
            turn_id,
            item_id,
            outcome,
        }))
    }

    pub async fn begin_decision(
        &self,
        actor: ApprovalActor,
        approval_id: Uuid,
        runtime_instance_id: Uuid,
        request: DecideApprovalRequest,
    ) -> Result<ApprovalDispatch, ApprovalServiceError> {
        let mut transaction = self.db.begin().await?;
        let row = sqlx::query(
            "SELECT a.runtime_instance_id, a.runtime_request_id, a.request_type, \
                    a.request_payload, a.state, a.version, a.decision \
             FROM approvals a JOIN profiles p ON p.id = a.profile_id \
             WHERE a.id = $1 AND a.organization_id = $2 AND p.owner_user_id = $3 \
               AND p.runtime_key = $4 FOR UPDATE",
        )
        .bind(approval_id)
        .bind(actor.organization_id)
        .bind(actor.user_id)
        .bind(&self.runtime_key)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(ApprovalServiceError::NotFound)?;
        let state: String = row.get("state");
        let version: i64 = row.get("version");
        let stored_runtime_instance_id: Option<Uuid> = row.get("runtime_instance_id");
        if stored_runtime_instance_id != Some(runtime_instance_id) || version != request.version {
            return Err(ApprovalServiceError::Conflict);
        }
        let request_type: String = row.get("request_type");
        let payload: Value = row.get("request_payload");
        let (response, terminal_state, decision) =
            approval_response(&request_type, &payload, request.decision)?;
        let previous_decision: Option<String> = row.get("decision");
        if state != "pending"
            && !(state == "delivery_unknown" && previous_decision.as_deref() == Some(decision))
        {
            return Err(ApprovalServiceError::Conflict);
        }
        let dispatch_version = version + 1;
        sqlx::query(
            "UPDATE approvals SET state = 'dispatching', decision = $1, decided_by = $2, \
                    decided_at = now(), version = $3 WHERE id = $4",
        )
        .bind(decision)
        .bind(actor.user_id)
        .bind(dispatch_version)
        .bind(approval_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;

        let runtime_request_id: String = row.get("runtime_request_id");
        Ok(ApprovalDispatch {
            approval_id,
            runtime_instance_id,
            runtime_request_id: serde_json::from_str(&runtime_request_id)
                .map_err(|_| ApprovalServiceError::Invalid)?,
            response,
            dispatch_version,
            terminal_state,
            decision,
        })
    }

    pub async fn begin_user_input_response(
        &self,
        actor: ApprovalActor,
        approval_id: Uuid,
        runtime_instance_id: Uuid,
        request: RespondUserInputRequest,
    ) -> Result<ApprovalDispatch, ApprovalServiceError> {
        let mut transaction = self.db.begin().await?;
        let row = sqlx::query(
            "SELECT a.runtime_instance_id, a.runtime_request_id, a.request_type, \
                    a.request_payload, a.state, a.version, a.decision \
             FROM approvals a JOIN profiles p ON p.id = a.profile_id \
             WHERE a.id = $1 AND a.organization_id = $2 AND p.owner_user_id = $3 \
               AND p.runtime_key = $4 FOR UPDATE",
        )
        .bind(approval_id)
        .bind(actor.organization_id)
        .bind(actor.user_id)
        .bind(&self.runtime_key)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(ApprovalServiceError::NotFound)?;
        let state: String = row.get("state");
        let version: i64 = row.get("version");
        let request_type: String = row.get("request_type");
        let payload: Value = row.get("request_payload");
        let stored_runtime_instance_id: Option<Uuid> = row.get("runtime_instance_id");
        let previous_decision: Option<String> = row.get("decision");
        if stored_runtime_instance_id != Some(runtime_instance_id)
            || version != request.version
            || (state != "pending"
                && !(state == "delivery_unknown"
                    && previous_decision.as_deref() == Some("answered")))
        {
            return Err(ApprovalServiceError::Conflict);
        }
        if request_type != USER_INPUT_REQUEST {
            return Err(ApprovalServiceError::Invalid);
        }
        validate_user_input_answers(&payload, &request)?;
        let dispatch_version = version + 1;
        sqlx::query(
            "UPDATE approvals SET state = 'dispatching', decision = 'answered', decided_by = $1, \
                    decided_at = now(), version = $2 WHERE id = $3",
        )
        .bind(actor.user_id)
        .bind(dispatch_version)
        .bind(approval_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        let runtime_request_id: String = row.get("runtime_request_id");
        Ok(ApprovalDispatch {
            approval_id,
            runtime_instance_id,
            runtime_request_id: serde_json::from_str(&runtime_request_id)
                .map_err(|_| ApprovalServiceError::Invalid)?,
            response: json!({ "answers": request.answers }),
            dispatch_version,
            terminal_state: "answered",
            decision: "answered",
        })
    }

    pub async fn complete_decision(
        &self,
        actor: ApprovalActor,
        dispatch: &ApprovalDispatch,
    ) -> Result<(), ApprovalServiceError> {
        let mut transaction = self.db.begin().await?;
        let updated = sqlx::query(
            "UPDATE approvals SET state = $1, version = version + 1 \
             WHERE id = $2 AND organization_id = $3 AND state = 'dispatching' AND version = $4",
        )
        .bind(dispatch.terminal_state)
        .bind(dispatch.approval_id)
        .bind(actor.organization_id)
        .bind(dispatch.dispatch_version)
        .execute(&mut *transaction)
        .await?;
        if updated.rows_affected() != 1 {
            let already_completed: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM approvals WHERE id = $1 \
                 AND organization_id = $2 AND state = $3 AND decision = $4)",
            )
            .bind(dispatch.approval_id)
            .bind(actor.organization_id)
            .bind(dispatch.terminal_state)
            .bind(dispatch.decision)
            .fetch_one(&mut *transaction)
            .await?;
            if !already_completed {
                return Err(ApprovalServiceError::Conflict);
            }
        }
        insert_audit(&mut transaction, actor, dispatch, "success").await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn mark_delivery_unknown(
        &self,
        actor: ApprovalActor,
        dispatch: &ApprovalDispatch,
    ) -> Result<(), ApprovalServiceError> {
        let mut transaction = self.db.begin().await?;
        sqlx::query(
            "UPDATE approvals SET state = 'delivery_unknown', version = version + 1 \
             WHERE id = $1 AND organization_id = $2 AND state = 'dispatching' AND version = $3",
        )
        .bind(dispatch.approval_id)
        .bind(actor.organization_id)
        .bind(dispatch.dispatch_version)
        .execute(&mut *transaction)
        .await?;
        insert_audit(&mut transaction, actor, dispatch, "delivery_unknown").await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn cancel_stale_runtime_requests(
        &self,
        runtime_instance_id: Uuid,
    ) -> Result<u64, ApprovalServiceError> {
        let result = sqlx::query(
            "UPDATE approvals a SET state = 'cancelled', \
                    decided_at = COALESCE(a.decided_at, now()), version = a.version + 1 \
             FROM profiles p WHERE p.id = a.profile_id AND p.runtime_key = $1 \
               AND a.state IN ('pending', 'dispatching', 'delivery_unknown') \
               AND a.runtime_instance_id IS DISTINCT FROM $2",
        )
        .bind(&self.runtime_key)
        .bind(runtime_instance_id)
        .execute(&self.db)
        .await?;
        Ok(result.rows_affected())
    }
}

fn resolved_terminal_state(state: &str, decision: Option<&str>) -> Option<&'static str> {
    if !matches!(state, "pending" | "dispatching" | "delivery_unknown") {
        return None;
    }
    Some(match decision {
        Some("rejected") => "rejected",
        Some("answered") => "answered",
        Some(_) => "approved",
        None => "cancelled",
    })
}

fn approval_outcome(state: &str) -> Option<ApprovalOutcome> {
    match state {
        "approved" => Some(ApprovalOutcome::Accepted),
        "rejected" => Some(ApprovalOutcome::Declined),
        "answered" => Some(ApprovalOutcome::Answered),
        "cancelled" => Some(ApprovalOutcome::Cancelled),
        _ => None,
    }
}

fn approval_payload(request_type: &str, params: &Value) -> Value {
    let mut payload = serde_json::Map::new();
    for key in ["threadId", "turnId", "itemId", "reason", "startedAtMs"] {
        if let Some(value) = params.get(key) {
            payload.insert(key.to_string(), value.clone());
        }
    }
    if request_type == COMMAND_APPROVAL {
        if let Some(command) = params.get("command") {
            payload.insert("command".to_string(), command.clone());
        }
    }
    if request_type == PERMISSIONS_APPROVAL {
        if let Some(permissions) = params.get("permissions") {
            payload.insert("permissions".to_string(), permissions.clone());
        }
    }
    if request_type == MCP_ELICITATION_REQUEST {
        for key in ["serverName", "mode", "message", "requestedSchema", "url"] {
            if let Some(value) = params.get(key) {
                payload.insert(key.to_string(), value.clone());
            }
        }
    }
    if request_type == USER_INPUT_REQUEST {
        for key in ["questions", "autoResolutionMs"] {
            if let Some(value) = params.get(key) {
                payload.insert(key.to_string(), value.clone());
            }
        }
    }
    Value::Object(payload)
}

fn validate_user_input_answers(
    payload: &Value,
    request: &RespondUserInputRequest,
) -> Result<(), ApprovalServiceError> {
    let question_ids = payload
        .get("questions")
        .and_then(Value::as_array)
        .ok_or(ApprovalServiceError::Invalid)?
        .iter()
        .filter_map(|question| question.get("id").and_then(Value::as_str))
        .filter(|id| !id.is_empty())
        .collect::<std::collections::BTreeSet<_>>();
    if question_ids.is_empty()
        || request.answers.len() != question_ids.len()
        || !request
            .answers
            .keys()
            .all(|id| question_ids.contains(id.as_str()))
    {
        return Err(ApprovalServiceError::Invalid);
    }
    let mut total_bytes = 0usize;
    for answer in request.answers.values() {
        if answer.answers.is_empty() || answer.answers.len() > 20 {
            return Err(ApprovalServiceError::Invalid);
        }
        for value in &answer.answers {
            if value.contains('\0') || value.len() > 4096 {
                return Err(ApprovalServiceError::Invalid);
            }
            total_bytes = total_bytes.saturating_add(value.len());
        }
    }
    if total_bytes > 64 * 1024 {
        return Err(ApprovalServiceError::Invalid);
    }
    Ok(())
}

async fn insert_audit(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor: ApprovalActor,
    dispatch: &ApprovalDispatch,
    outcome: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO audit_log \
         (organization_id, actor_id, action, target_type, target_id, metadata, outcome) \
         VALUES ($1, $2, 'approval.decide', 'approval', $3, $4, $5)",
    )
    .bind(actor.organization_id)
    .bind(actor.user_id)
    .bind(dispatch.approval_id)
    .bind(json!({ "decision": dispatch.decision }))
    .bind(outcome)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

fn summary_from_row(row: &sqlx::postgres::PgRow) -> ApprovalSummary {
    let payload: Value = row.get("request_payload");
    ApprovalSummary {
        id: row.get("id"),
        run_id: row.get("run_id"),
        thread_id: row.get("thread_id"),
        request_type: row.get("request_type"),
        item_id: payload
            .get("itemId")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        reason: payload
            .get("reason")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        command: payload
            .get("command")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        state: row.get("state"),
        version: row.get("version"),
        created_at: row.get("created_at"),
        decided_at: row.get("decided_at"),
    }
}

fn mcp_elicitation_accept_content(payload: &Value) -> Result<Value, ApprovalServiceError> {
    let mode = payload
        .get("mode")
        .and_then(Value::as_str)
        .ok_or(ApprovalServiceError::Invalid)?;
    if mode == "url" {
        return Ok(Value::Null);
    }
    if mode != "form" {
        return Err(ApprovalServiceError::Invalid);
    }
    let schema = payload
        .get("requestedSchema")
        .and_then(Value::as_object)
        .ok_or(ApprovalServiceError::Invalid)?;
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .ok_or(ApprovalServiceError::Invalid)?;
    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut content = serde_json::Map::new();
    for field in required {
        let field = field.as_str().ok_or(ApprovalServiceError::Invalid)?;
        let property = properties
            .get(field)
            .and_then(Value::as_object)
            .ok_or(ApprovalServiceError::Invalid)?;
        let value = if let Some(value) = property.get("const") {
            value.clone()
        } else if let Some(value) = property.get("default") {
            value.clone()
        } else if property.get("type").and_then(Value::as_str) == Some("boolean") {
            Value::Bool(true)
        } else if let Some(values) = property.get("enum").and_then(Value::as_array) {
            if values.len() != 1 {
                return Err(ApprovalServiceError::Invalid);
            }
            values[0].clone()
        } else {
            return Err(ApprovalServiceError::Invalid);
        };
        content.insert(field.to_string(), value);
    }
    Ok(Value::Object(content))
}

fn approval_response(
    request_type: &str,
    payload: &Value,
    decision: ApprovalDecision,
) -> Result<(Value, &'static str, &'static str), ApprovalServiceError> {
    let (terminal_state, stored_decision) = match decision {
        ApprovalDecision::Accept | ApprovalDecision::AcceptForSession => ("approved", "approved"),
        ApprovalDecision::Decline => ("rejected", "rejected"),
        ApprovalDecision::Cancel => ("cancelled", "rejected"),
    };
    let response = match request_type {
        COMMAND_APPROVAL | FILE_APPROVAL => {
            let decision = match decision {
                ApprovalDecision::Accept => "accept",
                ApprovalDecision::AcceptForSession => "acceptForSession",
                ApprovalDecision::Decline => "decline",
                ApprovalDecision::Cancel => "cancel",
            };
            json!({ "decision": decision })
        }
        PERMISSIONS_APPROVAL => match decision {
            ApprovalDecision::Accept => {
                json!({ "permissions": payload.get("permissions").cloned().unwrap_or_else(|| json!({})), "scope": "turn" })
            }
            ApprovalDecision::AcceptForSession => {
                json!({ "permissions": payload.get("permissions").cloned().unwrap_or_else(|| json!({})), "scope": "session" })
            }
            ApprovalDecision::Decline | ApprovalDecision::Cancel => {
                json!({ "permissions": {}, "scope": "turn" })
            }
        },
        MCP_ELICITATION_REQUEST => match decision {
            ApprovalDecision::Accept | ApprovalDecision::AcceptForSession => {
                json!({
                    "action": "accept",
                    "content": mcp_elicitation_accept_content(payload)?,
                    "_meta": null
                })
            }
            ApprovalDecision::Decline => {
                json!({ "action": "decline", "content": null, "_meta": null })
            }
            ApprovalDecision::Cancel => {
                json!({ "action": "cancel", "content": null, "_meta": null })
            }
        },
        _ => return Err(ApprovalServiceError::Invalid),
    };
    Ok((response, terminal_state, stored_decision))
}

#[cfg(test)]
mod tests {
    use super::{
        approval_outcome, approval_response, resolved_terminal_state, validate_user_input_answers,
        ApprovalOutcome, COMMAND_APPROVAL, MCP_ELICITATION_REQUEST, PERMISSIONS_APPROVAL,
    };
    use open_web_codex_platform_contracts::{
        ApprovalDecision, RespondUserInputRequest, UserInputAnswer,
    };
    use serde_json::json;
    use std::collections::BTreeMap;

    #[test]
    fn maps_platform_decisions_to_typed_runtime_responses() {
        let (command, state, _) = approval_response(
            COMMAND_APPROVAL,
            &json!({}),
            ApprovalDecision::AcceptForSession,
        )
        .unwrap();
        assert_eq!(command, json!({ "decision": "acceptForSession" }));
        assert_eq!(state, "approved");

        let (permissions, state, _) = approval_response(
            PERMISSIONS_APPROVAL,
            &json!({ "permissions": { "network": { "enabled": true } } }),
            ApprovalDecision::Accept,
        )
        .unwrap();
        assert_eq!(permissions["scope"], "turn");
        assert_eq!(permissions["permissions"]["network"]["enabled"], true);
        assert_eq!(state, "approved");
    }

    #[test]
    fn maps_mcp_confirmation_elicitations_to_typed_runtime_responses() {
        let confirmation = json!({
            "mode": "form",
            "requestedSchema": {
                "type": "object",
                "properties": {
                    "confirmed": { "type": "boolean" }
                },
                "required": ["confirmed"]
            }
        });
        let (accepted, state, _) = approval_response(
            MCP_ELICITATION_REQUEST,
            &confirmation,
            ApprovalDecision::Accept,
        )
        .unwrap();
        assert_eq!(
            accepted,
            json!({
                "action": "accept",
                "content": { "confirmed": true },
                "_meta": null
            })
        );
        assert_eq!(state, "approved");

        let empty = json!({
            "mode": "form",
            "requestedSchema": {
                "type": "object",
                "properties": {}
            }
        });
        let (accepted, _, _) =
            approval_response(MCP_ELICITATION_REQUEST, &empty, ApprovalDecision::Accept).unwrap();
        assert_eq!(accepted["content"], json!({}));

        let (declined, state, _) = approval_response(
            MCP_ELICITATION_REQUEST,
            &confirmation,
            ApprovalDecision::Decline,
        )
        .unwrap();
        assert_eq!(
            declined,
            json!({ "action": "decline", "content": null, "_meta": null })
        );
        assert_eq!(state, "rejected");

        let url_request = json!({
            "mode": "url",
            "url": "http://127.0.0.1:43123/one-time-token"
        });
        let (accepted, state, _) = approval_response(
            MCP_ELICITATION_REQUEST,
            &url_request,
            ApprovalDecision::Accept,
        )
        .unwrap();
        assert_eq!(
            accepted,
            json!({ "action": "accept", "content": null, "_meta": null })
        );
        assert_eq!(state, "approved");
    }

    #[test]
    fn rejects_mcp_form_acceptance_when_the_card_cannot_supply_required_input() {
        let payload = json!({
            "mode": "form",
            "requestedSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" }
                },
                "required": ["name"]
            }
        });
        assert!(
            approval_response(MCP_ELICITATION_REQUEST, &payload, ApprovalDecision::Accept,)
                .is_err()
        );
    }

    #[test]
    fn validates_user_input_answers_against_the_persisted_questions() {
        let payload = json!({
            "questions": [
                { "id": "first", "question": "First?" },
                { "id": "second", "question": "Second?" }
            ]
        });
        let request = RespondUserInputRequest {
            answers: BTreeMap::from([
                (
                    "first".to_string(),
                    UserInputAnswer {
                        answers: vec!["yes".to_string()],
                    },
                ),
                (
                    "second".to_string(),
                    UserInputAnswer {
                        answers: vec!["details".to_string()],
                    },
                ),
            ]),
            version: 0,
        };
        assert!(validate_user_input_answers(&payload, &request).is_ok());

        let missing = RespondUserInputRequest {
            answers: BTreeMap::from([(
                "first".to_string(),
                UserInputAnswer {
                    answers: vec!["yes".to_string()],
                },
            )]),
            version: 0,
        };
        assert!(validate_user_input_answers(&payload, &missing).is_err());
    }

    #[test]
    fn runtime_resolution_settles_only_active_approvals_and_preserves_decisions() {
        assert_eq!(resolved_terminal_state("pending", None), Some("cancelled"));
        assert_eq!(
            resolved_terminal_state("dispatching", Some("approved")),
            Some("approved")
        );
        assert_eq!(
            resolved_terminal_state("delivery_unknown", Some("rejected")),
            Some("rejected")
        );
        assert_eq!(
            resolved_terminal_state("dispatching", Some("answered")),
            Some("answered")
        );
        assert_eq!(resolved_terminal_state("approved", Some("approved")), None);
        assert_eq!(resolved_terminal_state("rejected", Some("rejected")), None);
        assert_eq!(
            approval_outcome("approved"),
            Some(ApprovalOutcome::Accepted)
        );
        assert_eq!(
            approval_outcome("rejected"),
            Some(ApprovalOutcome::Declined)
        );
        assert_eq!(
            approval_outcome("answered"),
            Some(ApprovalOutcome::Answered)
        );
        assert_eq!(
            approval_outcome("cancelled"),
            Some(ApprovalOutcome::Cancelled)
        );
    }
}
