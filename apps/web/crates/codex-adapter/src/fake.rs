use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use std::collections::HashMap;

use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use chrono::Utc;
use serde_json::{json, Value};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::{
    AdapterError, AuthorizedWorkspace, CanceledProfileLogin, CodexAdapter, HealthStatus,
    ProfileLoginStatus, ProfileMutation, ProfileQuery, ReviewTarget, StartedProfileLogin,
    StartedThread, TurnOptions,
};

/// A tracked mock thread for list/show responses.
#[derive(Clone)]
struct MockThread {
    id: String,
    ws_id: String,
    created_at: String,
    status: String,
    msg_count: u64,
    updated_at: i64,
}

/// In-memory state shared between RPC handlers and event generator.
struct FakeState {
    workspaces: Vec<Value>,
    threads: Vec<MockThread>,
    /// Events queued by RPC handlers (e.g. thread/started).
    pending_events: Vec<Value>,
}

/// In-memory Codex adapter that simulates workspace, thread and event flows.
pub struct FakeCodexAdapter {
    state: Arc<Mutex<FakeState>>,
    active_login_id: Arc<Mutex<Option<String>>>,
    login_statuses: Arc<Mutex<HashMap<String, ProfileLoginStatus>>>,
    notify: Arc<tokio::sync::Notify>,
    counter: Arc<AtomicU64>,
    runtime_instance_id: Uuid,
}

impl Default for FakeCodexAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeCodexAdapter {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(FakeState {
                workspaces: vec![],
                threads: vec![],
                pending_events: vec![],
            })),
            active_login_id: Arc::new(Mutex::new(None)),
            login_statuses: Arc::new(Mutex::new(HashMap::new())),
            notify: Arc::new(tokio::sync::Notify::new()),
            counter: Arc::new(AtomicU64::new(1)),
            runtime_instance_id: Uuid::now_v7(),
        }
    }

    /// Pre-populate with a sample workspace.
    pub async fn with_demo_workspace(self) -> Self {
        let mut state = self.state.lock().await;
        state.workspaces.push(json!({
            "id": Uuid::now_v7().to_string(),
            "name": "open-web-codex",
            "path": &std::env::current_dir()
                .unwrap_or_else(|_| "/tmp/demo".into())
                .to_string_lossy()
                .to_string(),
            "connected": false,
            "kind": "main",
            "settings": { "sidebarCollapsed": false },
        }));
        drop(state);
        self
    }

    /// Helper: push an SSE frame event and notify the event loop.
    async fn emit(&self, evt: Value) {
        let mut state = self.state.lock().await;
        state.pending_events.push(evt);
        self.notify.notify_one();
    }

    /// Build a thread/started SSE frame.
    fn started_event(ws_id: &str, th_id: &str) -> Value {
        json!({
            "method": "app-server-event",
            "params": {
                "workspace_id": ws_id,
                "message": {
                    "method": "thread/started",
                    "params": { "threadId": th_id },
                },
            },
        })
    }

    fn completed_event(ws_id: &str, th_id: &str) -> Value {
        json!({
            "method": "app-server-event",
            "params": {
                "workspace_id": ws_id,
                "message": {
                    "method": "thread/completed",
                    "params": { "threadId": th_id },
                },
            },
        })
    }
}

#[async_trait]
impl CodexAdapter for FakeCodexAdapter {
    async fn health(&self) -> Result<HealthStatus, AdapterError> {
        Ok(HealthStatus {
            ok: true,
            version: "0.1.0-mock".into(),
            name: "open-web-codex-mock".into(),
        })
    }

    async fn runtime_instance_id(&self) -> Uuid {
        self.runtime_instance_id
    }

    async fn rpc(&self, method: &str, params: Value) -> Result<Value, AdapterError> {
        match method {
            "list_workspaces" => {
                let s = self.state.lock().await;
                Ok(Value::Array(s.workspaces.clone()))
            }

            "add_workspace" => {
                let path = params["path"]
                    .as_str()
                    .ok_or_else(|| AdapterError::Internal("missing path".into()))?
                    .to_string();
                let mut s = self.state.lock().await;
                let id = Uuid::now_v7().to_string();
                let name = path.rsplit('/').next().unwrap_or("workspace").to_string();
                let ws = json!({
                    "id": id, "name": name, "path": path,
                    "connected": false, "kind": "main",
                    "settings": { "sidebarCollapsed": false },
                });
                s.workspaces.push(ws.clone());
                Ok(ws)
            }

            "connect_workspace" => {
                let id = params["id"]
                    .as_str()
                    .ok_or_else(|| AdapterError::Internal("missing id".into()))?;
                let mut s = self.state.lock().await;
                for ws in &mut s.workspaces {
                    if ws["id"] == id {
                        ws["connected"] = json!(true);
                        return Ok(json!({}));
                    }
                }
                Err(AdapterError::Rpc(format!("workspace not found: {id}")))
            }

            "start_thread" => {
                let ws_id = params["workspaceId"]
                    .as_str()
                    .ok_or_else(|| AdapterError::Internal("missing workspaceId".into()))?;
                let now = Utc::now();
                {
                    let s = self.state.lock().await;
                    if !s.workspaces.iter().any(|w| w["id"] == ws_id) {
                        return Err(AdapterError::Rpc(format!("workspace not found: {ws_id}")));
                    }
                }
                let n = self.counter.fetch_add(1, Ordering::SeqCst);
                let thread_id = format!("mock-thread-{:04x}", n);
                let created = now.to_rfc3339();
                let updated_ts = now.timestamp_millis();

                {
                    let mut s = self.state.lock().await;
                    s.threads.push(MockThread {
                        id: thread_id.clone(),
                        ws_id: ws_id.to_string(),
                        created_at: created.clone(),
                        status: "active".into(),
                        msg_count: 0,
                        updated_at: updated_ts,
                    });
                }
                // Emit thread/started event immediately
                self.emit(Self::started_event(ws_id, &thread_id)).await;

                Ok(json!({ "threadId": thread_id, "createdAt": created }))
            }

            "list_threads" => {
                let ws_id = params["workspaceId"].as_str().unwrap_or("");
                let s = self.state.lock().await;
                let threads: Vec<Value> = s
                    .threads
                    .iter()
                    .filter(|t| t.ws_id == ws_id)
                    .map(|t| {
                        json!({
                            "id": t.id, "name": format!("Fake Thread ({})", &t.id[13..]),
                            "createdAt": t.created_at, "updatedAt": t.updated_at,
                            "messageCount": t.msg_count, "status": t.status,
                        })
                    })
                    .collect();
                Ok(json!({ "threads": threads, "totalCount": threads.len() }))
            }

            "send_user_message" => {
                let ws_id = params["workspaceId"].as_str().unwrap_or("");
                let th_id = params["threadId"].as_str().unwrap_or("");
                let text = params["text"].as_str().unwrap_or("");
                tracing::info!(ws = %ws_id, thread = %th_id, text = %text, "fake: user message");

                // Increment message count
                {
                    let mut s = self.state.lock().await;
                    if let Some(th) = s.threads.iter_mut().find(|t| t.id == th_id) {
                        th.msg_count += 1;
                        th.updated_at = Utc::now().timestamp_millis();
                    }
                }

                // Schedule a mock turn after a brief delay (handled by subscribe_events).
                let turn_id = format!("turn-{}", Uuid::now_v7());
                Ok(json!({ "status": "sent", "turnId": turn_id }))
            }

            other => Err(AdapterError::NotImplemented(format!(
                "fake adapter: method '{other}' not implemented"
            ))),
        }
    }

    async fn start_thread(
        &self,
        workspace: &AuthorizedWorkspace,
    ) -> Result<StartedThread, AdapterError> {
        {
            let mut state = self.state.lock().await;
            if !state
                .workspaces
                .iter()
                .any(|value| value["id"] == workspace.id)
            {
                state.workspaces.push(json!({
                    "id": workspace.id,
                    "name": workspace.id,
                    "path": workspace.root,
                    "connected": true,
                    "kind": "run",
                }));
            }
        }
        let result = self
            .rpc("start_thread", json!({ "workspaceId": workspace.id }))
            .await?;
        let thread_id = result
            .get("threadId")
            .and_then(Value::as_str)
            .ok_or_else(|| AdapterError::Rpc("fake Thread omitted id".to_string()))?;
        Ok(StartedThread {
            thread_id: thread_id.to_string(),
        })
    }

    async fn fork_thread(
        &self,
        _source_workspace: &AuthorizedWorkspace,
        target_workspace: &AuthorizedWorkspace,
        thread_id: &str,
    ) -> Result<StartedThread, AdapterError> {
        if thread_id.trim().is_empty() {
            return Err(AdapterError::Internal(
                "fork source Thread is required".to_string(),
            ));
        }
        self.start_thread(target_workspace).await
    }

    async fn read_thread(
        &self,
        workspace: &AuthorizedWorkspace,
        thread_id: &str,
    ) -> Result<Value, AdapterError> {
        let state = self.state.lock().await;
        let thread = state
            .threads
            .iter()
            .find(|thread| thread.id == thread_id && thread.ws_id == workspace.id)
            .ok_or_else(|| AdapterError::Rpc("fake Thread was not found".to_string()))?;
        Ok(json!({
            "thread": {
                "id": thread.id,
                "name": format!("Fake Thread ({})", thread.id),
                "preview": "",
                "createdAt": Utc::now().timestamp(),
                "updatedAt": thread.updated_at / 1000,
                "status": { "type": if thread.status == "active" { "active" } else { "idle" } },
                "turns": [],
            }
        }))
    }

    async fn list_thread_turns(
        &self,
        workspace: &AuthorizedWorkspace,
        thread_id: &str,
    ) -> Result<Vec<Value>, AdapterError> {
        self.read_thread(workspace, thread_id).await?;
        Ok(Vec::new())
    }

    async fn read_mcp_resource(
        &self,
        workspace: &AuthorizedWorkspace,
        thread_id: &str,
        server: &str,
        uri: &str,
    ) -> Result<Value, AdapterError> {
        self.read_thread(workspace, thread_id).await?;
        if server.trim().is_empty() || uri.trim().is_empty() {
            return Err(AdapterError::Internal(
                "MCP Resource server and URI are required".to_string(),
            ));
        }
        Err(AdapterError::NotImplemented(
            "fake adapter does not provide MCP Resources".to_string(),
        ))
    }

    async fn send_user_message(
        &self,
        workspace: &AuthorizedWorkspace,
        thread_id: &str,
        text: &str,
        _options: &TurnOptions,
    ) -> Result<Value, AdapterError> {
        self.rpc(
            "send_user_message",
            json!({
                "workspaceId": workspace.id,
                "threadId": thread_id,
                "text": text,
            }),
        )
        .await
    }

    async fn steer_turn(
        &self,
        workspace: &AuthorizedWorkspace,
        thread_id: &str,
        turn_id: &str,
        text: &str,
        _images: &[String],
    ) -> Result<Value, AdapterError> {
        if workspace.id.trim().is_empty()
            || thread_id.trim().is_empty()
            || turn_id.trim().is_empty()
            || text.trim().is_empty()
        {
            return Err(AdapterError::Internal(
                "workspace, Thread, Turn, and text are required".to_string(),
            ));
        }
        Ok(json!({ "status": "steered", "turnId": turn_id }))
    }

    async fn interrupt_turn(
        &self,
        workspace: &AuthorizedWorkspace,
        thread_id: &str,
        turn_id: &str,
    ) -> Result<(), AdapterError> {
        if workspace.id.trim().is_empty()
            || thread_id.trim().is_empty()
            || turn_id.trim().is_empty()
        {
            return Err(AdapterError::Internal(
                "workspace, Thread, and Turn ids are required".to_string(),
            ));
        }
        let mut state = self.state.lock().await;
        let thread = state
            .threads
            .iter_mut()
            .find(|thread| thread.id == thread_id && thread.ws_id == workspace.id)
            .ok_or_else(|| AdapterError::Rpc("fake Thread was not found".to_string()))?;
        thread.status = "interrupted".to_string();
        Ok(())
    }

    async fn respond_to_server_request(
        &self,
        runtime_instance_id: Uuid,
        _request_id: Value,
        _result: Value,
    ) -> Result<(), AdapterError> {
        if runtime_instance_id != self.runtime_instance_id {
            return Err(AdapterError::Rpc(
                "Server Request belongs to a previous Runtime instance".to_string(),
            ));
        }
        Ok(())
    }

    async fn query_profile(&self, query: ProfileQuery) -> Result<Value, AdapterError> {
        Ok(match query {
            ProfileQuery::Account => {
                json!({ "account": null, "requiresOpenaiAuth": false })
            }
            ProfileQuery::RateLimits => json!({}),
            ProfileQuery::Usage => json!({
                "summary": { "lifetimeTokens": 0, "peakDailyTokens": 0 },
                "dailyUsageBuckets": []
            }),
            ProfileQuery::CollaborationModes => json!({ "data": [] }),
            ProfileQuery::Apps { .. }
            | ProfileQuery::McpServers { .. }
            | ProfileQuery::ExperimentalFeatures { .. } => {
                json!({ "data": [], "nextCursor": null })
            }
            ProfileQuery::Skills { .. } => json!({ "data": [] }),
            ProfileQuery::Config => json!({
                "config": {
                    "features": { "multi_agent": true },
                    "agents": { "max_threads": 6, "max_depth": 1 }
                }
            }),
        })
    }

    async fn mutate_profile(&self, _mutation: ProfileMutation) -> Result<Value, AdapterError> {
        Ok(json!({ "status": "ok" }))
    }

    async fn start_profile_login(&self) -> Result<StartedProfileLogin, AdapterError> {
        let login_id = Uuid::now_v7().to_string();
        *self.active_login_id.lock().await = Some(login_id.clone());
        self.login_statuses.lock().await.insert(
            login_id.clone(),
            ProfileLoginStatus {
                completed: true,
                success: Some(true),
                error: None,
            },
        );
        Ok(StartedProfileLogin {
            login_id,
            auth_url: "https://example.invalid/codex-login".to_string(),
        })
    }

    async fn cancel_profile_login(&self) -> Result<CanceledProfileLogin, AdapterError> {
        let login_id = self.active_login_id.lock().await.take();
        let canceled = login_id.is_some();
        if let Some(login_id) = login_id {
            self.login_statuses.lock().await.remove(&login_id);
        }
        Ok(CanceledProfileLogin {
            canceled,
            status: if canceled { "canceled" } else { "notFound" }.to_string(),
        })
    }

    async fn profile_login_status(
        &self,
        login_id: &str,
    ) -> Result<ProfileLoginStatus, AdapterError> {
        let mut statuses = self.login_statuses.lock().await;
        let status = statuses
            .get(login_id)
            .cloned()
            .ok_or_else(|| AdapterError::Rpc("fake Profile login was not found".to_string()))?;
        if status.completed {
            statuses.remove(login_id);
        }
        Ok(status)
    }

    async fn archive_thread(
        &self,
        workspace: &AuthorizedWorkspace,
        thread_id: &str,
    ) -> Result<(), AdapterError> {
        let mut state = self.state.lock().await;
        let thread = state
            .threads
            .iter_mut()
            .find(|thread| thread.id == thread_id && thread.ws_id == workspace.id)
            .ok_or_else(|| AdapterError::Rpc("fake Thread was not found".to_string()))?;
        thread.status = "archived".to_string();
        Ok(())
    }

    async fn set_thread_name(
        &self,
        workspace: &AuthorizedWorkspace,
        thread_id: &str,
        _name: &str,
    ) -> Result<(), AdapterError> {
        let state = self.state.lock().await;
        if state
            .threads
            .iter()
            .any(|thread| thread.id == thread_id && thread.ws_id == workspace.id)
        {
            Ok(())
        } else {
            Err(AdapterError::Rpc("fake Thread was not found".to_string()))
        }
    }

    async fn generate_text(
        &self,
        _workspace: &AuthorizedWorkspace,
        prompt: &str,
        _model: Option<&str>,
    ) -> Result<String, AdapterError> {
        if prompt.contains("worktreeName") {
            Ok(r#"{"title":"Thread","worktreeName":"feat/thread"}"#.to_string())
        } else if prompt.contains("developerInstructions") {
            Ok(r#"{"description":"Specialized agent","developerInstructions":"Complete the requested specialty carefully and report concrete results."}"#.to_string())
        } else {
            Ok("Update workspace changes".to_string())
        }
    }

    async fn compact_thread(
        &self,
        workspace: &AuthorizedWorkspace,
        thread_id: &str,
    ) -> Result<Value, AdapterError> {
        if workspace.id.is_empty() || thread_id.is_empty() {
            return Err(AdapterError::Internal(
                "workspace and Thread are required".to_string(),
            ));
        }
        Ok(json!({ "status": "compacting" }))
    }

    async fn start_review(
        &self,
        workspace: &AuthorizedWorkspace,
        thread_id: &str,
        _target: ReviewTarget,
    ) -> Result<Value, AdapterError> {
        if workspace.id.is_empty() || thread_id.is_empty() {
            return Err(AdapterError::Internal(
                "workspace and Thread are required".to_string(),
            ));
        }
        Ok(json!({
            "turn": { "id": format!("review-{}", Uuid::now_v7()), "status": "inProgress" },
            "reviewThreadId": thread_id,
        }))
    }

    async fn open_terminal(
        &self,
        workspace: &AuthorizedWorkspace,
        process_id: &str,
        _cols: u16,
        _rows: u16,
    ) -> Result<(), AdapterError> {
        self.emit(json!({
            "method": "app-server-event",
            "params": {
                "workspace_id": workspace.id,
                "message": {
                    "method": "command/exec/outputDelta",
                    "params": {
                        "processId": process_id,
                        "stream": "stdout",
                        "deltaBase64": BASE64.encode(b"fake-shell$ "),
                        "capReached": false,
                    },
                },
            },
        }))
        .await;
        Ok(())
    }

    async fn write_terminal(
        &self,
        workspace: &AuthorizedWorkspace,
        process_id: &str,
        data: &str,
    ) -> Result<(), AdapterError> {
        self.emit(json!({
            "method": "app-server-event",
            "params": {
                "workspace_id": workspace.id,
                "message": {
                    "method": "command/exec/outputDelta",
                    "params": {
                        "processId": process_id,
                        "stream": "stdout",
                        "deltaBase64": BASE64.encode(data.as_bytes()),
                        "capReached": false,
                    },
                },
            },
        }))
        .await;
        Ok(())
    }

    async fn resize_terminal(
        &self,
        _workspace: &AuthorizedWorkspace,
        _process_id: &str,
        _cols: u16,
        _rows: u16,
    ) -> Result<(), AdapterError> {
        Ok(())
    }

    async fn close_terminal(
        &self,
        workspace: &AuthorizedWorkspace,
        process_id: &str,
    ) -> Result<(), AdapterError> {
        self.emit(json!({
            "method": "app-server-event",
            "params": {
                "workspace_id": workspace.id,
                "message": {
                    "method": "platform/terminalExited",
                    "params": {
                        "processId": process_id,
                        "workspaceId": workspace.id,
                        "exitCode": 0,
                        "failed": false,
                    },
                },
            },
        }))
        .await;
        Ok(())
    }

    async fn subscribe_events(&self, sender: UnboundedSender<Vec<u8>>) -> Result<(), AdapterError> {
        let state = self.state.clone();
        let notify = self.notify.clone();
        let counter = self.counter.clone();
        let runtime_instance_id = self.runtime_instance_id;

        tokio::spawn(async move {
            let mut heartbeat = tokio::time::interval(tokio::time::Duration::from_secs(10));
            let mut item_timer = tokio::time::interval(tokio::time::Duration::from_secs(8));
            let mut item_seq: u64 = 0;
            let mut thread_ticks: HashMap<String, u32> = HashMap::new();

            // Helper: send an SSE frame
            let send = |mut data: Value| {
                if let Some(params) = data.get_mut("params").and_then(Value::as_object_mut) {
                    params.insert(
                        "runtime_instance_id".to_string(),
                        Value::String(runtime_instance_id.to_string()),
                    );
                }
                let frame = format!("data: {}\n\n", data.to_string());
                let _ = sender.send(frame.into_bytes());
            };

            loop {
                tokio::select! {
                    _ = notify.notified() => {
                        // Drain pending events
                        let events = {
                            let mut s = state.lock().await;
                            std::mem::take(&mut s.pending_events)
                        };
                        for evt in events { send(evt); }
                    }
                    _ = heartbeat.tick() => {
                        let n = counter.fetch_add(1, Ordering::SeqCst);
                        send(json!({
                            "method": "app-server-event",
                            "params": {
                                "workspace_id": null,
                                "message": {
                                    "method": "heartbeat",
                                    "params": { "seq": n },
                                },
                            },
                        }));
                    }
                    _ = item_timer.tick() => {
                        // For each active thread, emit mock item activity
                        let threads = {
                            let s = state.lock().await;
                            s.threads.iter()
                                .filter(|t| t.status == "active")
                                .map(|t| (t.id.clone(), t.ws_id.clone()))
                                .collect::<Vec<_>>()
                        };
                        for (th_id, ws_id) in &threads {
                            let ticks = thread_ticks.entry(th_id.clone()).or_insert(0);
                            if *ticks >= 3 {
                                // Emit thread/completed
                                send(FakeCodexAdapter::completed_event(ws_id, th_id));
                                // Mark thread completed in state
                                {
                                    let mut s = state.lock().await;
                                    if let Some(th) = s.threads.iter_mut().find(|t| t.id == *th_id) {
                                        th.status = "completed".to_string();
                                    }
                                }
                                tracing::info!(thread = %th_id, "mock thread completed after {ticks} ticks");
                                continue;
                            }
                            *ticks += 1;

                            item_seq += 1;
                            let item_id = format!("mock-item-{item_seq:04x}");

                            // Cycle through item types
                            let item_kind = match item_seq % 4 {
                                0 => "plan",
                                1 => "explore",
                                2 => "tool",
                                _ => "message",
                            };
                            let desc = match item_kind {
                                "plan" => "Analyzing project structure and dependencies",
                                "explore" => "Reading source files for context",
                                "tool" => "Running analysis tools",
                                _ => "Processing request",
                            };

                            send(json!({
                                "method": "app-server-event",
                                "params": {
                                    "workspace_id": ws_id,
                                    "message": {
                                        "method": "item/started",
                                        "params": {
                                            "threadId": th_id,
                                            "item": {
                                                "id": item_id,
                                                "kind": item_kind,
                                                "description": desc,
                                                "status": "in_progress",
                                            },
                                        },
                                    },
                                },
                            }));

                            // Mark completed shortly after (simulated by next tick's item)
                            send(json!({
                                "method": "app-server-event",
                                "params": {
                                    "workspace_id": ws_id,
                                    "message": {
                                        "method": "item/completed",
                                        "params": {
                                            "threadId": th_id,
                                            "item": {
                                                "id": item_id,
                                                "kind": item_kind,
                                                "status": "completed",
                                            },
                                        },
                                    },
                                },
                            }));
                        }
                    }
                }

                if sender.is_closed() {
                    break;
                }
            }
        });

        Ok(())
    }
}
