//! Persistent Codex Profile lifecycle and native app-server transport.
//!
//! The host owns the process and protocol connection for one persistent
//! `CODEX_HOME`. Product authorization, workspace provisioning and browser
//! projections remain platform responsibilities.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use open_web_codex_codex_contracts::{
    negotiate_capability_manifest, CapabilityManifest, NegotiationPolicy, NegotiationResult,
};
use serde_json::{json, Value};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{broadcast, oneshot, Mutex, RwLock};
use tokio::time::timeout;
use uuid::Uuid;

const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(300);
const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(15);
const DEFAULT_EVENT_CAPACITY: usize = 1_024;
const RUNTIME_DIRECTORY: &str = ".open-web-codex";
const LOCK_FILE: &str = "app-server.lock";

/// Creates a missing Profile home and returns its canonical directory path.
///
/// The Host must call this before spawning Codex. Codex itself intentionally
/// treats a configured but missing `CODEX_HOME` as invalid.
pub fn ensure_profile_home(path: &Path) -> io::Result<PathBuf> {
    match fs::metadata(path) {
        Ok(metadata) if !metadata.is_dir() => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Profile home {} is not a directory", path.display()),
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            create_private_directory(path)?;
        }
        Err(error) => return Err(error),
    }

    let canonical_path = path.canonicalize()?;
    if !canonical_path.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Profile home {} is not a directory", path.display()),
        ));
    }
    restrict_directory_permissions(&canonical_path)?;
    Ok(canonical_path)
}

fn ensure_profile_layout(path: &Path) -> io::Result<(PathBuf, PathBuf)> {
    let home = ensure_profile_home(path)?;
    let runtime = home.join(RUNTIME_DIRECTORY);
    if !runtime.exists() {
        create_private_directory(&runtime)?;
    }
    let runtime = runtime.canonicalize()?;
    restrict_directory_permissions(&runtime)?;
    Ok((home, runtime))
}

fn create_private_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true).mode(0o700).create(path)?;
    }
    #[cfg(not(unix))]
    fs::create_dir_all(path)?;
    Ok(())
}

fn restrict_directory_permissions(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Configuration for one persistent Profile app-server.
#[derive(Clone)]
pub struct ProfileHostConfig {
    pub profile_id: String,
    pub codex_home: PathBuf,
    pub workspace_root: PathBuf,
    pub codex_bin: PathBuf,
    pub codex_args: Vec<OsString>,
    pub client_version: String,
    pub request_timeout: Duration,
    pub event_capacity: usize,
    pub negotiation_policy: NegotiationPolicy,
    environment: Vec<(OsString, OsString)>,
}

impl ProfileHostConfig {
    pub fn new(
        profile_id: impl Into<String>,
        codex_home: impl Into<PathBuf>,
        workspace_root: impl Into<PathBuf>,
    ) -> Self {
        Self {
            profile_id: profile_id.into(),
            codex_home: codex_home.into(),
            workspace_root: workspace_root.into(),
            codex_bin: PathBuf::from("codex"),
            codex_args: Vec::new(),
            client_version: env!("CARGO_PKG_VERSION").to_string(),
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            event_capacity: DEFAULT_EVENT_CAPACITY,
            negotiation_policy: NegotiationPolicy {
                required_capabilities: vec![
                    "protocol.initialize".to_string(),
                    "thread.lifecycle".to_string(),
                    "turn.lifecycle".to_string(),
                ],
                ..NegotiationPolicy::default()
            },
            environment: Vec::new(),
        }
    }

    pub fn with_codex_bin(mut self, codex_bin: impl Into<PathBuf>) -> Self {
        self.codex_bin = codex_bin.into();
        self
    }

    /// Adds a child-process environment value. Values are intentionally
    /// excluded from `Debug` output and host health snapshots.
    pub fn with_environment(
        mut self,
        key: impl Into<OsString>,
        value: impl Into<OsString>,
    ) -> Self {
        self.environment.push((key.into(), value.into()));
        self
    }
}

impl std::fmt::Debug for ProfileHostConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProfileHostConfig")
            .field("profile_id", &self.profile_id)
            .field("codex_home", &self.codex_home)
            .field("workspace_root", &self.workspace_root)
            .field("codex_bin", &self.codex_bin)
            .field("codex_args", &self.codex_args)
            .field("client_version", &self.client_version)
            .field("request_timeout", &self.request_timeout)
            .field("event_capacity", &self.event_capacity)
            .field("environment", &"[redacted]")
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileHostState {
    Initializing,
    Ready,
    Failed,
    Stopped,
}

#[derive(Debug, Clone)]
pub struct ProfileHostSnapshot {
    pub profile_id: String,
    pub state: ProfileHostState,
    pub process_id: Option<u32>,
    pub server_build: Option<String>,
    pub protocol_version: Option<String>,
    pub capability_count: usize,
    pub last_error: Option<String>,
}

/// One notification emitted by a specific app-server process instance.
///
/// Runtime request ids are only unique inside this instance, so callers must
/// retain the instance id whenever they persist or answer a Server Request.
#[derive(Debug, Clone)]
pub struct ProfileHostEvent {
    pub runtime_instance_id: Uuid,
    pub message: Value,
}

#[derive(Debug, Error)]
pub enum ProfileHostError {
    #[error("invalid Profile configuration: {0}")]
    InvalidConfig(String),
    #[error("failed to prepare Profile: {0}")]
    ProfileIo(#[source] io::Error),
    #[error("Profile {profile_id} already has an app-server owner")]
    AlreadyRunning { profile_id: String },
    #[error("failed to spawn Codex app-server: {0}")]
    Spawn(#[source] io::Error),
    #[error("Codex app-server transport closed")]
    TransportClosed,
    #[error("Codex app-server request belongs to a previous process instance")]
    StaleRuntimeRequest,
    #[error("Codex app-server cannot restart while Turns or Server Requests are active")]
    RuntimeBusy,
    #[error("Codex app-server request timed out: {method}")]
    RequestTimeout { method: String },
    #[error("Codex app-server rejected {method}: {message}")]
    Rpc { method: String, message: String },
    #[error("Codex app-server returned an invalid initialize response: {0}")]
    InvalidInitialize(String),
    #[error("Codex app-server is incompatible: {0}")]
    Incompatible(String),
}

struct ProfileLock {
    file: Option<File>,
}

impl ProfileLock {
    fn acquire(runtime: &Path, profile_id: &str) -> Result<Self, ProfileHostError> {
        let path = runtime.join(LOCK_FILE);
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(0);
        }
        let mut file = options.open(&path).map_err(|error| {
            #[cfg(windows)]
            if matches!(
                error.kind(),
                io::ErrorKind::PermissionDenied | io::ErrorKind::WouldBlock
            ) {
                return ProfileHostError::AlreadyRunning {
                    profile_id: profile_id.to_string(),
                };
            }
            ProfileHostError::ProfileIo(error)
        })?;

        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            // SAFETY: `file` owns a valid descriptor for the duration of the
            // call. The advisory lock is released when the descriptor closes.
            let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            if result != 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::WouldBlock {
                    return Err(ProfileHostError::AlreadyRunning {
                        profile_id: profile_id.to_string(),
                    });
                }
                return Err(ProfileHostError::ProfileIo(error));
            }
        }

        file.set_len(0).map_err(ProfileHostError::ProfileIo)?;
        file.seek(SeekFrom::Start(0))
            .map_err(ProfileHostError::ProfileIo)?;
        writeln!(
            file,
            "{}",
            json!({
                "profileId": profile_id,
                "ownerPid": std::process::id(),
            })
        )
        .map_err(ProfileHostError::ProfileIo)?;
        file.sync_data().map_err(ProfileHostError::ProfileIo)?;

        Ok(Self { file: Some(file) })
    }
}

impl Drop for ProfileLock {
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            #[cfg(unix)]
            {
                use std::os::fd::AsRawFd;
                // SAFETY: the descriptor is valid until `file` is dropped.
                unsafe {
                    libc::flock(file.as_raw_fd(), libc::LOCK_UN);
                }
            }
            drop(file);
        }
        // Keep the inode in place. Removing an unlocked advisory-lock file can
        // race with the next owner opening it and create two independently
        // locked inodes for the same Profile.
    }
}

type PendingSender = oneshot::Sender<Result<Value, String>>;

struct ProfileHostInner {
    home: PathBuf,
    request_timeout: Duration,
    stdin: Mutex<ChildStdin>,
    child: Mutex<Child>,
    pending: Mutex<HashMap<u64, PendingSender>>,
    next_id: AtomicU64,
    events: broadcast::Sender<ProfileHostEvent>,
    snapshot: RwLock<ProfileHostSnapshot>,
    manifest: RwLock<Option<CapabilityManifest>>,
    negotiation: RwLock<Option<NegotiationResult>>,
    lifecycle: RwLock<()>,
    process_generation: AtomicU64,
    runtime_instance_id: RwLock<Uuid>,
    active_turns: RwLock<HashSet<String>>,
    pending_server_requests: RwLock<HashSet<String>>,
    scheduled_restart: Mutex<Option<ProfileHostConfig>>,
    _profile_lock: ProfileLock,
}

/// A native, persistent connection to one Profile's Codex app-server.
#[derive(Clone)]
pub struct ProfileHost {
    inner: Arc<ProfileHostInner>,
}

impl ProfileHost {
    pub async fn spawn(config: ProfileHostConfig) -> Result<Self, ProfileHostError> {
        validate_config(&config)?;
        let workspace_root = config
            .workspace_root
            .canonicalize()
            .map_err(ProfileHostError::ProfileIo)?;
        if !workspace_root.is_dir() {
            return Err(ProfileHostError::InvalidConfig(format!(
                "workspace root {} is not a directory",
                workspace_root.display()
            )));
        }
        let (home, runtime) =
            ensure_profile_layout(&config.codex_home).map_err(ProfileHostError::ProfileIo)?;
        let profile_lock = ProfileLock::acquire(&runtime, &config.profile_id)?;

        let spawned = spawn_app_server(&config, &home, &workspace_root)?;
        let process_id = spawned.child.id();
        let event_capacity = config.event_capacity.max(1);
        let (events, _) = broadcast::channel(event_capacity);
        let snapshot = ProfileHostSnapshot {
            profile_id: config.profile_id.clone(),
            state: ProfileHostState::Initializing,
            process_id,
            server_build: None,
            protocol_version: None,
            capability_count: 0,
            last_error: None,
        };

        let inner = Arc::new(ProfileHostInner {
            home,
            request_timeout: config.request_timeout,
            stdin: Mutex::new(spawned.stdin),
            child: Mutex::new(spawned.child),
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            events,
            snapshot: RwLock::new(snapshot),
            manifest: RwLock::new(None),
            negotiation: RwLock::new(None),
            lifecycle: RwLock::new(()),
            process_generation: AtomicU64::new(1),
            runtime_instance_id: RwLock::new(Uuid::now_v7()),
            active_turns: RwLock::new(HashSet::new()),
            pending_server_requests: RwLock::new(HashSet::new()),
            scheduled_restart: Mutex::new(None),
            _profile_lock: profile_lock,
        });
        spawn_stdout_reader(Arc::downgrade(&inner), 1, spawned.stdout);
        spawn_stderr_monitor(Arc::downgrade(&inner), 1, spawned.stderr);

        let host = Self { inner };
        if let Err(error) = host.initialize(&config).await {
            host.mark_failed(error.to_string()).await;
            host.terminate_child().await;
            return Err(error);
        }
        Ok(host)
    }

    async fn initialize(&self, config: &ProfileHostConfig) -> Result<(), ProfileHostError> {
        let response = timeout(
            INITIALIZE_TIMEOUT,
            self.request("initialize", initialize_params(config)),
        )
        .await
        .map_err(|_| ProfileHostError::RequestTimeout {
            method: "initialize".to_string(),
        })??;

        self.finish_initialize(config, response, false).await
    }

    async fn finish_initialize(
        &self,
        config: &ProfileHostConfig,
        response: Value,
        lifecycle_locked: bool,
    ) -> Result<(), ProfileHostError> {
        let returned_home = response
            .get("codexHome")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ProfileHostError::InvalidInitialize("missing result.codexHome".to_string())
            })?;
        let returned_home = Path::new(returned_home)
            .canonicalize()
            .map_err(ProfileHostError::ProfileIo)?;
        if returned_home != self.inner.home {
            return Err(ProfileHostError::InvalidInitialize(format!(
                "app-server reported CODEX_HOME {} instead of {}",
                returned_home.display(),
                self.inner.home.display()
            )));
        }

        let manifest_value = response.get("capabilityManifest").cloned().ok_or_else(|| {
            ProfileHostError::InvalidInitialize("missing capabilityManifest".to_string())
        })?;
        let manifest: CapabilityManifest =
            serde_json::from_value(manifest_value).map_err(|error| {
                ProfileHostError::InvalidInitialize(format!("invalid capabilityManifest: {error}"))
            })?;
        let negotiation =
            negotiate_capability_manifest(manifest.clone(), &config.negotiation_policy)
                .map_err(|error| ProfileHostError::InvalidInitialize(error.to_string()))?;
        if negotiation.status != "compatible" {
            return Err(ProfileHostError::Incompatible(
                negotiation.reasons.join("; "),
            ));
        }

        if lifecycle_locked {
            self.notify_unlocked("initialized", None).await?;
        } else {
            self.notify("initialized", None).await?;
        }
        {
            let mut snapshot = self.inner.snapshot.write().await;
            snapshot.state = ProfileHostState::Ready;
            snapshot.server_build = Some(manifest.server.build_version.clone());
            snapshot.protocol_version = Some(manifest.server.protocol_version.clone());
            snapshot.capability_count = manifest.capabilities.len();
            snapshot.last_error = None;
        }
        *self.inner.manifest.write().await = Some(manifest);
        *self.inner.negotiation.write().await = Some(negotiation);
        Ok(())
    }

    pub async fn request(&self, method: &str, params: Value) -> Result<Value, ProfileHostError> {
        let _lifecycle = self.inner.lifecycle.read().await;
        let result = self.request_unlocked(method, params).await?;
        // The turn/start response can be observed just before its corresponding
        // turn/started notification. Record it while the lifecycle read lock is
        // still held so a credential-triggered restart cannot enter that gap.
        if method == "turn/start" {
            if let Some(turn_id) = result.pointer("/turn/id").and_then(Value::as_str) {
                self.inner
                    .active_turns
                    .write()
                    .await
                    .insert(turn_id.to_string());
            }
        }
        Ok(result)
    }

    /// Send a request whose response is expected only when a long-running
    /// operation exits. The lifecycle read lock is held only while publishing
    /// the request, so Profile shutdown can still drain the pending response.
    pub async fn request_long_running(
        &self,
        method: &str,
        params: Value,
    ) -> Result<Value, ProfileHostError> {
        let (_id, receiver) = {
            let _lifecycle = self.inner.lifecycle.read().await;
            self.begin_request(method, params).await?
        };
        let response = match receiver.await {
            Ok(Ok(response)) => response,
            Ok(Err(message)) => {
                return Err(ProfileHostError::Rpc {
                    method: method.to_string(),
                    message,
                })
            }
            Err(_) => return Err(ProfileHostError::TransportClosed),
        };
        parse_rpc_result(method, response)
    }

    async fn request_unlocked(
        &self,
        method: &str,
        params: Value,
    ) -> Result<Value, ProfileHostError> {
        let (id, receiver) = self.begin_request(method, params).await?;

        let response = match timeout(self.inner.request_timeout, receiver).await {
            Ok(Ok(Ok(response))) => response,
            Ok(Ok(Err(message))) => {
                return Err(ProfileHostError::Rpc {
                    method: method.to_string(),
                    message,
                })
            }
            Ok(Err(_)) => return Err(ProfileHostError::TransportClosed),
            Err(_) => {
                self.inner.pending.lock().await.remove(&id);
                return Err(ProfileHostError::RequestTimeout {
                    method: method.to_string(),
                });
            }
        };

        parse_rpc_result(method, response)
    }

    async fn begin_request(
        &self,
        method: &str,
        params: Value,
    ) -> Result<(u64, oneshot::Receiver<Result<Value, String>>), ProfileHostError> {
        let id = self.inner.next_id.fetch_add(1, Ordering::SeqCst);
        let (sender, receiver) = oneshot::channel();
        self.inner.pending.lock().await.insert(id, sender);
        if let Err(error) = self
            .write_message(json!({ "id": id, "method": method, "params": params }))
            .await
        {
            self.inner.pending.lock().await.remove(&id);
            return Err(error);
        }
        Ok((id, receiver))
    }

    pub async fn notify(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<(), ProfileHostError> {
        let _lifecycle = self.inner.lifecycle.read().await;
        self.notify_unlocked(method, params).await
    }

    async fn notify_unlocked(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<(), ProfileHostError> {
        let message = match params {
            Some(params) => json!({ "method": method, "params": params }),
            None => json!({ "method": method }),
        };
        self.write_message(message).await
    }

    pub async fn respond(
        &self,
        runtime_instance_id: Uuid,
        request_id: Value,
        result: Result<Value, Value>,
    ) -> Result<(), ProfileHostError> {
        let _lifecycle = self.inner.lifecycle.read().await;
        if *self.inner.runtime_instance_id.read().await != runtime_instance_id {
            return Err(ProfileHostError::StaleRuntimeRequest);
        }
        let message = match result {
            Ok(result) => json!({ "id": request_id, "result": result }),
            Err(error) => json!({ "id": request_id, "error": error }),
        };
        self.write_message(message).await
    }

    async fn write_message(&self, message: Value) -> Result<(), ProfileHostError> {
        let mut line = serde_json::to_vec(&message).map_err(|error| ProfileHostError::Rpc {
            method: "serialize".to_string(),
            message: error.to_string(),
        })?;
        line.push(b'\n');
        let mut stdin = self.inner.stdin.lock().await;
        stdin
            .write_all(&line)
            .await
            .map_err(|_| ProfileHostError::TransportClosed)?;
        stdin
            .flush()
            .await
            .map_err(|_| ProfileHostError::TransportClosed)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ProfileHostEvent> {
        self.inner.events.subscribe()
    }

    pub async fn runtime_instance_id(&self) -> Uuid {
        *self.inner.runtime_instance_id.read().await
    }

    pub async fn snapshot(&self) -> ProfileHostSnapshot {
        self.inner.snapshot.read().await.clone()
    }

    pub async fn capability_manifest(&self) -> Option<CapabilityManifest> {
        self.inner.manifest.read().await.clone()
    }

    pub async fn negotiation(&self) -> Option<NegotiationResult> {
        self.inner.negotiation.read().await.clone()
    }

    pub async fn shutdown(&self) -> Result<(), ProfileHostError> {
        let _lifecycle = self.inner.lifecycle.write().await;
        *self.inner.scheduled_restart.lock().await = None;
        self.shutdown_unlocked().await
    }

    async fn shutdown_unlocked(&self) -> Result<(), ProfileHostError> {
        {
            let mut snapshot = self.inner.snapshot.write().await;
            if snapshot.state == ProfileHostState::Stopped {
                return Ok(());
            }
            snapshot.state = ProfileHostState::Stopped;
        }
        {
            let mut stdin = self.inner.stdin.lock().await;
            let _ = stdin.shutdown().await;
        }
        self.terminate_child().await;
        drain_pending(&self.inner, "app-server stopped").await;
        clear_runtime_work(&self.inner).await;
        Ok(())
    }

    /// Restart the owned app-server in place while retaining the Profile lock,
    /// request identity sequence and event subscription channel. This is used
    /// when server-owned secret environment values change.
    pub async fn restart(&self, config: ProfileHostConfig) -> Result<(), ProfileHostError> {
        let workspace_root = self.validate_restart_config(&config).await?;
        let _lifecycle = self.inner.lifecycle.write().await;
        if self.runtime_is_busy().await {
            return Err(ProfileHostError::RuntimeBusy);
        }
        *self.inner.scheduled_restart.lock().await = None;
        self.restart_unlocked(&config, workspace_root).await
    }

    /// Schedule an app-server restart at the next server-controlled Turn
    /// boundary. The current process keeps serving an in-flight Turn; callers
    /// must invoke [`Self::apply_scheduled_restart`] before starting or
    /// resuming the next Thread operation.
    pub async fn schedule_restart(
        &self,
        config: ProfileHostConfig,
    ) -> Result<(), ProfileHostError> {
        self.validate_restart_config(&config).await?;
        *self.inner.scheduled_restart.lock().await = Some(config);
        Ok(())
    }

    /// Apply a scheduled restart once the current Runtime has no active Turn
    /// or unresolved Server Request. Returns whether a new process instance
    /// was started.
    pub async fn apply_scheduled_restart(&self) -> Result<bool, ProfileHostError> {
        let _lifecycle = self.inner.lifecycle.write().await;
        let Some(config) = self.inner.scheduled_restart.lock().await.take() else {
            return Ok(false);
        };
        if self.runtime_is_busy().await {
            *self.inner.scheduled_restart.lock().await = Some(config);
            return Err(ProfileHostError::RuntimeBusy);
        }
        let workspace_root = match self.validate_restart_config(&config).await {
            Ok(workspace_root) => workspace_root,
            Err(error) => {
                *self.inner.scheduled_restart.lock().await = Some(config);
                return Err(error);
            }
        };
        match self.restart_unlocked(&config, workspace_root).await {
            Ok(()) => Ok(true),
            Err(error) => {
                *self.inner.scheduled_restart.lock().await = Some(config);
                Err(error)
            }
        }
    }

    async fn validate_restart_config(
        &self,
        config: &ProfileHostConfig,
    ) -> Result<PathBuf, ProfileHostError> {
        validate_config(config)?;
        let expected_profile = self.snapshot().await.profile_id;
        if config.profile_id != expected_profile {
            return Err(ProfileHostError::InvalidConfig(
                "restart profile_id does not match the running Profile".to_string(),
            ));
        }
        let home = config
            .codex_home
            .canonicalize()
            .map_err(ProfileHostError::ProfileIo)?;
        if home != self.inner.home {
            return Err(ProfileHostError::InvalidConfig(
                "restart CODEX_HOME does not match the running Profile".to_string(),
            ));
        }
        let workspace_root = config
            .workspace_root
            .canonicalize()
            .map_err(ProfileHostError::ProfileIo)?;
        if !workspace_root.is_dir() {
            return Err(ProfileHostError::InvalidConfig(format!(
                "workspace root {} is not a directory",
                workspace_root.display()
            )));
        }
        Ok(workspace_root)
    }

    async fn runtime_is_busy(&self) -> bool {
        !self.inner.active_turns.read().await.is_empty()
            || !self.inner.pending_server_requests.read().await.is_empty()
    }

    async fn restart_unlocked(
        &self,
        config: &ProfileHostConfig,
        workspace_root: PathBuf,
    ) -> Result<(), ProfileHostError> {
        self.inner.process_generation.fetch_add(1, Ordering::SeqCst);
        self.shutdown_unlocked().await?;
        *self.inner.runtime_instance_id.write().await = Uuid::now_v7();
        let generation = self.inner.process_generation.load(Ordering::SeqCst);
        let spawned = spawn_app_server(config, &self.inner.home, &workspace_root)?;
        let process_id = spawned.child.id();
        *self.inner.stdin.lock().await = spawned.stdin;
        *self.inner.child.lock().await = spawned.child;
        *self.inner.manifest.write().await = None;
        *self.inner.negotiation.write().await = None;
        {
            let mut snapshot = self.inner.snapshot.write().await;
            snapshot.state = ProfileHostState::Initializing;
            snapshot.process_id = process_id;
            snapshot.server_build = None;
            snapshot.protocol_version = None;
            snapshot.capability_count = 0;
            snapshot.last_error = None;
        }
        spawn_stdout_reader(Arc::downgrade(&self.inner), generation, spawned.stdout);
        spawn_stderr_monitor(Arc::downgrade(&self.inner), generation, spawned.stderr);

        if let Err(error) = self.initialize_unlocked(&config).await {
            self.mark_failed(error.to_string()).await;
            self.terminate_child().await;
            return Err(error);
        }
        Ok(())
    }

    async fn initialize_unlocked(
        &self,
        config: &ProfileHostConfig,
    ) -> Result<(), ProfileHostError> {
        let response = timeout(
            INITIALIZE_TIMEOUT,
            self.request_unlocked("initialize", initialize_params(config)),
        )
        .await
        .map_err(|_| ProfileHostError::RequestTimeout {
            method: "initialize".to_string(),
        })??;
        self.finish_initialize(config, response, true).await
    }

    async fn terminate_child(&self) {
        let mut child = self.inner.child.lock().await;
        let _ = child.start_kill();
        let _ = timeout(Duration::from_secs(5), child.wait()).await;
    }

    async fn mark_failed(&self, message: String) {
        let mut snapshot = self.inner.snapshot.write().await;
        if snapshot.state != ProfileHostState::Stopped {
            snapshot.state = ProfileHostState::Failed;
            snapshot.last_error = Some(message);
        }
    }
}

struct SpawnedAppServer {
    child: Child,
    stdin: ChildStdin,
    stdout: tokio::process::ChildStdout,
    stderr: tokio::process::ChildStderr,
}

fn spawn_app_server(
    config: &ProfileHostConfig,
    home: &Path,
    workspace_root: &Path,
) -> Result<SpawnedAppServer, ProfileHostError> {
    let mut command = Command::new(&config.codex_bin);
    command
        .args(&config.codex_args)
        .arg("app-server")
        .current_dir(workspace_root)
        .env("CODEX_HOME", home)
        .envs(config.environment.iter().map(|(key, value)| (key, value)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = command.spawn().map_err(ProfileHostError::Spawn)?;
    let stdin = child.stdin.take().ok_or_else(|| {
        ProfileHostError::InvalidInitialize("child stdin was not available".to_string())
    })?;
    let stdout = child.stdout.take().ok_or_else(|| {
        ProfileHostError::InvalidInitialize("child stdout was not available".to_string())
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        ProfileHostError::InvalidInitialize("child stderr was not available".to_string())
    })?;
    Ok(SpawnedAppServer {
        child,
        stdin,
        stdout,
        stderr,
    })
}

fn initialize_params(config: &ProfileHostConfig) -> Value {
    json!({
        "clientInfo": {
            "name": "open_web_codex_profile_host",
            "title": "Open Web Codex Profile Host",
            "version": config.client_version,
        },
        "capabilities": {
            "experimentalApi": true,
        },
    })
}

fn validate_config(config: &ProfileHostConfig) -> Result<(), ProfileHostError> {
    if config.profile_id.trim().is_empty() {
        return Err(ProfileHostError::InvalidConfig(
            "profile_id must not be empty".to_string(),
        ));
    }
    if config.event_capacity == 0 {
        return Err(ProfileHostError::InvalidConfig(
            "event_capacity must be greater than zero".to_string(),
        ));
    }
    if config.codex_bin.as_os_str().is_empty() {
        return Err(ProfileHostError::InvalidConfig(
            "codex_bin must not be empty".to_string(),
        ));
    }
    Ok(())
}

fn spawn_stdout_reader(
    inner: std::sync::Weak<ProfileHostInner>,
    generation: u64,
    stdout: tokio::process::ChildStdout,
) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        loop {
            let line = match lines.next_line().await {
                Ok(Some(line)) => line,
                Ok(None) => break,
                Err(_) => break,
            };
            if line.trim().is_empty() {
                continue;
            }
            let Some(inner) = inner.upgrade() else {
                return;
            };
            if inner.process_generation.load(Ordering::SeqCst) != generation {
                return;
            }
            match serde_json::from_str::<Value>(&line) {
                Ok(message) => dispatch_incoming(&inner, generation, message).await,
                Err(_) => {
                    let runtime_instance_id = *inner.runtime_instance_id.read().await;
                    let _ = inner.events.send(ProfileHostEvent {
                        runtime_instance_id,
                        message: json!({
                            "method": "codex/parseError",
                            "params": { "message": "app-server emitted invalid JSON" },
                        }),
                    });
                }
            }
        }

        if let Some(inner) = inner.upgrade() {
            if inner.process_generation.load(Ordering::SeqCst) != generation {
                return;
            }
            {
                let mut snapshot = inner.snapshot.write().await;
                if snapshot.state != ProfileHostState::Stopped {
                    snapshot.state = ProfileHostState::Failed;
                    snapshot.last_error = Some("app-server stdout closed".to_string());
                }
            }
            drain_pending(&inner, "app-server stdout closed").await;
            // Runtime work belongs to the process generation that just died.
            // Keeping these identities would permanently block a safe restart
            // even though no process remains to complete them.
            clear_runtime_work(&inner).await;
        }
    });
}

fn spawn_stderr_monitor(
    inner: std::sync::Weak<ProfileHostInner>,
    generation: u64,
    stderr: tokio::process::ChildStderr,
) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if line.trim().is_empty() {
                continue;
            }
            let Some(inner) = inner.upgrade() else {
                return;
            };
            if inner.process_generation.load(Ordering::SeqCst) != generation {
                return;
            }
            // stderr may contain paths or credentials. Record only the fact
            // that diagnostics were observed; do not forward its contents.
            let mut snapshot = inner.snapshot.write().await;
            if snapshot.state != ProfileHostState::Stopped && snapshot.last_error.is_none() {
                snapshot.last_error = Some("app-server wrote diagnostic output".to_string());
            }
        }
    });
}

async fn dispatch_incoming(inner: &ProfileHostInner, generation: u64, message: Value) {
    let response_id = message.get("id").and_then(Value::as_u64);
    let is_response = message.get("result").is_some() || message.get("error").is_some();
    if is_response {
        if let Some(id) = response_id {
            if let Some(sender) = inner.pending.lock().await.remove(&id) {
                let _ = sender.send(Ok(message));
                return;
            }
        }
    }

    if message.get("method").and_then(Value::as_str).is_some() {
        update_runtime_work(inner, &message).await;
        if inner.process_generation.load(Ordering::SeqCst) == generation {
            let runtime_instance_id = *inner.runtime_instance_id.read().await;
            let _ = inner.events.send(ProfileHostEvent {
                runtime_instance_id,
                message,
            });
        }
    }
}

async fn update_runtime_work(inner: &ProfileHostInner, message: &Value) {
    let method = message
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let turn_id = message
        .pointer("/params/turn/id")
        .or_else(|| message.pointer("/params/turnId"))
        .and_then(Value::as_str)
        .map(str::to_string);
    match method {
        "turn/started" => {
            if let Some(turn_id) = turn_id {
                inner.active_turns.write().await.insert(turn_id);
            }
        }
        "turn/completed" => {
            if let Some(turn_id) = turn_id {
                inner.active_turns.write().await.remove(&turn_id);
            }
        }
        "item/commandExecution/requestApproval"
        | "item/fileChange/requestApproval"
        | "mcpServer/elicitation/request"
        | "item/permissions/requestApproval"
        | "item/tool/requestUserInput" => {
            if let Some(request_id) = message.get("id") {
                if let Ok(request_id) = serde_json::to_string(request_id) {
                    inner
                        .pending_server_requests
                        .write()
                        .await
                        .insert(request_id);
                }
            }
        }
        "serverRequest/resolved" => {
            if let Some(request_id) = message.pointer("/params/requestId") {
                if let Ok(request_id) = serde_json::to_string(request_id) {
                    inner
                        .pending_server_requests
                        .write()
                        .await
                        .remove(&request_id);
                }
            }
        }
        _ => {}
    }
}

async fn drain_pending(inner: &ProfileHostInner, message: &str) {
    let pending = std::mem::take(&mut *inner.pending.lock().await);
    for (_, sender) in pending {
        let _ = sender.send(Err(message.to_string()));
    }
}

async fn clear_runtime_work(inner: &ProfileHostInner) {
    inner.active_turns.write().await.clear();
    inner.pending_server_requests.write().await.clear();
}

fn rpc_error_message(error: &Value) -> String {
    error
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| error.as_str())
        .unwrap_or("unknown app-server error")
        .to_string()
}

fn parse_rpc_result(method: &str, response: Value) -> Result<Value, ProfileHostError> {
    if let Some(error) = response.get("error") {
        return Err(ProfileHostError::Rpc {
            method: method.to_string(),
            message: rpc_error_message(error),
        });
    }
    response
        .get("result")
        .cloned()
        .ok_or_else(|| ProfileHostError::Rpc {
            method: method.to_string(),
            message: "response contained neither result nor error".to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::{
        clear_runtime_work, dispatch_incoming, ensure_profile_home, ensure_profile_layout,
        ProfileHost, ProfileHostConfig, ProfileHostError, ProfileHostInner, ProfileHostSnapshot,
        ProfileHostState, ProfileLock,
    };
    use serde_json::json;
    use std::collections::{HashMap, HashSet};
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    use tokio::sync::{broadcast, oneshot, Mutex, RwLock};
    use uuid::Uuid;

    fn temporary_path(name: &str) -> PathBuf {
        static NEXT_PATH_ID: AtomicU64 = AtomicU64::new(1);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock is after Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "open-web-codex-profile-host-{name}-{}-{timestamp}-{}",
            std::process::id(),
            NEXT_PATH_ID.fetch_add(1, Ordering::Relaxed),
        ))
    }

    #[test]
    fn creates_a_missing_profile_home() {
        let path = temporary_path("missing");
        let resolved = ensure_profile_home(&path).expect("create profile home");

        assert!(resolved.is_dir());
        assert_eq!(resolved, path.canonicalize().expect("canonical path"));

        fs::remove_dir_all(path).expect("remove profile home");
    }

    #[cfg(unix)]
    #[test]
    fn profile_directories_are_private() {
        use std::os::unix::fs::PermissionsExt;

        let path = temporary_path("permissions");
        let (_, runtime) = ensure_profile_layout(&path).expect("create profile layout");

        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(runtime).unwrap().permissions().mode() & 0o777,
            0o700
        );

        fs::remove_dir_all(path).expect("remove profile home");
    }

    #[test]
    fn rejects_a_profile_home_that_is_a_file() {
        let path = temporary_path("file");
        fs::write(&path, "not a directory").expect("create file");

        let error = ensure_profile_home(&path).expect_err("file cannot be a profile home");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);

        fs::remove_file(path).expect("remove file");
    }

    #[cfg(unix)]
    #[test]
    fn profile_lock_has_a_single_owner() {
        let path = temporary_path("lock");
        let (_, runtime) = ensure_profile_layout(&path).expect("create profile layout");
        let first = ProfileLock::acquire(&runtime, "profile-1").expect("first lock");
        let second = ProfileLock::acquire(&runtime, "profile-1");

        assert!(matches!(
            second,
            Err(ProfileHostError::AlreadyRunning { profile_id }) if profile_id == "profile-1"
        ));

        drop(first);
        ProfileLock::acquire(&runtime, "profile-1").expect("lock after release");
        fs::remove_dir_all(path).expect("remove profile home");
    }

    #[test]
    fn debug_output_redacts_child_environment() {
        let config = ProfileHostConfig::new("profile", "/tmp/profile", "/tmp")
            .with_environment("PROVIDER_API_KEY", "secret-value");
        let debug = format!("{config:?}");

        assert!(!debug.contains("secret-value"));
        assert!(debug.contains("[redacted]"));
    }

    async fn test_inner(event_capacity: usize) -> (Arc<ProfileHostInner>, PathBuf) {
        let path = temporary_path("router");
        let (home, runtime) = ensure_profile_layout(&path).expect("create layout");
        let lock = ProfileLock::acquire(&runtime, "test-profile").expect("profile lock");
        let mut child = tokio::process::Command::new("sh")
            .arg("-c")
            .arg("sleep 30")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .expect("spawn test child");
        let stdin = child.stdin.take().expect("test stdin");
        let (events, _) = broadcast::channel(event_capacity);
        let inner = Arc::new(ProfileHostInner {
            home,
            request_timeout: Duration::from_secs(1),
            stdin: Mutex::new(stdin),
            child: Mutex::new(child),
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            events,
            snapshot: RwLock::new(ProfileHostSnapshot {
                profile_id: "test-profile".to_string(),
                state: ProfileHostState::Ready,
                process_id: None,
                server_build: None,
                protocol_version: None,
                capability_count: 0,
                last_error: None,
            }),
            manifest: RwLock::new(None),
            negotiation: RwLock::new(None),
            lifecycle: RwLock::new(()),
            process_generation: AtomicU64::new(1),
            runtime_instance_id: RwLock::new(Uuid::now_v7()),
            active_turns: RwLock::new(HashSet::new()),
            pending_server_requests: RwLock::new(HashSet::new()),
            scheduled_restart: Mutex::new(None),
            _profile_lock: lock,
        });
        (inner, path)
    }

    #[tokio::test]
    async fn correlates_out_of_order_responses_and_ignores_duplicates() {
        let (inner, path) = test_inner(8).await;
        let (first_tx, first_rx) = oneshot::channel();
        let (second_tx, second_rx) = oneshot::channel();
        inner.pending.lock().await.insert(1, first_tx);
        inner.pending.lock().await.insert(2, second_tx);

        dispatch_incoming(
            &inner,
            1,
            json!({ "id": 2, "result": { "value": "second" } }),
        )
        .await;
        dispatch_incoming(
            &inner,
            1,
            json!({ "id": 2, "result": { "value": "duplicate" } }),
        )
        .await;
        dispatch_incoming(
            &inner,
            1,
            json!({ "id": 1, "result": { "value": "first" } }),
        )
        .await;

        assert_eq!(
            second_rx.await.unwrap().unwrap()["result"]["value"],
            "second"
        );
        assert_eq!(first_rx.await.unwrap().unwrap()["result"]["value"], "first");
        assert!(inner.pending.lock().await.is_empty());

        let mut child = inner.child.lock().await;
        let _ = child.kill().await;
        drop(child);
        drop(inner);
        fs::remove_dir_all(path).expect("remove profile home");
    }

    #[tokio::test]
    async fn bounded_event_stream_reports_lag_to_slow_consumers() {
        let (inner, path) = test_inner(2).await;
        let mut receiver = inner.events.subscribe();
        for sequence in 0..4 {
            dispatch_incoming(
                &inner,
                1,
                json!({ "method": "item/updated", "params": { "sequence": sequence } }),
            )
            .await;
        }

        assert!(matches!(
            receiver.recv().await,
            Err(broadcast::error::RecvError::Lagged(2))
        ));
        assert_eq!(
            receiver.recv().await.unwrap().message["params"]["sequence"],
            2
        );
        assert_eq!(
            receiver.recv().await.unwrap().message["params"]["sequence"],
            3
        );

        let mut child = inner.child.lock().await;
        let _ = child.kill().await;
        drop(child);
        drop(inner);
        fs::remove_dir_all(path).expect("remove profile home");
    }

    #[tokio::test]
    async fn runtime_instance_guards_responses_and_tracks_restart_blockers() {
        let (inner, path) = test_inner(8).await;
        let host = ProfileHost {
            inner: inner.clone(),
        };
        let runtime_instance_id = host.runtime_instance_id().await;
        let stale = Uuid::nil();

        let error = host
            .respond(stale, json!(7), Ok(json!({ "decision": "accept" })))
            .await
            .expect_err("stale Runtime request must not be written");
        assert!(matches!(error, ProfileHostError::StaleRuntimeRequest));

        let mut events = inner.events.subscribe();
        dispatch_incoming(
            &inner,
            1,
            json!({
                "method": "turn/started",
                "params": { "turn": { "id": "turn-1" }, "threadId": "thread-1" }
            }),
        )
        .await;
        dispatch_incoming(
            &inner,
            1,
            json!({
                "id": "approval-1",
                "method": "item/commandExecution/requestApproval",
                "params": { "threadId": "thread-1", "turnId": "turn-1" }
            }),
        )
        .await;

        assert!(inner.active_turns.read().await.contains("turn-1"));
        assert!(inner
            .pending_server_requests
            .read()
            .await
            .contains("\"approval-1\""));
        assert_eq!(
            events.recv().await.expect("turn event").runtime_instance_id,
            runtime_instance_id
        );
        assert_eq!(
            events
                .recv()
                .await
                .expect("approval event")
                .runtime_instance_id,
            runtime_instance_id
        );

        dispatch_incoming(
            &inner,
            1,
            json!({
                "method": "turn/completed",
                "params": { "turn": { "id": "turn-1" }, "threadId": "thread-1" }
            }),
        )
        .await;
        dispatch_incoming(
            &inner,
            1,
            json!({
                "method": "serverRequest/resolved",
                "params": { "threadId": "thread-1", "requestId": "approval-1" }
            }),
        )
        .await;
        assert!(inner.active_turns.read().await.is_empty());
        assert!(inner.pending_server_requests.read().await.is_empty());

        inner
            .active_turns
            .write()
            .await
            .insert("orphaned-turn".into());
        inner
            .pending_server_requests
            .write()
            .await
            .insert("\"orphaned-request\"".into());
        clear_runtime_work(&inner).await;
        assert!(inner.active_turns.read().await.is_empty());
        assert!(inner.pending_server_requests.read().await.is_empty());

        let mut child = inner.child.lock().await;
        let _ = child.kill().await;
        drop(child);
        drop(host);
        drop(inner);
        fs::remove_dir_all(path).expect("remove profile home");
    }

    #[tokio::test]
    async fn scheduled_restart_waits_for_the_active_turn_boundary() {
        let (inner, path) = test_inner(8).await;
        let host = ProfileHost {
            inner: inner.clone(),
        };
        inner
            .active_turns
            .write()
            .await
            .insert("active-turn".to_string());
        host.schedule_restart(ProfileHostConfig::new("test-profile", &path, &path))
            .await
            .expect("schedule restart");

        let error = host
            .apply_scheduled_restart()
            .await
            .expect_err("active Turn must defer the restart");

        assert!(matches!(error, ProfileHostError::RuntimeBusy));
        assert!(inner.scheduled_restart.lock().await.is_some());

        let mut child = inner.child.lock().await;
        let _ = child.kill().await;
        drop(child);
        drop(host);
        drop(inner);
        fs::remove_dir_all(path).expect("remove profile home");
    }
}
