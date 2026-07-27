use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use open_web_codex_profile_host::{ProfileHost, ProfileHostConfig, ProfileHostState};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, OnceLock};
use tokio::sync::broadcast;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::Mutex;
use tokio::sync::OwnedMutexGuard;
use tokio::sync::RwLock;

use crate::{
    AdapterError, AuthorizedWorkspace, CanceledProfileLogin, CodexAdapter, HealthStatus,
    ProfileLoginStatus, ProfileMutation, ProfileQuery, ReviewTarget, StartedProfileLogin,
    StartedThread, TurnOptions,
};

/// Adapter backed directly by a native Profile Host and Codex app-server
/// JSONL connection without an intermediate local gateway.
pub struct RealCodexAdapter {
    host: ProfileHost,
    workspace_id: String,
    workspace_root: PathBuf,
    thread_workspaces: Arc<RwLock<HashMap<String, AuthorizedWorkspace>>>,
    thread_history_modes: Arc<RwLock<HashMap<String, bool>>>,
    suppressed_threads: Arc<RwLock<HashSet<String>>>,
    active_login_id: Arc<RwLock<Option<String>>>,
    login_statuses: Arc<RwLock<HashMap<String, ProfileLoginStatus>>>,
    terminal_workspaces: Arc<RwLock<HashMap<String, AuthorizedWorkspace>>>,
    runtime_instance: Arc<Mutex<Option<uuid::Uuid>>>,
    local_events: broadcast::Sender<Value>,
}

impl RealCodexAdapter {
    pub async fn spawn(
        config: ProfileHostConfig,
        workspace_id: impl Into<String>,
    ) -> Result<Self, AdapterError> {
        let workspace_root = config.workspace_root.clone();
        let host = ProfileHost::spawn(config).await?;
        Self::from_host(host, workspace_id, workspace_root)
    }

    pub fn from_host(
        host: ProfileHost,
        workspace_id: impl Into<String>,
        workspace_root: PathBuf,
    ) -> Result<Self, AdapterError> {
        let workspace_root = workspace_root.canonicalize().map_err(|error| {
            AdapterError::Internal(format!("failed to resolve workspace root: {error}"))
        })?;
        let (local_events, _) = broadcast::channel(256);
        Ok(Self {
            host,
            workspace_id: workspace_id.into(),
            workspace_root,
            thread_workspaces: Arc::new(RwLock::new(HashMap::new())),
            thread_history_modes: Arc::new(RwLock::new(HashMap::new())),
            suppressed_threads: Arc::new(RwLock::new(HashSet::new())),
            active_login_id: Arc::new(RwLock::new(None)),
            login_statuses: Arc::new(RwLock::new(HashMap::new())),
            terminal_workspaces: Arc::new(RwLock::new(HashMap::new())),
            runtime_instance: Arc::new(Mutex::new(None)),
            local_events,
        })
    }

    /// Clone the native Profile connection for server-owned typed services.
    /// The browser must never receive this transport directly.
    pub fn profile_host(&self) -> ProfileHost {
        self.host.clone()
    }

    fn require_workspace(&self, params: &Value) -> Result<(), AdapterError> {
        let requested = params
            .get("workspaceId")
            .and_then(Value::as_str)
            .ok_or_else(|| AdapterError::Internal("missing workspaceId".to_string()))?;
        if requested != self.workspace_id {
            return Err(AdapterError::Rpc(format!(
                "workspace '{requested}' is not registered with this Profile Host"
            )));
        }
        Ok(())
    }

    fn authorized_root(&self, workspace: &AuthorizedWorkspace) -> Result<String, AdapterError> {
        let root = workspace.root.canonicalize().map_err(|error| {
            AdapterError::Internal(format!("failed to resolve authorized workspace: {error}"))
        })?;
        if !is_authorized_workspace_root(&self.workspace_root, &root) {
            return Err(AdapterError::Rpc(
                "workspace is outside the Profile Host Runner root".to_string(),
            ));
        }
        Ok(root.to_string_lossy().to_string())
    }

    fn thread_start_params(&self, workspace_root: &str) -> Value {
        let mut params = json!({
            "cwd": workspace_root,
            "approvalPolicy": "on-request",
            "historyMode": "paginated",
        });
        add_selected_capability_roots(&mut params, Path::new(workspace_root));
        params
    }

    fn thread_resume_params(&self, thread_id: &str, workspace_root: &str) -> Value {
        json!({
            "threadId": thread_id,
            "cwd": workspace_root,
            "approvalPolicy": "on-request",
            "excludeTurns": true,
        })
    }

    fn thread_fork_params(&self, thread_id: &str, target_root: &str) -> Value {
        json!({
            "threadId": thread_id,
            "cwd": target_root,
            "approvalPolicy": "on-request",
        })
    }

    async fn prepare_runtime(&self) -> Result<OwnedMutexGuard<Option<uuid::Uuid>>, AdapterError> {
        let mut runtime_instance = self.runtime_instance.clone().lock_owned().await;
        self.host.apply_scheduled_restart().await?;
        let current = self.host.runtime_instance_id().await;
        if runtime_instance.as_ref() != Some(&current) {
            self.thread_workspaces.write().await.clear();
            self.terminal_workspaces.write().await.clear();
            *runtime_instance = Some(current);
        }
        Ok(runtime_instance)
    }

    async fn start_thread_in_workspace(
        &self,
        workspace: &AuthorizedWorkspace,
    ) -> Result<StartedThread, AdapterError> {
        let _runtime = self.prepare_runtime().await?;
        let workspace_root = self.authorized_root(workspace)?;
        let result = self
            .host
            .request("thread/start", self.thread_start_params(&workspace_root))
            .await?;
        let thread_id = result
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                AdapterError::Rpc("thread/start response omitted thread.id".to_string())
            })?;
        self.thread_workspaces
            .write()
            .await
            .insert(thread_id.to_string(), workspace.clone());
        self.thread_history_modes
            .write()
            .await
            .insert(thread_id.to_string(), true);
        Ok(StartedThread {
            thread_id: thread_id.to_string(),
        })
    }

    async fn ensure_thread_bound(
        &self,
        workspace: &AuthorizedWorkspace,
        thread_id: &str,
    ) -> Result<(String, OwnedMutexGuard<Option<uuid::Uuid>>), AdapterError> {
        if thread_id.trim().is_empty() {
            return Err(AdapterError::Internal("Thread id is required".to_string()));
        }
        let runtime = self.prepare_runtime().await?;
        let workspace_root = self.authorized_root(workspace)?;
        if let Some(bound) = self.thread_workspaces.read().await.get(thread_id).cloned() {
            if bound == *workspace {
                return Ok((workspace_root, runtime));
            }
            return Err(AdapterError::Rpc(
                "Thread is not bound to the authorized workspace".to_string(),
            ));
        }

        let resumed = self
            .host
            .request(
                "thread/resume",
                self.thread_resume_params(thread_id, &workspace_root),
            )
            .await?;
        let paginated = resumed
            .pointer("/thread/historyMode")
            .and_then(Value::as_str)
            .is_some_and(|mode| mode == "paginated");
        self.thread_workspaces
            .write()
            .await
            .insert(thread_id.to_string(), workspace.clone());
        self.thread_history_modes
            .write()
            .await
            .insert(thread_id.to_string(), paginated);
        Ok((workspace_root, runtime))
    }

    async fn list_paginated_turn_shells(
        &self,
        thread_id: &str,
    ) -> Result<Vec<Value>, AdapterError> {
        let mut turns = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let response = self
                .host
                .request(
                    "thread/turns/list",
                    json!({
                        "threadId": thread_id,
                        "cursor": cursor,
                        "limit": 100,
                        "sortDirection": "asc",
                        "itemsView": "notLoaded",
                    }),
                )
                .await?;
            let page = response
                .get("data")
                .and_then(Value::as_array)
                .ok_or_else(|| AdapterError::Rpc("thread/turns/list omitted data".to_string()))?;
            turns.extend(page.iter().cloned());
            let next_cursor = response
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_string);
            if next_cursor.is_none() {
                return Ok(turns);
            }
            if next_cursor == cursor {
                return Err(AdapterError::Rpc(
                    "thread/turns/list returned a non-advancing cursor".to_string(),
                ));
            }
            cursor = next_cursor;
        }
    }

    async fn list_paginated_thread_items(
        &self,
        thread_id: &str,
    ) -> Result<HashMap<String, Vec<Value>>, AdapterError> {
        let mut items_by_turn = HashMap::<String, Vec<Value>>::new();
        let mut cursor: Option<String> = None;
        loop {
            let response = self
                .host
                .request(
                    "thread/items/list",
                    json!({
                        "threadId": thread_id,
                        "cursor": cursor,
                        "limit": 100,
                        "sortDirection": "asc",
                    }),
                )
                .await?;
            let page = response
                .get("data")
                .and_then(Value::as_array)
                .ok_or_else(|| AdapterError::Rpc("thread/items/list omitted data".to_string()))?;
            for entry in page {
                let turn_id = entry.get("turnId").and_then(Value::as_str).ok_or_else(|| {
                    AdapterError::Rpc("thread/items/list entry omitted turnId".to_string())
                })?;
                let item = entry.get("item").cloned().ok_or_else(|| {
                    AdapterError::Rpc("thread/items/list entry omitted item".to_string())
                })?;
                items_by_turn
                    .entry(turn_id.to_string())
                    .or_default()
                    .push(item);
            }
            let next_cursor = response
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_string);
            if next_cursor.is_none() {
                return Ok(items_by_turn);
            }
            if next_cursor == cursor {
                return Err(AdapterError::Rpc(
                    "thread/items/list returned a non-advancing cursor".to_string(),
                ));
            }
            cursor = next_cursor;
        }
    }

    async fn list_paginated_thread_turns(
        &self,
        thread_id: &str,
    ) -> Result<Vec<Value>, AdapterError> {
        // The official app-server marks `itemsView: full` as a compatibility
        // path and hydrates every Turn serially. Read the two indexed streams
        // concurrently and join them by the protocol's stable turnId instead.
        let (mut turns, mut items_by_turn) = tokio::try_join!(
            self.list_paginated_turn_shells(thread_id),
            self.list_paginated_thread_items(thread_id),
        )?;
        for turn in &mut turns {
            let Some(turn_id) = turn.get("id").and_then(Value::as_str).map(str::to_string) else {
                return Err(AdapterError::Rpc(
                    "thread/turns/list entry omitted id".to_string(),
                ));
            };
            let Some(turn) = turn.as_object_mut() else {
                return Err(AdapterError::Rpc(
                    "thread/turns/list entry was not an object".to_string(),
                ));
            };
            turn.insert(
                "items".to_string(),
                Value::Array(items_by_turn.remove(&turn_id).unwrap_or_default()),
            );
        }
        Ok(turns)
    }

    async fn send_user_message_in_workspace(
        &self,
        workspace: &AuthorizedWorkspace,
        thread_id: &str,
        text: &str,
        options: &TurnOptions,
    ) -> Result<Value, AdapterError> {
        if thread_id.trim().is_empty() || (text.trim().is_empty() && options.images.is_empty()) {
            return Err(AdapterError::Internal(
                "Thread id and message text or image input are required".to_string(),
            ));
        }
        let (workspace_root, _runtime) = self.ensure_thread_bound(workspace, thread_id).await?;

        let mut input = Vec::new();
        if !text.trim().is_empty() {
            input.push(json!({ "type": "text", "text": text.trim() }));
        }
        for image in &options.images {
            if !(image.starts_with("data:")
                || image.starts_with("https://")
                || image.starts_with("http://"))
            {
                return Err(AdapterError::Internal(
                    "image input must be an embedded or remote URL".to_string(),
                ));
            }
            input.push(json!({ "type": "image", "url": image }));
        }
        let read_only = options.access_mode.as_deref() == Some("read-only");
        let mut params = json!({
            "threadId": thread_id,
            "input": input,
            "cwd": &workspace_root,
            "approvalPolicy": "on-request",
            "sandboxPolicy": turn_sandbox_policy(Path::new(&workspace_root), read_only),
        });
        let object = params
            .as_object_mut()
            .expect("turn/start params are an object");
        if let Some(model) = options
            .model
            .as_ref()
            .filter(|value| !value.trim().is_empty())
        {
            object.insert("model".to_string(), json!(model));
        }
        if let Some(model_provider) = options
            .model_provider
            .as_ref()
            .filter(|value| !value.trim().is_empty())
        {
            object.insert("modelProvider".to_string(), json!(model_provider));
        }
        if let Some(effort) = options
            .effort
            .as_ref()
            .filter(|value| !value.trim().is_empty())
        {
            object.insert("effort".to_string(), json!(effort));
        }
        if let Some(service_tier) = options
            .service_tier
            .as_ref()
            .filter(|value| !value.trim().is_empty())
        {
            object.insert("serviceTier".to_string(), json!(service_tier));
        }
        if let Some(collaboration_mode) = &options.collaboration_mode {
            object.insert("collaborationMode".to_string(), collaboration_mode.clone());
        }
        let result = self.host.request("turn/start", params).await?;
        Ok(json!({
            "status": "sent",
            "turnId": result.pointer("/turn/id").cloned().unwrap_or(Value::Null),
        }))
    }

    async fn steer_turn_in_workspace(
        &self,
        workspace: &AuthorizedWorkspace,
        thread_id: &str,
        turn_id: &str,
        text: &str,
        images: &[String],
    ) -> Result<Value, AdapterError> {
        if thread_id.trim().is_empty() || turn_id.trim().is_empty() || text.trim().is_empty() {
            return Err(AdapterError::Internal(
                "Thread id, Turn id and message text are required".to_string(),
            ));
        }
        let (_workspace_root, _runtime) = self.ensure_thread_bound(workspace, thread_id).await?;
        let mut input = vec![json!({ "type": "text", "text": text.trim() })];
        for image in images {
            if !(image.starts_with("data:")
                || image.starts_with("https://")
                || image.starts_with("http://"))
            {
                return Err(AdapterError::Internal(
                    "image input must be an embedded or remote URL".to_string(),
                ));
            }
            input.push(json!({ "type": "image", "url": image }));
        }
        self.host
            .request(
                "turn/steer",
                json!({
                    "threadId": thread_id,
                    "expectedTurnId": turn_id,
                    "input": input,
                }),
            )
            .await
            .map_err(Into::into)
    }

    async fn interrupt_turn_in_workspace(
        &self,
        workspace: &AuthorizedWorkspace,
        thread_id: &str,
        turn_id: &str,
    ) -> Result<(), AdapterError> {
        if thread_id.trim().is_empty() || turn_id.trim().is_empty() {
            return Err(AdapterError::Internal(
                "Thread id and Turn id are required".to_string(),
            ));
        }
        let (_workspace_root, _runtime) = self.ensure_thread_bound(workspace, thread_id).await?;
        self.host
            .request(
                "turn/interrupt",
                json!({ "threadId": thread_id, "turnId": turn_id }),
            )
            .await?;
        Ok(())
    }
}

fn is_authorized_workspace_root(runner_root: &Path, workspace_root: &Path) -> bool {
    workspace_root.starts_with(runner_root)
}

fn login_completion(message: &Value) -> Option<(String, bool, Option<String>)> {
    if message.get("method").and_then(Value::as_str) != Some("account/login/completed") {
        return None;
    }
    let params = message.get("params")?;
    let login_id = params
        .get("loginId")
        .or_else(|| params.get("login_id"))?
        .as_str()?
        .trim();
    if login_id.is_empty() {
        return None;
    }
    let success = params
        .get("success")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let error = params
        .get("error")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    Some((login_id.to_string(), success, error))
}

#[async_trait]
impl CodexAdapter for RealCodexAdapter {
    async fn health(&self) -> Result<HealthStatus, AdapterError> {
        let snapshot = self.host.snapshot().await;
        Ok(HealthStatus {
            ok: snapshot.state == ProfileHostState::Ready,
            version: snapshot
                .server_build
                .unwrap_or_else(|| "unknown".to_string()),
            name: "codex-app-server".to_string(),
        })
    }

    async fn runtime_instance_id(&self) -> uuid::Uuid {
        self.host.runtime_instance_id().await
    }

    async fn rpc(&self, method: &str, params: Value) -> Result<Value, AdapterError> {
        match method {
            "list_workspaces" => Ok(json!([{
                "id": self.workspace_id,
                "name": self.workspace_id,
                "path": self.workspace_root,
                "connected": true,
                "kind": "profile",
            }])),
            "start_thread" => {
                self.require_workspace(&params)?;
                let workspace = AuthorizedWorkspace {
                    id: self.workspace_id.clone(),
                    root: self.workspace_root.clone(),
                };
                let started = self.start_thread_in_workspace(&workspace).await?;
                Ok(json!({ "threadId": started.thread_id }))
            }
            "send_user_message" => {
                self.require_workspace(&params)?;
                let workspace = AuthorizedWorkspace {
                    id: self.workspace_id.clone(),
                    root: self.workspace_root.clone(),
                };
                let thread_id = params.get("threadId").and_then(Value::as_str).unwrap_or_default();
                let text = params.get("text").and_then(Value::as_str).unwrap_or_default();
                self.send_user_message_in_workspace(
                    &workspace,
                    thread_id,
                    text,
                    &TurnOptions::default(),
                )
                .await
            }
            other => Err(AdapterError::NotImplemented(format!(
                "native Profile Host adapter method '{other}' is not available through the transitional RPC interface"
            ))),
        }
    }

    async fn start_thread(
        &self,
        workspace: &AuthorizedWorkspace,
    ) -> Result<StartedThread, AdapterError> {
        self.start_thread_in_workspace(workspace).await
    }

    async fn fork_thread(
        &self,
        source_workspace: &AuthorizedWorkspace,
        target_workspace: &AuthorizedWorkspace,
        thread_id: &str,
    ) -> Result<StartedThread, AdapterError> {
        let (_source_root, _runtime) = self
            .ensure_thread_bound(source_workspace, thread_id)
            .await?;
        let target_root = self.authorized_root(target_workspace)?;
        let result = self
            .host
            .request(
                "thread/fork",
                self.thread_fork_params(thread_id, &target_root),
            )
            .await?;
        let forked_thread_id = result
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .ok_or_else(|| AdapterError::Rpc("thread/fork response omitted thread.id".to_string()))?
            .to_string();
        self.thread_workspaces
            .write()
            .await
            .insert(forked_thread_id.clone(), target_workspace.clone());
        let paginated = result
            .pointer("/thread/historyMode")
            .and_then(Value::as_str)
            .is_some_and(|mode| mode == "paginated");
        self.thread_history_modes
            .write()
            .await
            .insert(forked_thread_id.clone(), paginated);
        Ok(StartedThread {
            thread_id: forked_thread_id,
        })
    }

    async fn read_thread(
        &self,
        workspace: &AuthorizedWorkspace,
        thread_id: &str,
    ) -> Result<Value, AdapterError> {
        let (_workspace_root, _runtime) = self.ensure_thread_bound(workspace, thread_id).await?;
        self.host
            .request(
                "thread/read",
                // Turns are fetched through the paginated thread/turns/list
                // method below. Keeping this metadata read unpaginated avoids
                // thread/read rejecting long histories.
                json!({ "threadId": thread_id, "includeTurns": false }),
            )
            .await
            .map_err(Into::into)
    }

    async fn list_thread_turns(
        &self,
        workspace: &AuthorizedWorkspace,
        thread_id: &str,
    ) -> Result<Vec<Value>, AdapterError> {
        let (_workspace_root, _runtime) = self.ensure_thread_bound(workspace, thread_id).await?;
        if self
            .thread_history_modes
            .read()
            .await
            .get(thread_id)
            .copied()
            .unwrap_or(false)
        {
            return self.list_paginated_thread_turns(thread_id).await;
        }

        // Existing Profile histories created before the platform opted into
        // official paginated storage remain on the Runtime's legacy rollout
        // contract. Keep that compatibility isolated here until those Profile
        // histories are retired.
        let mut turns = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let response = self
                .host
                .request(
                    "thread/turns/list",
                    json!({
                        "threadId": thread_id,
                        "cursor": cursor,
                        "limit": 100,
                        "sortDirection": "asc",
                        "itemsView": "full",
                    }),
                )
                .await?;
            let page = response
                .get("data")
                .and_then(Value::as_array)
                .ok_or_else(|| AdapterError::Rpc("thread/turns/list omitted data".to_string()))?;
            turns.extend(page.iter().cloned());
            let next_cursor = response
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_string);
            if next_cursor.is_none() {
                return Ok(turns);
            }
            if next_cursor == cursor {
                return Err(AdapterError::Rpc(
                    "thread/turns/list returned a non-advancing cursor".to_string(),
                ));
            }
            cursor = next_cursor;
        }
    }

    async fn read_mcp_resource(
        &self,
        workspace: &AuthorizedWorkspace,
        thread_id: &str,
        server: &str,
        uri: &str,
    ) -> Result<Value, AdapterError> {
        let (_workspace_root, _runtime) = self.ensure_thread_bound(workspace, thread_id).await?;
        if server.trim().is_empty() || uri.trim().is_empty() {
            return Err(AdapterError::Internal(
                "MCP Resource server and URI are required".to_string(),
            ));
        }
        self.host
            .request(
                "mcpServer/resource/read",
                json!({
                    "threadId": thread_id,
                    "server": server,
                    "uri": uri,
                }),
            )
            .await
            .map_err(Into::into)
    }

    async fn send_user_message(
        &self,
        workspace: &AuthorizedWorkspace,
        thread_id: &str,
        text: &str,
        options: &TurnOptions,
    ) -> Result<Value, AdapterError> {
        self.send_user_message_in_workspace(workspace, thread_id, text, options)
            .await
    }

    async fn steer_turn(
        &self,
        workspace: &AuthorizedWorkspace,
        thread_id: &str,
        turn_id: &str,
        text: &str,
        images: &[String],
    ) -> Result<Value, AdapterError> {
        self.steer_turn_in_workspace(workspace, thread_id, turn_id, text, images)
            .await
    }

    async fn interrupt_turn(
        &self,
        workspace: &AuthorizedWorkspace,
        thread_id: &str,
        turn_id: &str,
    ) -> Result<(), AdapterError> {
        self.interrupt_turn_in_workspace(workspace, thread_id, turn_id)
            .await
    }

    async fn respond_to_server_request(
        &self,
        runtime_instance_id: uuid::Uuid,
        request_id: Value,
        result: Value,
    ) -> Result<(), AdapterError> {
        self.host
            .respond(runtime_instance_id, request_id, Ok(result))
            .await?;
        Ok(())
    }

    async fn query_profile(&self, query: ProfileQuery) -> Result<Value, AdapterError> {
        let (method, params) = match query {
            ProfileQuery::Account => ("account/read", json!({ "refreshToken": false })),
            ProfileQuery::RateLimits => ("account/rateLimits/read", json!({})),
            ProfileQuery::Usage => ("account/usage/read", json!({})),
            ProfileQuery::CollaborationModes => ("collaborationMode/list", json!({})),
            ProfileQuery::Apps {
                cursor,
                limit,
                thread_id,
            } => (
                "app/list",
                json!({ "cursor": cursor, "limit": limit, "threadId": thread_id }),
            ),
            ProfileQuery::McpServers {
                cursor,
                limit,
                thread_id,
            } => (
                "mcpServerStatus/list",
                json!({ "cursor": cursor, "limit": limit, "threadId": thread_id }),
            ),
            ProfileQuery::ExperimentalFeatures {
                cursor,
                limit,
                thread_id,
            } => (
                "experimentalFeature/list",
                json!({ "cursor": cursor, "limit": limit, "threadId": thread_id }),
            ),
            ProfileQuery::Skills {
                workspace,
                force_reload,
            } => {
                let root = self.authorized_root(&workspace)?;
                (
                    "skills/list",
                    json!({ "cwds": [root], "forceReload": force_reload }),
                )
            }
            ProfileQuery::Config => (
                "config/read",
                json!({ "includeLayers": false, "cwd": null }),
            ),
        };
        self.host.request(method, params).await.map_err(Into::into)
    }

    async fn mutate_profile(&self, mutation: ProfileMutation) -> Result<Value, AdapterError> {
        match mutation {
            ProfileMutation::SetExperimentalFeature { name, enabled } => {
                let mut enablement = serde_json::Map::new();
                enablement.insert(name, json!(enabled));
                self.host
                    .request(
                        "experimentalFeature/enablement/set",
                        json!({ "enablement": enablement }),
                    )
                    .await
                    .map_err(Into::into)
            }
            ProfileMutation::SetAgentCore {
                multi_agent_enabled,
                max_threads,
                max_depth,
            } => {
                self.host
                    .request(
                        "config/batchWrite",
                        json!({
                            "edits": [
                                { "keyPath": "features.multi_agent", "value": multi_agent_enabled, "mergeStrategy": "replace" },
                                { "keyPath": "agents.max_threads", "value": max_threads, "mergeStrategy": "replace" },
                                { "keyPath": "agents.max_depth", "value": max_depth, "mergeStrategy": "replace" }
                            ],
                            "filePath": null,
                            "expectedVersion": null,
                            "reloadUserConfig": true
                        }),
                    )
                    .await
                    .map_err(Into::into)
            }
            ProfileMutation::SetAgentDefinition {
                original_name,
                name,
                description,
                config_file,
            } => {
                let mut definition = serde_json::Map::new();
                if let Some(description) = description {
                    definition.insert("description".to_string(), json!(description));
                }
                definition.insert("config_file".to_string(), json!(config_file));
                let definition = Value::Object(definition);
                if let Some(original_name) = original_name.filter(|value| value != &name) {
                    self.host
                        .request(
                            "config/batchWrite",
                            json!({
                                "edits": [
                                    { "keyPath": format!("agents.{original_name}"), "value": Value::Null, "mergeStrategy": "replace" },
                                    { "keyPath": format!("agents.{name}"), "value": definition, "mergeStrategy": "replace" }
                                ],
                                "filePath": null,
                                "expectedVersion": null,
                                "reloadUserConfig": true
                            }),
                        )
                        .await
                        .map_err(Into::into)
                } else {
                    self.host
                        .request(
                            "config/batchWrite",
                            json!({
                                "edits": [
                                    { "keyPath": format!("agents.{name}"), "value": definition, "mergeStrategy": "replace" }
                                ],
                                "filePath": null,
                                "expectedVersion": null,
                                "reloadUserConfig": true
                            }),
                        )
                        .await
                        .map_err(Into::into)
                }
            }
            ProfileMutation::RemoveAgentDefinition { name } => {
                self.host
                    .request(
                        "config/batchWrite",
                        json!({
                            "edits": [
                                { "keyPath": format!("agents.{name}"), "value": Value::Null, "mergeStrategy": "replace" }
                            ],
                            "filePath": null,
                            "expectedVersion": null,
                            "reloadUserConfig": true
                        }),
                    )
                    .await
                    .map_err(Into::into)
            }
        }
    }

    async fn start_profile_login(&self) -> Result<StartedProfileLogin, AdapterError> {
        let response = self
            .host
            .request(
                "account/login/start",
                json!({
                    "type": "chatgpt",
                    "codexStreamlinedLogin": false,
                    "useHostedLoginSuccessPage": true,
                    "appBrand": "codex"
                }),
            )
            .await?;
        if response.get("type").and_then(Value::as_str) != Some("chatgpt") {
            return Err(AdapterError::Rpc(
                "account/login/start returned an unexpected login type".to_string(),
            ));
        }
        let login_id = response
            .get("loginId")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| AdapterError::Rpc("account/login/start omitted loginId".to_string()))?
            .to_string();
        let auth_url = response
            .get("authUrl")
            .and_then(Value::as_str)
            .filter(|value| value.starts_with("https://") || value.starts_with("http://"))
            .ok_or_else(|| {
                AdapterError::Rpc("account/login/start returned an invalid authUrl".to_string())
            })?
            .to_string();
        *self.active_login_id.write().await = Some(login_id.clone());
        self.login_statuses
            .write()
            .await
            .entry(login_id.clone())
            .or_insert(ProfileLoginStatus {
                completed: false,
                success: None,
                error: None,
            });
        Ok(StartedProfileLogin { login_id, auth_url })
    }

    async fn cancel_profile_login(&self) -> Result<CanceledProfileLogin, AdapterError> {
        let Some(login_id) = self.active_login_id.read().await.clone() else {
            return Ok(CanceledProfileLogin {
                canceled: false,
                status: "notFound".to_string(),
            });
        };
        let response = self
            .host
            .request("account/login/cancel", json!({ "loginId": &login_id }))
            .await?;
        let status = response
            .get("status")
            .and_then(Value::as_str)
            .ok_or_else(|| AdapterError::Rpc("account/login/cancel omitted status".to_string()))?;
        if !matches!(status, "canceled" | "notFound") {
            return Err(AdapterError::Rpc(
                "account/login/cancel returned an unexpected status".to_string(),
            ));
        }
        let mut active = self.active_login_id.write().await;
        if active.as_deref() == Some(login_id.as_str()) {
            *active = None;
        }
        self.login_statuses.write().await.remove(&login_id);
        Ok(CanceledProfileLogin {
            canceled: status == "canceled",
            status: status.to_string(),
        })
    }

    async fn profile_login_status(
        &self,
        login_id: &str,
    ) -> Result<ProfileLoginStatus, AdapterError> {
        let mut statuses = self.login_statuses.write().await;
        let status = statuses
            .get(login_id)
            .cloned()
            .ok_or_else(|| AdapterError::Rpc("Profile login was not found".to_string()))?;
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
        let (_workspace_root, _runtime) = self.ensure_thread_bound(workspace, thread_id).await?;
        self.host
            .request("thread/archive", json!({ "threadId": thread_id }))
            .await?;
        self.thread_workspaces.write().await.remove(thread_id);
        Ok(())
    }

    async fn set_thread_name(
        &self,
        workspace: &AuthorizedWorkspace,
        thread_id: &str,
        name: &str,
    ) -> Result<(), AdapterError> {
        let (_workspace_root, _runtime) = self.ensure_thread_bound(workspace, thread_id).await?;
        self.host
            .request(
                "thread/name/set",
                json!({ "threadId": thread_id, "name": name }),
            )
            .await?;
        Ok(())
    }

    async fn generate_text(
        &self,
        workspace: &AuthorizedWorkspace,
        prompt: &str,
        model: Option<&str>,
    ) -> Result<String, AdapterError> {
        if prompt.trim().is_empty() {
            return Err(AdapterError::Internal(
                "generation prompt is required".to_string(),
            ));
        }
        let mut events = self.host.subscribe();
        let started = self.start_thread_in_workspace(workspace).await?;
        self.suppressed_threads
            .write()
            .await
            .insert(started.thread_id.clone());
        let workspace_root = self.authorized_root(workspace)?;
        let mut params = json!({
            "threadId": &started.thread_id,
            "input": [{ "type": "text", "text": prompt.trim() }],
            "cwd": workspace_root,
            "approvalPolicy": "never",
            "sandboxPolicy": { "type": "readOnly" }
        });
        if let Some(model) = model.filter(|value| !value.trim().is_empty()) {
            params
                .as_object_mut()
                .expect("generation params are an object")
                .insert("model".to_string(), json!(model));
        }
        let thread_id = started.thread_id.clone();
        let collected = async {
            let turn = self.host.request("turn/start", params).await?;
            let turn_id = turn
                .pointer("/turn/id")
                .and_then(Value::as_str)
                .map(str::to_string);
            tokio::time::timeout(std::time::Duration::from_secs(60), async {
                let mut output = String::new();
                loop {
                    let event = events.recv().await.map_err(|error| {
                        AdapterError::Unreachable(format!(
                            "background generation stream closed: {error}"
                        ))
                    })?;
                    let event = event.message;
                    if message_thread_id(&event) != Some(thread_id.as_str()) {
                        continue;
                    }
                    let method = event
                        .get("method")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if method == "item/agentMessage/delta" {
                        if let Some(delta) = event.pointer("/params/delta").and_then(Value::as_str)
                        {
                            output.push_str(delta);
                        }
                    } else if method == "turn/completed" && turn_matches(&event, turn_id.as_deref())
                    {
                        return Ok(output);
                    } else if method == "turn/error" && turn_matches(&event, turn_id.as_deref()) {
                        return Err(AdapterError::Rpc(
                            "background generation failed".to_string(),
                        ));
                    }
                }
            })
            .await
            .map_err(|_| AdapterError::Unreachable("background generation timed out".to_string()))?
        }
        .await;
        let _ = self
            .host
            .request("thread/archive", json!({ "threadId": &started.thread_id }))
            .await;
        self.thread_workspaces
            .write()
            .await
            .remove(&started.thread_id);
        let output = collected?.trim().to_string();
        if output.is_empty() {
            return Err(AdapterError::Rpc(
                "background generation returned no text".to_string(),
            ));
        }
        Ok(output)
    }

    async fn compact_thread(
        &self,
        workspace: &AuthorizedWorkspace,
        thread_id: &str,
    ) -> Result<Value, AdapterError> {
        let (_workspace_root, _runtime) = self.ensure_thread_bound(workspace, thread_id).await?;
        self.host
            .request("thread/compact/start", json!({ "threadId": thread_id }))
            .await
            .map_err(Into::into)
    }

    async fn start_review(
        &self,
        workspace: &AuthorizedWorkspace,
        thread_id: &str,
        target: ReviewTarget,
    ) -> Result<Value, AdapterError> {
        let (_workspace_root, _runtime) = self.ensure_thread_bound(workspace, thread_id).await?;
        let target = match target {
            ReviewTarget::UncommittedChanges => json!({ "type": "uncommittedChanges" }),
            ReviewTarget::BaseBranch { branch } => {
                json!({ "type": "baseBranch", "branch": branch })
            }
            ReviewTarget::Commit { sha, title } => {
                json!({ "type": "commit", "sha": sha, "title": title })
            }
            ReviewTarget::Custom { instructions } => {
                json!({ "type": "custom", "instructions": instructions })
            }
        };
        self.host
            .request(
                "review/start",
                json!({ "threadId": thread_id, "target": target, "delivery": "inline" }),
            )
            .await
            .map_err(Into::into)
    }

    async fn open_terminal(
        &self,
        workspace: &AuthorizedWorkspace,
        process_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), AdapterError> {
        let root = self.authorized_root(workspace)?;
        if process_id.trim().is_empty() || cols == 0 || rows == 0 {
            return Err(AdapterError::Internal(
                "terminal process id and non-zero size are required".to_string(),
            ));
        }
        self.terminal_workspaces
            .write()
            .await
            .insert(process_id.to_string(), workspace.clone());
        let host = self.host.clone();
        let terminal_workspaces = self.terminal_workspaces.clone();
        let local_events = self.local_events.clone();
        let process_id = process_id.to_string();
        let workspace_id = workspace.id.clone();
        tokio::spawn(async move {
            let result = host
                .request_long_running(
                    "command/exec",
                    json!({
                        "command": terminal_command(),
                        "processId": process_id,
                        "tty": true,
                        "streamStdin": true,
                        "streamStdoutStderr": true,
                        "disableOutputCap": true,
                        "disableTimeout": true,
                        "cwd": root,
                        "size": { "cols": cols, "rows": rows },
                        "permissionProfile": ":workspace",
                    }),
                )
                .await;
            terminal_workspaces.write().await.remove(&process_id);
            let (exit_code, error) = match result {
                Ok(value) => (value.get("exitCode").and_then(Value::as_i64), None),
                Err(error) => (None, Some(error.to_string())),
            };
            let _ = local_events.send(json!({
                "method": "platform/terminalExited",
                "params": {
                    "processId": process_id,
                    "workspaceId": workspace_id,
                    "exitCode": exit_code,
                    "failed": error.is_some(),
                }
            }));
        });
        Ok(())
    }

    async fn write_terminal(
        &self,
        workspace: &AuthorizedWorkspace,
        process_id: &str,
        data: &str,
    ) -> Result<(), AdapterError> {
        self.require_terminal(workspace, process_id).await?;
        self.host
            .request(
                "command/exec/write",
                json!({
                    "processId": process_id,
                    "deltaBase64": BASE64.encode(data.as_bytes()),
                }),
            )
            .await?;
        Ok(())
    }

    async fn resize_terminal(
        &self,
        workspace: &AuthorizedWorkspace,
        process_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), AdapterError> {
        self.require_terminal(workspace, process_id).await?;
        if cols == 0 || rows == 0 {
            return Err(AdapterError::Internal(
                "terminal size must be non-zero".to_string(),
            ));
        }
        self.host
            .request(
                "command/exec/resize",
                json!({ "processId": process_id, "size": { "cols": cols, "rows": rows } }),
            )
            .await?;
        Ok(())
    }

    async fn close_terminal(
        &self,
        workspace: &AuthorizedWorkspace,
        process_id: &str,
    ) -> Result<(), AdapterError> {
        self.require_terminal(workspace, process_id).await?;
        self.host
            .request("command/exec/terminate", json!({ "processId": process_id }))
            .await?;
        Ok(())
    }

    async fn subscribe_events(&self, sender: UnboundedSender<Vec<u8>>) -> Result<(), AdapterError> {
        let mut receiver = self.host.subscribe();
        let mut local_receiver = self.local_events.subscribe();
        loop {
            let received = tokio::select! {
                message = receiver.recv() => message.map(|event| (event.runtime_instance_id, event.message)),
                message = local_receiver.recv() => match message {
                    Ok(message) => Ok((self.host.runtime_instance_id().await, message)),
                    Err(error) => Err(error),
                },
            };
            match received {
                Ok((runtime_instance_id, message)) => {
                    if let Some((login_id, success, error)) = login_completion(&message) {
                        self.login_statuses.write().await.insert(
                            login_id.clone(),
                            ProfileLoginStatus {
                                completed: true,
                                success: Some(success),
                                error,
                            },
                        );
                        let mut active = self.active_login_id.write().await;
                        if active.as_deref() == Some(login_id.as_str()) {
                            *active = None;
                        }
                    }
                    if let Some(thread_id) = message_thread_id(&message) {
                        if self.suppressed_threads.read().await.contains(thread_id) {
                            continue;
                        }
                    }
                    let workspace_id = if let Some(workspace_id) = message_workspace_id(&message) {
                        workspace_id.to_string()
                    } else if let Some(process_id) = message_process_id(&message) {
                        self.terminal_workspaces
                            .read()
                            .await
                            .get(process_id)
                            .map(|workspace| workspace.id.as_str())
                            .unwrap_or(&self.workspace_id)
                            .to_string()
                    } else {
                        match message_thread_id(&message) {
                            Some(thread_id) => self
                                .thread_workspaces
                                .read()
                                .await
                                .get(thread_id)
                                .map(|workspace| workspace.id.as_str())
                                .unwrap_or(&self.workspace_id)
                                .to_string(),
                            None => self.workspace_id.clone(),
                        }
                    };
                    let frame =
                        app_server_event_frame(&workspace_id, runtime_instance_id, message)?;
                    if sender.send(frame).is_err() {
                        return Ok(());
                    }
                }
                Err(broadcast::error::RecvError::Lagged(count)) => {
                    let frame = app_server_event_frame(
                        &self.workspace_id,
                        self.host.runtime_instance_id().await,
                        json!({
                            "method": "codex/eventLagged",
                            "params": { "dropped": count },
                        }),
                    )?;
                    if sender.send(frame).is_err() {
                        return Ok(());
                    }
                }
                Err(broadcast::error::RecvError::Closed) => {
                    return Err(AdapterError::Unreachable(
                        "Profile Host event stream closed".to_string(),
                    ));
                }
            }
        }
    }
}

impl RealCodexAdapter {
    async fn require_terminal(
        &self,
        workspace: &AuthorizedWorkspace,
        process_id: &str,
    ) -> Result<(), AdapterError> {
        self.authorized_root(workspace)?;
        let bound = self
            .terminal_workspaces
            .read()
            .await
            .get(process_id)
            .cloned();
        if bound.as_ref() != Some(workspace) {
            return Err(AdapterError::Rpc(
                "terminal is not bound to the authorized workspace".to_string(),
            ));
        }
        Ok(())
    }
}

fn terminal_command() -> Vec<String> {
    #[cfg(windows)]
    {
        vec!["powershell.exe".to_string(), "-NoLogo".to_string()]
    }
    #[cfg(not(windows))]
    {
        vec![
            std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string()),
            "-l".to_string(),
        ]
    }
}

fn message_thread_id(message: &Value) -> Option<&str> {
    message
        .pointer("/params/threadId")
        .or_else(|| message.pointer("/params/thread_id"))
        .or_else(|| message.pointer("/params/thread/id"))
        .and_then(Value::as_str)
}

fn message_process_id(message: &Value) -> Option<&str> {
    message.pointer("/params/processId").and_then(Value::as_str)
}

fn message_workspace_id(message: &Value) -> Option<&str> {
    message
        .pointer("/params/workspaceId")
        .and_then(Value::as_str)
}

const CAPABILITY_ROOTS_ENV: &str = "OPEN_WEB_CODEX_CAPABILITY_ROOTS";
const WORKSPACE_ENVIRONMENT_ID: &str = "local";

fn add_selected_capability_roots(params: &mut Value, workspace_root: &Path) {
    let Ok(process_cwd) = std::env::current_dir() else {
        return;
    };
    let env_value = std::env::var_os(CAPABILITY_ROOTS_ENV);
    let selected = selected_capability_roots_json(
        workspace_root,
        &process_cwd,
        source_repo_root(),
        env_value.as_ref().map(std::ffi::OsString::as_os_str),
    );
    if !selected.is_empty() {
        params["selectedCapabilityRoots"] = Value::Array(selected);
    }
}

fn selected_capability_roots_json(
    workspace_root: &Path,
    process_cwd: &Path,
    source_root: Option<&Path>,
    env_value: Option<&std::ffi::OsStr>,
) -> Vec<Value> {
    discover_selected_capability_root_paths(workspace_root, process_cwd, source_root, env_value)
        .into_iter()
        .map(|root| selected_capability_root_json(&root))
        .collect()
}

fn discover_selected_capability_root_paths(
    workspace_root: &Path,
    process_cwd: &Path,
    source_root: Option<&Path>,
    env_value: Option<&std::ffi::OsStr>,
) -> Vec<std::path::PathBuf> {
    let mut candidates = Vec::new();
    if let Some(value) = env_value {
        candidates.extend(std::env::split_paths(value));
    }
    candidates.extend(plugin_roots_below_tools(process_cwd));
    if workspace_root != process_cwd {
        candidates.extend(plugin_roots_below_tools(workspace_root));
    }
    if let Some(source_root) = source_root
        .filter(|source_root| *source_root != process_cwd && *source_root != workspace_root)
    {
        candidates.extend(plugin_roots_below_tools(source_root));
        if is_plugin_root(source_root) {
            candidates.push(source_root.to_path_buf());
        }
    }
    if is_plugin_root(process_cwd) {
        candidates.push(process_cwd.to_path_buf());
    }
    if workspace_root != process_cwd && is_plugin_root(workspace_root) {
        candidates.push(workspace_root.to_path_buf());
    }

    let mut roots = candidates
        .into_iter()
        .filter(|path| is_plugin_root(path))
        .filter_map(|path| path.canonicalize().ok())
        .collect::<Vec<_>>();
    roots.sort();
    roots.dedup();
    roots
}

fn source_repo_root() -> Option<&'static Path> {
    option_env!("CARGO_MANIFEST_DIR")
        .map(Path::new)
        .and_then(|manifest_dir| manifest_dir.ancestors().nth(4))
}

fn plugin_roots_below_tools(root: &Path) -> Vec<std::path::PathBuf> {
    let tools = root.join("tools");
    let Ok(entries) = std::fs::read_dir(tools) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| is_plugin_root(path))
        .collect()
}

fn is_plugin_root(path: &Path) -> bool {
    path.join(".codex-plugin").join("plugin.json").is_file()
}

fn selected_capability_root_json(root: &Path) -> Value {
    json!({
        "id": selected_capability_root_id(root),
        "location": {
            "type": "environment",
            "environmentId": WORKSPACE_ENVIRONMENT_ID,
            "path": root.to_string_lossy(),
        },
    })
}

fn selected_capability_root_id(root: &Path) -> String {
    let slug = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("capability")
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    format!(
        "local-{}",
        if slug.is_empty() { "capability" } else { &slug }
    )
}

fn turn_matches(message: &Value, expected: Option<&str>) -> bool {
    expected.is_none_or(|expected| {
        message
            .pointer("/params/turnId")
            .or_else(|| message.pointer("/params/turn/id"))
            .and_then(Value::as_str)
            == Some(expected)
    })
}

fn turn_sandbox_policy(workspace_root: &Path, read_only: bool) -> Value {
    if read_only {
        json!({ "type": "readOnly" })
    } else if codex_sandbox_disabled_by_environment() || codex_bubblewrap_is_unavailable() {
        json!({ "type": "externalSandbox", "networkAccess": "enabled" })
    } else {
        json!({
            "type": "workspaceWrite",
            "writableRoots": [workspace_root],
            "networkAccess": true,
        })
    }
}

fn codex_sandbox_disabled_by_environment() -> bool {
    std::env::var("OPEN_WEB_CODEX_DISABLE_CODEX_SANDBOX")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

fn codex_bubblewrap_is_unavailable() -> bool {
    static UNAVAILABLE: OnceLock<bool> = OnceLock::new();
    *UNAVAILABLE.get_or_init(|| {
        if !cfg!(target_os = "linux") {
            return false;
        }
        match Command::new("bwrap")
            .args([
                "--unshare-user",
                "--uid",
                "0",
                "--gid",
                "0",
                "--ro-bind",
                "/",
                "/",
                "true",
            ])
            .output()
        {
            Ok(output) => !output.status.success(),
            Err(_) => false,
        }
    })
}

fn app_server_event_frame(
    workspace_id: &str,
    runtime_instance_id: uuid::Uuid,
    message: Value,
) -> Result<Vec<u8>, AdapterError> {
    let envelope = json!({
        "method": "app-server-event",
        "params": {
            "workspace_id": workspace_id,
            "runtime_instance_id": runtime_instance_id,
            "message": message,
        },
    });
    let mut frame = b"data: ".to_vec();
    serde_json::to_writer(&mut frame, &envelope)
        .map_err(|error| AdapterError::Internal(format!("failed to encode event: {error}")))?;
    frame.extend_from_slice(b"\n\n");
    Ok(frame)
}

#[cfg(test)]
mod tests {
    use super::{
        app_server_event_frame, discover_selected_capability_root_paths,
        is_authorized_workspace_root, login_completion, message_thread_id,
        selected_capability_root_id, selected_capability_roots_json, turn_sandbox_policy,
    };
    use serde_json::{json, Value};
    use std::path::Path;

    fn create_plugin_root(root: &Path, name: &str) -> std::path::PathBuf {
        let plugin = root.join(name);
        std::fs::create_dir_all(plugin.join(".codex-plugin")).expect("create plugin dir");
        std::fs::write(
            plugin.join(".codex-plugin").join("plugin.json"),
            r#"{"name":"test-plugin","version":"0.1.0"}"#,
        )
        .expect("write plugin manifest");
        plugin
    }

    #[test]
    fn accepts_nested_runner_workspaces_without_prefix_confusion() {
        let runner_root = Path::new("/runner");

        assert!(is_authorized_workspace_root(runner_root, runner_root));
        assert!(is_authorized_workspace_root(
            runner_root,
            Path::new("/runner/workspaces/workspace-1")
        ));
        assert!(!is_authorized_workspace_root(
            runner_root,
            Path::new("/runner-other/workspace-1")
        ));
        assert!(!is_authorized_workspace_root(
            runner_root,
            Path::new("/outside/workspace-1")
        ));
    }

    #[test]
    fn wraps_native_notifications_in_the_existing_internal_event_envelope() {
        let frame = app_server_event_frame(
            "workspace-1",
            uuid::Uuid::nil(),
            json!({
                "method": "thread/started",
                "params": { "thread": { "id": "thread-1" } },
            }),
        )
        .expect("event frame");
        let payload = frame
            .strip_prefix(b"data: ")
            .and_then(|value| value.strip_suffix(b"\n\n"))
            .expect("SSE data frame");
        let value: Value = serde_json::from_slice(payload).expect("valid event JSON");

        assert_eq!(value["params"]["workspace_id"], "workspace-1");
        assert_eq!(value["params"]["message"]["method"], "thread/started");
    }

    #[test]
    fn finds_thread_ids_in_notification_variants() {
        assert_eq!(
            message_thread_id(&json!({"params": {"threadId": "thread-1"}})),
            Some("thread-1")
        );
        assert_eq!(
            message_thread_id(&json!({"params": {"thread": {"id": "thread-2"}}})),
            Some("thread-2")
        );
    }

    #[test]
    fn discovers_checked_in_and_workspace_capability_roots() {
        let temp = tempfile::tempdir().expect("tempdir");
        let process = temp.path().join("repo");
        let workspace = temp.path().join("workspace");
        std::fs::create_dir_all(process.join("tools")).expect("process tools");
        std::fs::create_dir_all(workspace.join("tools")).expect("workspace tools");
        let repo_plugin = create_plugin_root(&process.join("tools"), "maps-mcp");
        let workspace_plugin = create_plugin_root(&workspace.join("tools"), "custom-mcp");
        let explicit_plugin = create_plugin_root(temp.path(), "explicit-mcp");

        let roots = discover_selected_capability_root_paths(
            &workspace,
            &process,
            None,
            Some(explicit_plugin.as_os_str()),
        );

        assert_eq!(roots, {
            let mut expected = vec![
                explicit_plugin.canonicalize().unwrap(),
                repo_plugin.canonicalize().unwrap(),
                workspace_plugin.canonicalize().unwrap(),
            ];
            expected.sort();
            expected
        });
    }

    #[test]
    fn discovers_source_tree_capability_roots_when_process_cwd_is_elsewhere() {
        let temp = tempfile::tempdir().expect("tempdir");
        let process = temp.path().join("other-cwd");
        let workspace = temp.path().join("workspace");
        let source = temp.path().join("source");
        std::fs::create_dir_all(&process).expect("process root");
        std::fs::create_dir_all(&workspace).expect("workspace root");
        std::fs::create_dir_all(source.join("tools")).expect("source tools");
        let source_plugin = create_plugin_root(&source.join("tools"), "maps-mcp");

        let roots =
            discover_selected_capability_root_paths(&workspace, &process, Some(&source), None);

        assert_eq!(roots, vec![source_plugin.canonicalize().unwrap()]);
    }

    #[test]
    fn builds_selected_capability_root_payload_for_thread_start() {
        let temp = tempfile::tempdir().expect("tempdir");
        let process = temp.path().join("repo");
        let workspace = temp.path().join("workspace");
        std::fs::create_dir_all(&process).expect("process root");
        std::fs::create_dir_all(workspace.join("tools")).expect("workspace tools");
        let plugin = create_plugin_root(&workspace.join("tools"), "maps-mcp");

        let selected = selected_capability_roots_json(&workspace, &process, None, None);

        assert_eq!(selected[0]["id"], "local-maps-mcp");
        assert_eq!(
            selected[0]["location"],
            json!({
                "type": "environment",
                "environmentId": "local",
                "path": plugin.canonicalize().unwrap().to_string_lossy(),
            })
        );
    }

    #[test]
    fn selected_capability_root_ids_are_stable_slugs() {
        assert_eq!(
            selected_capability_root_id(Path::new("/tmp/Workspace Maps MCP")),
            "local-workspace-maps-mcp"
        );
    }
    #[test]
    fn builds_workspace_write_turn_sandbox_by_default() {
        let policy = turn_sandbox_policy(Path::new("/runner/workspace"), false);

        assert_eq!(policy["type"], "workspaceWrite");
        assert_eq!(policy["writableRoots"], json!(["/runner/workspace"]));
        assert_eq!(policy["networkAccess"], true);
    }

    #[test]
    fn read_only_turn_sandbox_ignores_external_sandbox_escape_hatch() {
        let policy = turn_sandbox_policy(Path::new("/runner/workspace"), true);

        assert_eq!(policy, json!({ "type": "readOnly" }));
    }

    #[test]
    fn captures_only_typed_login_completion_fields() {
        assert_eq!(
            login_completion(&json!({
                "method": "account/login/completed",
                "params": {
                    "loginId": "login-1",
                    "success": false,
                    "error": " authorization failed ",
                    "authorization": "must-not-be-projected"
                }
            })),
            Some((
                "login-1".to_string(),
                false,
                Some("authorization failed".to_string())
            ))
        );
        assert_eq!(
            login_completion(&json!({"method": "account/updated", "params": {}})),
            None
        );
    }
}
