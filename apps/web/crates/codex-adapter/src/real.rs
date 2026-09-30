use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use open_web_codex_profile_host::{ProfileHost, ProfileHostConfig, ProfileHostState};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
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
    ProfileLoginStatus, ProfileMutation, ProfileQuery, ReviewTarget, RuntimeThreadIdentity,
    RuntimeThreadIdentitySidecar, StartedProfileLogin, StartedThread, TurnOptions,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadSkillConfig {
    pub name: String,
    pub enabled: bool,
    /// The one small, always-loaded Root Skill. Package policy decides which
    /// task Skills remain enabled in this Root Thread's native catalog.
    pub main_prompt: Option<PathBuf>,
}

/// One trusted package-owned Root execution configuration. The Platform
/// selects only its package `id`; paths and Runtime config remain server-owned.
#[derive(Debug, Clone, PartialEq)]
pub struct RootExecutionConfig {
    pub id: String,
    pub skill_config: Vec<ThreadSkillConfig>,
    pub runtime_config: Value,
}

fn thread_start_params(
    workspace_root: &str,
    execution: Option<&RootExecutionConfig>,
) -> Result<Value, AdapterError> {
    let mut params = json!({
        "cwd": workspace_root,
        "approvalPolicy": "on-request",
        "historyMode": "paginated",
    });
    if let Some(execution) = execution {
        params["config"] = root_execution_runtime_config(execution)?;
    }
    Ok(params)
}

fn root_execution_runtime_config(execution: &RootExecutionConfig) -> Result<Value, AdapterError> {
    let mut config = execution.runtime_config.clone();
    let object = config.as_object_mut().ok_or_else(|| {
        AdapterError::Internal(format!(
            "Copilot package '{}' Runtime config is not an object",
            execution.id
        ))
    })?;
    object.insert("skills.include_instructions".to_string(), json!(true));
    object.insert(
        "skills.config".to_string(),
        execution
            .skill_config
            .iter()
            .map(|entry| {
                json!({
                    "name": entry.name.as_str(),
                    "enabled": entry.enabled,
                })
            })
            .collect::<Vec<_>>()
            .into(),
    );
    Ok(config)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RootSkillSelection {
    name: String,
    main_prompt: String,
}

fn resolve_root_skill_selections(
    skill_config: &[ThreadSkillConfig],
) -> Result<Vec<RootSkillSelection>, AdapterError> {
    let roots = skill_config
        .iter()
        .filter(|entry| entry.main_prompt.is_some())
        .collect::<Vec<_>>();
    if roots.len() != 1 {
        return Err(AdapterError::Internal(
            "Copilot package must declare exactly one always-loaded Root Skill".to_string(),
        ));
    }
    roots
        .into_iter()
        .map(|entry| {
            if !entry.enabled {
                return Err(AdapterError::Internal(format!(
                    "Copilot Root Skill '{}' must remain enabled in the native catalog",
                    entry.name
                )));
            }
            let main_prompt = entry.main_prompt.as_ref().ok_or_else(|| {
                AdapterError::Internal(format!(
                    "Copilot Root Skill '{}' omitted its Profile main prompt",
                    entry.name
                ))
            })?;
            let main_prompt = main_prompt.canonicalize().map_err(|error| {
                AdapterError::Internal(format!(
                    "failed to resolve Copilot Root Skill '{}': {error}",
                    entry.name
                ))
            })?;
            if !main_prompt.is_file() {
                return Err(AdapterError::Internal(format!(
                    "Copilot Root Skill '{}' is not a regular file",
                    entry.name
                )));
            }
            Ok(RootSkillSelection {
                name: entry.name.clone(),
                main_prompt: main_prompt.to_string_lossy().into_owned(),
            })
        })
        .collect()
}

#[derive(Debug, Clone)]
struct ResolvedRootExecution {
    config: RootExecutionConfig,
    skill_selections: Vec<RootSkillSelection>,
}

fn thread_fork_params(thread_id: &str, target_root: &str) -> Value {
    json!({
        "threadId": thread_id,
        "cwd": target_root,
        "approvalPolicy": "on-request",
    })
}

fn thread_resume_params(
    thread_id: &str,
    workspace_root: &str,
    execution: Option<&RootExecutionConfig>,
) -> Result<Value, AdapterError> {
    let mut params = json!({
        "threadId": thread_id,
        "cwd": workspace_root,
        "approvalPolicy": "on-request",
        "excludeTurns": true,
    });
    if let Some(execution) = execution {
        params["config"] = root_execution_runtime_config(execution)?;
    }
    Ok(params)
}

fn agent_core_batch_write_params(
    multi_agent_enabled: bool,
    max_threads: u32,
    max_depth: u32,
) -> Value {
    json!({
        "edits": [
            { "keyPath": "features.multi_agent", "value": multi_agent_enabled, "mergeStrategy": "replace" },
            { "keyPath": "agents.max_concurrent_threads_per_session", "value": max_threads, "mergeStrategy": "replace" },
            { "keyPath": "agents.max_depth", "value": max_depth, "mergeStrategy": "replace" }
        ],
        "filePath": null,
        "expectedVersion": null,
        "reloadUserConfig": true
    })
}

/// Adapter backed directly by a native Profile Host and Codex app-server
/// JSONL connection without an intermediate local gateway.
pub struct RealCodexAdapter {
    host: ProfileHost,
    workspace_id: String,
    workspace_root: PathBuf,
    thread_workspaces: Arc<RwLock<HashMap<String, AuthorizedWorkspace>>>,
    child_thread_identities: Arc<RwLock<HashMap<String, RuntimeThreadIdentity>>>,
    thread_history_modes: Arc<RwLock<HashMap<String, bool>>>,
    suppressed_threads: Arc<RwLock<HashSet<String>>>,
    active_login_id: Arc<RwLock<Option<String>>>,
    login_statuses: Arc<RwLock<HashMap<String, ProfileLoginStatus>>>,
    terminal_workspaces: Arc<RwLock<HashMap<String, AuthorizedWorkspace>>>,
    runtime_instance: Arc<Mutex<Option<uuid::Uuid>>>,
    local_events: broadcast::Sender<Value>,
    root_executions: BTreeMap<String, ResolvedRootExecution>,
    default_root_execution_id: Option<String>,
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
        Self::from_host_with_root_skill_config(host, workspace_id, workspace_root, Vec::new())
    }

    pub fn from_host_with_root_skill_config(
        host: ProfileHost,
        workspace_id: impl Into<String>,
        workspace_root: PathBuf,
        root_skill_config: Vec<ThreadSkillConfig>,
    ) -> Result<Self, AdapterError> {
        let root_executions = if root_skill_config.is_empty() {
            Vec::new()
        } else {
            vec![RootExecutionConfig {
                id: "default".to_string(),
                skill_config: root_skill_config,
                runtime_config: json!({}),
            }]
        };
        let default_root_execution_id =
            (!root_executions.is_empty()).then(|| "default".to_string());
        Self::from_host_with_root_executions(
            host,
            workspace_id,
            workspace_root,
            root_executions,
            default_root_execution_id,
        )
    }

    pub fn from_host_with_root_executions(
        host: ProfileHost,
        workspace_id: impl Into<String>,
        workspace_root: PathBuf,
        root_executions: Vec<RootExecutionConfig>,
        default_root_execution_id: Option<String>,
    ) -> Result<Self, AdapterError> {
        let workspace_root = workspace_root.canonicalize().map_err(|error| {
            AdapterError::Internal(format!("failed to resolve workspace root: {error}"))
        })?;
        let mut resolved_executions = BTreeMap::new();
        for execution in root_executions {
            if execution.id.trim().is_empty() {
                return Err(AdapterError::Internal(
                    "Copilot package id is empty".to_string(),
                ));
            }
            let skill_selections = resolve_root_skill_selections(&execution.skill_config)?;
            let id = execution.id.clone();
            if resolved_executions
                .insert(
                    id.clone(),
                    ResolvedRootExecution {
                        config: execution,
                        skill_selections,
                    },
                )
                .is_some()
            {
                return Err(AdapterError::Internal(format!(
                    "duplicate Copilot package '{id}'"
                )));
            }
        }
        if let Some(default_id) = default_root_execution_id.as_deref() {
            if !resolved_executions.contains_key(default_id) {
                return Err(AdapterError::Internal(format!(
                    "default Copilot package '{default_id}' is not configured"
                )));
            }
        }
        let (local_events, _) = broadcast::channel(256);
        Ok(Self {
            host,
            workspace_id: workspace_id.into(),
            workspace_root,
            thread_workspaces: Arc::new(RwLock::new(HashMap::new())),
            child_thread_identities: Arc::new(RwLock::new(HashMap::new())),
            thread_history_modes: Arc::new(RwLock::new(HashMap::new())),
            suppressed_threads: Arc::new(RwLock::new(HashSet::new())),
            active_login_id: Arc::new(RwLock::new(None)),
            login_statuses: Arc::new(RwLock::new(HashMap::new())),
            terminal_workspaces: Arc::new(RwLock::new(HashMap::new())),
            runtime_instance: Arc::new(Mutex::new(None)),
            local_events,
            root_executions: resolved_executions,
            default_root_execution_id,
        })
    }

    fn root_execution(
        &self,
        package_id: Option<&str>,
    ) -> Result<Option<&ResolvedRootExecution>, AdapterError> {
        let selected = package_id.or(self.default_root_execution_id.as_deref());
        let Some(selected) = selected else {
            return Ok(None);
        };
        self.root_executions.get(selected).map(Some).ok_or_else(|| {
            AdapterError::Internal(format!(
                "Copilot package '{selected}' is not configured in this Runtime"
            ))
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

    fn mcp_resource_resume_params(
        thread_id: &str,
        workspace_root: &str,
        execution: Option<&RootExecutionConfig>,
    ) -> Result<Value, AdapterError> {
        let mut params = json!({
            "threadId": thread_id,
            "cwd": workspace_root,
            "excludeTurns": true,
        });
        if let Some(execution) = execution {
            params["config"] = root_execution_runtime_config(execution)?;
        }
        Ok(params)
    }

    async fn prepare_runtime(&self) -> Result<OwnedMutexGuard<Option<uuid::Uuid>>, AdapterError> {
        let mut runtime_instance = self.runtime_instance.clone().lock_owned().await;
        self.host.apply_scheduled_restart().await?;
        let current = self.host.runtime_instance_id().await;
        if runtime_instance.as_ref() != Some(&current) {
            self.thread_workspaces.write().await.clear();
            self.child_thread_identities.write().await.clear();
            self.terminal_workspaces.write().await.clear();
            *runtime_instance = Some(current);
        }
        Ok(runtime_instance)
    }

    async fn start_thread_in_workspace(
        &self,
        workspace: &AuthorizedWorkspace,
        execution: Option<&ResolvedRootExecution>,
    ) -> Result<StartedThread, AdapterError> {
        let _runtime = self.prepare_runtime().await?;
        let workspace_root = self.authorized_root(workspace)?;
        let result = self
            .host
            .request(
                "thread/start",
                thread_start_params(&workspace_root, execution.map(|value| &value.config))?,
            )
            .await?;
        let thread_id = result
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                AdapterError::Rpc("thread/start response omitted thread.id".to_string())
            })?
            .to_string();
        self.thread_workspaces
            .write()
            .await
            .insert(thread_id.clone(), workspace.clone());
        self.thread_history_modes
            .write()
            .await
            .insert(thread_id.clone(), true);
        Ok(StartedThread { thread_id })
    }

    async fn ensure_thread_bound(
        &self,
        workspace: &AuthorizedWorkspace,
        thread_id: &str,
    ) -> Result<(String, OwnedMutexGuard<Option<uuid::Uuid>>), AdapterError> {
        self.ensure_thread_bound_with_execution(workspace, thread_id, None)
            .await
    }

    async fn ensure_thread_bound_with_execution(
        &self,
        workspace: &AuthorizedWorkspace,
        thread_id: &str,
        execution: Option<&RootExecutionConfig>,
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
                thread_resume_params(thread_id, &workspace_root, execution)?,
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

    async fn abandon_bound_unmaterialized_thread(
        &self,
        workspace: &AuthorizedWorkspace,
        thread_id: &str,
    ) -> Result<bool, AdapterError> {
        if thread_id.trim().is_empty() {
            return Err(AdapterError::Internal("Thread id is required".to_string()));
        }
        self.authorized_root(workspace)?;
        let mut runtime = self.runtime_instance.clone().lock_owned().await;
        let current = self.host.runtime_instance_id().await;
        if runtime.as_ref() != Some(&current) {
            self.thread_workspaces.write().await.clear();
            self.child_thread_identities.write().await.clear();
            self.terminal_workspaces.write().await.clear();
            *runtime = Some(current);
            return Ok(false);
        }
        let Some(bound) = self.thread_workspaces.read().await.get(thread_id).cloned() else {
            return Ok(false);
        };
        if bound != *workspace {
            return Err(AdapterError::Rpc(
                "Thread is not bound to the authorized workspace".to_string(),
            ));
        }
        if !self.host.abandon_unmaterialized_thread(thread_id).await {
            return Ok(false);
        }
        self.thread_workspaces.write().await.remove(thread_id);
        self.child_thread_identities.write().await.remove(thread_id);
        self.thread_history_modes.write().await.remove(thread_id);
        Ok(true)
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
        let is_child_thread = self
            .child_thread_identities
            .read()
            .await
            .contains_key(thread_id);
        if !is_child_thread {
            let execution = self.root_execution(options.copilot_package_id.as_deref())?;
            for skill in execution
                .into_iter()
                .flat_map(|execution| execution.skill_selections.iter())
            {
                input.push(json!({
                    "type": "skill",
                    "name": skill.name.as_str(),
                    "path": skill.main_prompt.as_str(),
                }));
            }
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
        // Cancellation must not queue behind the adapter's preparation mutex:
        // history/resource requests hold it across RPCs, and preparation may
        // try to restart the very process whose active Turn we need to stop.
        // Validate current Runtime metadata instead of resuming the Thread.
        let workspace_root = self.authorized_root(workspace)?;
        let runtime_instance_id = self.host.runtime_instance_id().await;
        let metadata = self
            .host
            .request_for_runtime(
                runtime_instance_id,
                "thread/read",
                json!({ "threadId": thread_id, "includeTurns": false }),
            )
            .await?;
        let cwd = metadata
            .pointer("/thread/cwd")
            .and_then(Value::as_str)
            .ok_or_else(|| AdapterError::Rpc("Thread metadata omitted cwd".to_string()))?;
        let cwd = Path::new(cwd).canonicalize().map_err(|error| {
            AdapterError::Internal(format!("failed to resolve Thread cwd: {error}"))
        })?;
        if metadata.pointer("/thread/id").and_then(Value::as_str) != Some(thread_id)
            || !cwd.starts_with(&workspace_root)
        {
            return Err(AdapterError::Rpc(
                "Thread is not bound to the authorized workspace".to_string(),
            ));
        }
        self.host
            .request_for_runtime(
                runtime_instance_id,
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
                let started = self.start_thread_in_workspace(&workspace, None).await?;
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
        copilot_package_id: Option<&str>,
    ) -> Result<StartedThread, AdapterError> {
        let execution = self.root_execution(copilot_package_id)?;
        self.start_thread_in_workspace(workspace, execution).await
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
            .request("thread/fork", thread_fork_params(thread_id, &target_root))
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
        copilot_package_id: Option<&str>,
        thread_id: &str,
        server: &str,
        uri: &str,
    ) -> Result<Value, AdapterError> {
        let execution = self.root_execution(copilot_package_id)?;
        let (workspace_root, _runtime) = self
            .ensure_thread_bound_with_execution(
                workspace,
                thread_id,
                execution.map(|value| &value.config),
            )
            .await?;
        if server.trim().is_empty() || uri.trim().is_empty() {
            return Err(AdapterError::Internal(
                "MCP Resource server and URI are required".to_string(),
            ));
        }
        // Child Threads may be unloaded after their terminal result has been
        // delivered. Rejoin the authoritative persisted Thread through the
        // official app-server lifecycle before asking its Role-scoped MCP
        // runtime to read the provider-owned Resource. This keeps the
        // producing Thread, cwd and MCP inventory authoritative without a
        // Platform-side Resource broker or a second MCP configuration owner.
        self.host
            .request(
                "thread/resume",
                Self::mcp_resource_resume_params(
                    thread_id,
                    &workspace_root,
                    execution.map(|value| &value.config),
                )?,
            )
            .await?;
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
                        agent_core_batch_write_params(
                            multi_agent_enabled,
                            max_threads,
                            max_depth,
                        ),
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
        if self
            .abandon_bound_unmaterialized_thread(workspace, thread_id)
            .await?
        {
            return Ok(());
        }
        let (_workspace_root, _runtime) = self.ensure_thread_bound(workspace, thread_id).await?;
        self.host
            .request("thread/archive", json!({ "threadId": thread_id }))
            .await?;
        self.thread_workspaces.write().await.remove(thread_id);
        self.child_thread_identities.write().await.remove(thread_id);
        self.thread_history_modes.write().await.remove(thread_id);
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
        let started = self.start_thread_in_workspace(workspace, None).await?;
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
                    let thread_identity = self.inherit_child_thread_workspace(&message).await?;
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
                    let frame = app_server_event_frame_with_identity(
                        &workspace_id,
                        runtime_instance_id,
                        message,
                        thread_identity.as_ref(),
                    )?;
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
    async fn inherit_child_thread_workspace(
        &self,
        message: &Value,
    ) -> Result<Option<RuntimeThreadIdentitySidecar>, AdapterError> {
        let Some(child_thread_id) = message_thread_id(message) else {
            return Ok(None);
        };
        let cached_identity = {
            let identities = self.child_thread_identities.read().await;
            cached_thread_identity_sidecar(&identities, child_thread_id)
        };
        if let Some(sidecar) = cached_identity {
            return Ok(Some(sidecar));
        }
        if self
            .thread_workspaces
            .read()
            .await
            .contains_key(child_thread_id)
        {
            return Ok(None);
        }
        match self
            .read_unbound_child_thread_identity(child_thread_id)
            .await
        {
            Ok(Some(identity)) => {
                match self
                    .bind_child_thread_workspace(&identity.thread_id, &identity.parent_thread_id)
                    .await
                {
                    Ok(()) => {
                        self.child_thread_identities
                            .write()
                            .await
                            .insert(child_thread_id.to_string(), identity.clone());
                        Ok(Some(RuntimeThreadIdentitySidecar::Resolved(identity)))
                    }
                    Err(error) => {
                        tracing::warn!(
                            thread_id = child_thread_id,
                            reason = identity_failure_category(&error),
                            "Runtime child Thread workspace binding unavailable"
                        );
                        Ok(Some(RuntimeThreadIdentitySidecar::Unavailable {
                            thread_id: child_thread_id.to_string(),
                        }))
                    }
                }
            }
            Ok(None) if message_parent_thread_id(message).is_some() => {
                Ok(Some(RuntimeThreadIdentitySidecar::Unavailable {
                    thread_id: child_thread_id.to_string(),
                }))
            }
            Ok(None) => Ok(None),
            Err(error) => {
                tracing::warn!(
                    thread_id = child_thread_id,
                    reason = identity_failure_category(&error),
                    "Runtime child Thread identity hydration unavailable"
                );
                Ok(Some(RuntimeThreadIdentitySidecar::Unavailable {
                    thread_id: child_thread_id.to_string(),
                }))
            }
        }
    }

    async fn bind_child_thread_workspace(
        &self,
        child_thread_id: &str,
        parent_thread_id: &str,
    ) -> Result<(), AdapterError> {
        if child_thread_id == parent_thread_id {
            return Err(AdapterError::Rpc(
                "Runtime child Thread referenced itself as parent".to_string(),
            ));
        }
        let parent_workspace = self
            .thread_workspaces
            .read()
            .await
            .get(parent_thread_id)
            .cloned();
        let Some(parent_workspace) = parent_workspace else {
            return Err(AdapterError::Rpc(
                "Runtime child Thread parent is not bound to an authorized Workspace".to_string(),
            ));
        };
        let mut thread_workspaces = self.thread_workspaces.write().await;
        match thread_workspaces.get(child_thread_id) {
            Some(existing) if existing != &parent_workspace => Err(AdapterError::Rpc(
                "Runtime child Thread changed its authorized Workspace".to_string(),
            )),
            Some(_) => Ok(()),
            None => {
                thread_workspaces.insert(child_thread_id.to_string(), parent_workspace);
                Ok(())
            }
        }
    }

    async fn read_unbound_child_thread_identity(
        &self,
        child_thread_id: &str,
    ) -> Result<Option<RuntimeThreadIdentity>, AdapterError> {
        let response = self
            .host
            .request(
                "thread/read",
                json!({ "threadId": child_thread_id, "includeTurns": false }),
            )
            .await?;
        let thread = response
            .get("thread")
            .and_then(Value::as_object)
            .ok_or_else(|| AdapterError::Rpc("Runtime thread/read omitted thread".to_string()))?;
        let returned_thread_id = bounded_map_string(thread, "id", 256).ok_or_else(|| {
            AdapterError::Rpc("Runtime thread/read omitted thread.id".to_string())
        })?;
        if returned_thread_id != child_thread_id {
            return Err(AdapterError::Rpc(
                "Runtime thread/read returned a different Thread".to_string(),
            ));
        }
        let Some(parent_thread_id) = thread_spawn_parent_thread_id(thread, child_thread_id)? else {
            return Ok(None);
        };
        let parent_workspace = self
            .thread_workspaces
            .read()
            .await
            .get(&parent_thread_id)
            .cloned()
            .ok_or_else(|| {
                AdapterError::Rpc(
                    "Runtime child Thread parent is not bound to an authorized Workspace"
                        .to_string(),
                )
            })?;
        let authorized_root = self.authorized_root(&parent_workspace)?;
        parse_runtime_thread_identity(thread, child_thread_id, &authorized_root)
    }

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

fn thread_spawn_parent_thread_id(
    thread: &Map<String, Value>,
    child_thread_id: &str,
) -> Result<Option<String>, AdapterError> {
    let source = thread
        .get("source")
        .and_then(Value::as_object)
        .and_then(|source| source.get("subAgent"))
        .and_then(Value::as_object);
    let thread_spawn =
        source.and_then(|source| source.get("thread_spawn").and_then(Value::as_object));
    let parent_thread_id = first_consistent_string(
        thread,
        &["parentThreadId"],
        thread_spawn,
        &["parent_thread_id"],
        256,
    )?;
    if thread_spawn.is_none() {
        if parent_thread_id.is_none() {
            return Ok(None);
        }
        return Err(AdapterError::Rpc(
            "Runtime Thread parent was not a ThreadSpawn source".to_string(),
        ));
    }
    let Some(parent_thread_id) = parent_thread_id else {
        return Err(AdapterError::Rpc(
            "Runtime ThreadSpawn omitted parentThreadId".to_string(),
        ));
    };
    if parent_thread_id == child_thread_id {
        return Err(AdapterError::Rpc(
            "Runtime child Thread referenced itself as parent".to_string(),
        ));
    }
    Ok(Some(parent_thread_id))
}

fn parse_runtime_thread_identity(
    thread: &Map<String, Value>,
    child_thread_id: &str,
    authorized_root: &str,
) -> Result<Option<RuntimeThreadIdentity>, AdapterError> {
    let Some(parent_thread_id) = thread_spawn_parent_thread_id(thread, child_thread_id)? else {
        return Ok(None);
    };
    let source = thread
        .get("source")
        .and_then(Value::as_object)
        .and_then(|source| source.get("subAgent"))
        .and_then(Value::as_object)
        .ok_or_else(|| AdapterError::Rpc("Runtime Thread source was not a subagent".to_string()))?;
    let thread_spawn = source
        .get("thread_spawn")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            AdapterError::Rpc("Runtime Thread source was not a ThreadSpawn".to_string())
        })?;
    let cwd = bounded_map_string(thread, "cwd", 4_096)
        .ok_or_else(|| AdapterError::Rpc("Runtime Thread omitted cwd".to_string()))?;
    let cwd = Path::new(&cwd).canonicalize().map_err(|error| {
        AdapterError::Rpc(format!(
            "Runtime child Thread cwd could not be resolved: {error}"
        ))
    })?;
    if !cwd.starts_with(Path::new(authorized_root)) {
        return Err(AdapterError::Rpc(
            "Runtime child Thread cwd is outside its authorized Workspace".to_string(),
        ));
    }
    let status = thread
        .get("status")
        .and_then(Value::as_object)
        .ok_or_else(|| AdapterError::Rpc("Runtime Thread omitted status".to_string()))?;
    if bounded_map_string(status, "type", 64).is_none() {
        return Err(AdapterError::Rpc(
            "Runtime Thread status omitted type".to_string(),
        ));
    }
    let agent_role = first_consistent_string(
        thread,
        &["agentRole"],
        Some(thread_spawn),
        &["agent_role"],
        128,
    )?;
    let agent_nickname = first_consistent_string(
        thread,
        &["agentNickname"],
        Some(thread_spawn),
        &["agent_nickname"],
        128,
    )?;
    let agent_path = bounded_map_string(thread_spawn, "agent_path", 512);
    Ok(Some(RuntimeThreadIdentity {
        thread_id: child_thread_id.to_string(),
        parent_thread_id,
        source_kind: "thread_spawn".to_string(),
        agent_path,
        agent_nickname,
        agent_role,
    }))
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

fn message_parent_thread_id(message: &Value) -> Option<&str> {
    message
        .pointer("/params/thread/parentThreadId")
        .or_else(|| message.pointer("/params/thread/parent_thread_id"))
        .or_else(|| message.pointer("/params/thread/source/subAgent/thread_spawn/parent_thread_id"))
        .or_else(|| message.pointer("/params/thread/source/subAgent/threadSpawn/parentThreadId"))
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

fn bounded_map_string(values: &Map<String, Value>, key: &str, max_len: usize) -> Option<String> {
    values
        .get(key)
        .and_then(Value::as_str)
        .and_then(|value| bounded_text(value, max_len))
}

fn bounded_text(value: &str, max_len: usize) -> Option<String> {
    let value = value.trim();
    (!value.is_empty() && value.len() <= max_len).then(|| value.to_string())
}

fn cached_thread_identity_sidecar(
    identities: &HashMap<String, RuntimeThreadIdentity>,
    thread_id: &str,
) -> Option<RuntimeThreadIdentitySidecar> {
    identities
        .get(thread_id)
        .cloned()
        .map(RuntimeThreadIdentitySidecar::Resolved)
}

fn first_consistent_string(
    primary: &Map<String, Value>,
    primary_keys: &[&str],
    secondary: Option<&Map<String, Value>>,
    secondary_keys: &[&str],
    max_len: usize,
) -> Result<Option<String>, AdapterError> {
    let mut values = Vec::new();
    for key in primary_keys {
        if let Some(value) = bounded_map_string(primary, key, max_len) {
            if !values.iter().any(|existing| existing == &value) {
                values.push(value);
            }
        }
    }
    if let Some(values_map) = secondary {
        for key in secondary_keys {
            if let Some(value) = bounded_map_string(values_map, key, max_len) {
                if !values.iter().any(|existing| existing == &value) {
                    values.push(value);
                }
            }
        }
    }
    match values.as_slice() {
        [] => Ok(None),
        [value] => Ok(Some(value.clone())),
        _ => Err(AdapterError::Rpc(
            "Runtime Thread identity metadata conflicted".to_string(),
        )),
    }
}

fn identity_failure_category(error: &AdapterError) -> &'static str {
    match error {
        AdapterError::Rpc(_) => "rpc",
        AdapterError::Unreachable(_) => "unreachable",
        AdapterError::ProfileHost(_) => "profile_host",
        AdapterError::Internal(_) => "internal",
        AdapterError::NotImplemented(_) => "unsupported",
        AdapterError::CapabilityUnavailable(_) => "capability",
    }
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
    app_server_event_frame_with_identity(workspace_id, runtime_instance_id, message, None)
}

fn app_server_event_frame_with_identity(
    workspace_id: &str,
    runtime_instance_id: uuid::Uuid,
    message: Value,
    thread_identity: Option<&RuntimeThreadIdentitySidecar>,
) -> Result<Vec<u8>, AdapterError> {
    let mut params = json!({
        "workspace_id": workspace_id,
        "runtime_instance_id": runtime_instance_id,
        "message": message,
    });
    if let Some(thread_identity) = thread_identity {
        params["thread_identity"] = serde_json::to_value(thread_identity).map_err(|error| {
            AdapterError::Internal(format!("failed to encode Thread identity: {error}"))
        })?;
    }
    let envelope = json!({
        "method": "app-server-event",
        "params": params,
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
        agent_core_batch_write_params, app_server_event_frame,
        app_server_event_frame_with_identity, cached_thread_identity_sidecar,
        codex_bubblewrap_is_unavailable, codex_sandbox_disabled_by_environment,
        is_authorized_workspace_root, login_completion, message_parent_thread_id,
        message_thread_id, parse_runtime_thread_identity, resolve_root_skill_selections,
        thread_resume_params, thread_spawn_parent_thread_id, thread_start_params,
        turn_sandbox_policy, RealCodexAdapter, RootExecutionConfig, ThreadSkillConfig,
    };
    use crate::{AdapterError, RuntimeThreadIdentity, RuntimeThreadIdentitySidecar};
    use serde_json::{json, Value};
    use std::collections::HashMap;
    use std::path::Path;

    #[cfg(unix)]
    #[tokio::test]
    async fn interrupt_bypasses_pending_reads_and_scheduled_restart_without_resuming() {
        use crate::{AuthorizedWorkspace, CodexAdapter};
        use open_web_codex_profile_host::{ProfileHost, ProfileHostConfig};
        use std::time::Duration;
        use tokio::time::timeout;

        let temp = tempfile::tempdir().expect("temporary profile");
        let root = temp.path().canonicalize().expect("workspace root");
        let mut config = ProfileHostConfig::new("interrupt-test", root.join("home"), &root)
            .with_codex_bin("python3")
            .with_environment("TEST_WORKSPACE", root.as_os_str());
        config.codex_args.push(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/interrupt_server.py")
                .into_os_string(),
        );
        let host = ProfileHost::spawn(config.clone()).await.expect("test host");
        let adapter = std::sync::Arc::new(
            RealCodexAdapter::from_host(host.clone(), "workspace", root.clone()).expect("adapter"),
        );
        let workspace = AuthorizedWorkspace {
            id: "workspace".into(),
            root,
        };
        // Bind the blocked read to this process, just as a preceding start does.
        drop(adapter.prepare_runtime().await.expect("prepare"));
        adapter
            .thread_workspaces
            .write()
            .await
            .insert("busy-thread".into(), workspace.clone());
        let mut events = host.subscribe();
        let reader = {
            let adapter = adapter.clone();
            let workspace = workspace.clone();
            tokio::spawn(async move { adapter.read_thread(&workspace, "busy-thread").await })
        };
        timeout(Duration::from_secs(2), async {
            loop {
                if events.recv().await.expect("event").message["method"] == "test/readBlocked" {
                    break;
                }
            }
        })
        .await
        .expect("read reached Runtime and holds adapter mutex");
        host.schedule_restart(config).await.expect("schedule restart");
        let instance = host.runtime_instance_id().await;
        timeout(
            Duration::from_secs(2),
            adapter.interrupt_turn(&workspace, "thread-1", "turn-1"),
        )
        .await
        .expect("interrupt must not wait for read")
        .expect("interrupt acknowledged");
        assert_eq!(
            instance,
            host.runtime_instance_id().await,
            "stop must not restart Runtime"
        );
        reader.await.expect("reader task").expect("read released by interrupt");

        for thread in ["other-workspace", "missing-thread"] {
            adapter
                .interrupt_turn(&workspace, thread, "turn-1")
                .await
                .expect_err("unauthorized or missing Thread cannot be interrupted");
        }
        host.shutdown().await.expect("shutdown");
    }

    #[test]
    fn thread_start_params_omit_skill_config_by_default() {
        let params = thread_start_params("/runner/workspace", None).expect("start params");

        assert_eq!(
            params,
            json!({
                "cwd": "/runner/workspace",
                "approvalPolicy": "on-request",
                "historyMode": "paginated",
            })
        );
        assert!(params.get("config").is_none());
    }

    #[test]
    fn thread_start_params_projects_root_scoped_skill_config_in_order() {
        let execution = RootExecutionConfig {
            id: "multi-agent".to_string(),
            skill_config: vec![
                ThreadSkillConfig {
                    name: "warehouse-supervisor".to_string(),
                    enabled: true,
                    main_prompt: Some("/profile/skills/warehouse-supervisor/SKILL.md".into()),
                },
                ThreadSkillConfig {
                    name: "warehouse-data".to_string(),
                    enabled: false,
                    main_prompt: None,
                },
                ThreadSkillConfig {
                    name: "warehouse-network-planning".to_string(),
                    enabled: false,
                    main_prompt: None,
                },
            ],
            runtime_config: json!({}),
        };
        let params =
            thread_start_params("/runner/workspace", Some(&execution)).expect("start params");

        assert_eq!(
            params,
            json!({
                "cwd": "/runner/workspace",
                "approvalPolicy": "on-request",
                "historyMode": "paginated",
                "config": {
                    "skills.include_instructions": true,
                    "skills.config": [
                        { "name": "warehouse-supervisor", "enabled": true },
                        { "name": "warehouse-data", "enabled": false },
                        { "name": "warehouse-network-planning", "enabled": false },
                    ],
                },
            })
        );
    }

    #[test]
    fn root_skill_selections_require_existing_profile_prompts() {
        let profile = tempfile::tempdir().expect("Profile root");
        let prompt = profile.path().join("skills/supervisor/SKILL.md");
        std::fs::create_dir_all(prompt.parent().expect("Skill directory"))
            .expect("create Skill directory");
        std::fs::write(
            &prompt,
            "---\nname: supervisor\ndescription: test\n---\nbody",
        )
        .expect("write Skill prompt");

        let selected = resolve_root_skill_selections(&[ThreadSkillConfig {
            name: "supervisor".to_string(),
            enabled: true,
            main_prompt: Some(prompt.clone()),
        }])
        .expect("resolve fixed Root Skill");
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].name, "supervisor");
        assert_eq!(
            selected[0].main_prompt,
            prompt
                .canonicalize()
                .expect("canonical Skill prompt")
                .to_string_lossy()
        );

        let error = resolve_root_skill_selections(&[
            ThreadSkillConfig {
                name: "first".to_string(),
                enabled: true,
                main_prompt: Some(prompt.clone()),
            },
            ThreadSkillConfig {
                name: "second".to_string(),
                enabled: true,
                main_prompt: Some(prompt.clone()),
            },
        ])
        .expect_err("multiple always-loaded Root Skills must be rejected");
        assert!(
            matches!(error, AdapterError::Internal(message) if message.contains("exactly one always-loaded Root Skill"))
        );

        let error = resolve_root_skill_selections(&[ThreadSkillConfig {
            name: "disabled".to_string(),
            enabled: false,
            main_prompt: Some(prompt),
        }])
        .expect_err("the always-loaded Root Skill must remain catalog-enabled");
        assert!(
            matches!(error, AdapterError::Internal(message) if message.contains("must remain enabled"))
        );
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
    fn wraps_runtime_identity_as_internal_sidecar_without_mutating_message() {
        let message = json!({
            "method": "item/completed",
            "params": {"threadId": "child-thread", "itemId": "item-1"}
        });
        let identity = RuntimeThreadIdentitySidecar::Resolved(RuntimeThreadIdentity {
            thread_id: "child-thread".to_string(),
            parent_thread_id: "root-thread".to_string(),
            source_kind: "thread_spawn".to_string(),
            agent_path: Some("/root/network".to_string()),
            agent_nickname: Some("Network".to_string()),
            agent_role: Some("network_agent".to_string()),
        });
        let frame = app_server_event_frame_with_identity(
            "workspace-1",
            uuid::Uuid::nil(),
            message.clone(),
            Some(&identity),
        )
        .expect("event frame");
        let payload = frame
            .strip_prefix(b"data: ")
            .and_then(|value| value.strip_suffix(b"\n\n"))
            .expect("SSE data frame");
        let value: Value = serde_json::from_slice(payload).expect("valid event JSON");
        assert_eq!(value["params"]["message"], message);
        assert_eq!(
            value["params"]["thread_identity"]["Resolved"]["thread_id"],
            "child-thread"
        );
    }

    #[test]
    fn parses_only_authoritative_thread_spawn_identity_shape() {
        let root = tempfile::tempdir().expect("workspace root");
        let child_cwd = root.path().join("child");
        std::fs::create_dir(&child_cwd).expect("child cwd");
        let canonical_root = root.path().canonicalize().expect("canonical root");
        let thread = json!({
            "id": "child-thread",
            "parentThreadId": "root-thread",
            "agentRole": "network_agent",
            "agentNickname": "Network",
            "cwd": child_cwd,
            "source": {"subAgent": {"thread_spawn": {
                "parent_thread_id": "root-thread",
                "depth": 1,
                "agent_role": "network_agent",
                "agent_nickname": "Network",
                "agent_path": "/root/network"
            }}},
            "status": {"type": "idle", "activeFlags": []}
        });
        let thread = thread.as_object().expect("thread object");
        let identity = parse_runtime_thread_identity(
            thread,
            "child-thread",
            canonical_root.to_str().expect("root path"),
        )
        .expect("authoritative shape");
        assert_eq!(
            identity
                .as_ref()
                .and_then(|identity| identity.agent_role.as_deref()),
            Some("network_agent")
        );
        assert_eq!(
            thread_spawn_parent_thread_id(thread, "child-thread").unwrap(),
            Some("root-thread".to_string())
        );
    }

    #[test]
    fn rejects_thread_identity_conflict_self_parent_non_spawn_and_cwd_escape() {
        let root = tempfile::tempdir().expect("workspace root");
        let child_cwd = root.path().join("child");
        std::fs::create_dir(&child_cwd).expect("child cwd");
        let canonical_root = root.path().canonicalize().expect("canonical root");
        let base = json!({
            "id": "child-thread",
            "parentThreadId": "root-thread",
            "agentRole": "network_agent",
            "cwd": child_cwd,
            "source": {"subAgent": {"thread_spawn": {
                "parent_thread_id": "root-thread",
                "depth": 1,
                "agent_role": "network_agent"
            }}},
            "status": {"type": "idle", "activeFlags": []}
        });
        let mut conflict = base.clone();
        conflict["source"]["subAgent"]["thread_spawn"]["agent_role"] = json!("data_planning_agent");
        let conflict = conflict.as_object().expect("conflict object");
        assert!(parse_runtime_thread_identity(
            conflict,
            "child-thread",
            canonical_root.to_str().expect("root path")
        )
        .is_err());

        let mut self_parent = base.clone();
        self_parent["parentThreadId"] = json!("child-thread");
        let self_parent = self_parent.as_object().expect("self-parent object");
        assert!(parse_runtime_thread_identity(
            self_parent,
            "child-thread",
            canonical_root.to_str().expect("root path")
        )
        .is_err());

        let mut non_spawn = base.clone();
        non_spawn["source"]["subAgent"] = json!("review");
        let non_spawn = non_spawn.as_object().expect("non-spawn object");
        assert!(parse_runtime_thread_identity(
            non_spawn,
            "child-thread",
            canonical_root.to_str().expect("root path")
        )
        .is_err());

        let outside = tempfile::tempdir().expect("outside root");
        let mut escaped = base;
        escaped["cwd"] = json!(outside.path());
        let escaped = escaped.as_object().expect("escaped object");
        assert!(parse_runtime_thread_identity(
            escaped,
            "child-thread",
            canonical_root.to_str().expect("root path")
        )
        .is_err());
    }

    #[test]
    fn reuses_cached_child_identity_until_runtime_state_is_cleared() {
        let identity = RuntimeThreadIdentity {
            thread_id: "child-thread".to_string(),
            parent_thread_id: "root-thread".to_string(),
            source_kind: "thread_spawn".to_string(),
            agent_path: Some("/root/network".to_string()),
            agent_nickname: Some("Network".to_string()),
            agent_role: Some("network_agent".to_string()),
        };
        let mut identities = HashMap::new();
        identities.insert(identity.thread_id.clone(), identity.clone());

        assert_eq!(
            cached_thread_identity_sidecar(&identities, "child-thread"),
            Some(RuntimeThreadIdentitySidecar::Resolved(identity))
        );
        identities.clear();
        assert_eq!(
            cached_thread_identity_sidecar(&identities, "child-thread"),
            None
        );
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
        assert_eq!(
            message_parent_thread_id(&json!({
                "params": {
                    "thread": {
                        "id": "thread-2",
                        "parentThreadId": "thread-1"
                    }
                }
            })),
            Some("thread-1")
        );
        assert_eq!(
            message_parent_thread_id(&json!({
                "params": {
                    "thread": {
                        "source": {
                            "subAgent": {
                                "thread_spawn": {
                                    "parent_thread_id": "thread-1"
                                }
                            }
                        }
                    }
                }
            })),
            Some("thread-1")
        );
    }

    #[test]
    fn writes_the_canonical_runtime_agent_concurrency_key() {
        let params = agent_core_batch_write_params(true, 2, 1);

        assert_eq!(
            params["edits"][1]["keyPath"],
            "agents.max_concurrent_threads_per_session"
        );
        assert!(params
            .to_string()
            .contains("agents.max_concurrent_threads_per_session"));
        assert!(!params.to_string().contains("agents.max_threads"));
    }

    #[test]
    fn selects_the_available_turn_sandbox_policy() {
        let policy = turn_sandbox_policy(Path::new("/runner/workspace"), false);

        if codex_sandbox_disabled_by_environment() || codex_bubblewrap_is_unavailable() {
            assert_eq!(policy["type"], "externalSandbox");
            assert_eq!(policy["networkAccess"], "enabled");
        } else {
            assert_eq!(policy["type"], "workspaceWrite");
            assert_eq!(policy["writableRoots"], json!(["/runner/workspace"]));
            assert_eq!(policy["networkAccess"], true);
        }
    }

    #[test]
    fn read_only_turn_sandbox_ignores_external_sandbox_escape_hatch() {
        let policy = turn_sandbox_policy(Path::new("/runner/workspace"), true);

        assert_eq!(policy, json!({ "type": "readOnly" }));
    }

    #[test]
    fn mcp_resource_resume_rejoins_with_selected_copilot_config_without_thread_policy() {
        let execution = RootExecutionConfig {
            id: "warehouse-network-copilot".to_string(),
            skill_config: Vec::new(),
            runtime_config: json!({
                "agents.network_agent.runtime_mcp_projection": true,
            }),
        };
        let value = RealCodexAdapter::mcp_resource_resume_params(
            "thread-1",
            "/runner/workspace",
            Some(&execution),
        )
        .expect("resume params");

        assert_eq!(value["threadId"], "thread-1");
        assert_eq!(value["cwd"], "/runner/workspace");
        assert_eq!(value["excludeTurns"], true);
        assert_eq!(
            value["config"]["agents.network_agent.runtime_mcp_projection"],
            true
        );
        assert_eq!(value["config"]["skills.include_instructions"], true);
        assert!(value.get("approvalPolicy").is_none());

        let first_resume = thread_resume_params("thread-1", "/runner/workspace", Some(&execution))
            .expect("first cold resume params");
        assert_eq!(
            first_resume["config"]["agents.network_agent.runtime_mcp_projection"],
            true
        );
        assert_eq!(first_resume["approvalPolicy"], "on-request");
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
