pub mod fake;
pub mod real;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use thiserror::Error;
use uuid::Uuid;

/// Errors from the Codex adapter layer.
#[derive(Debug, Error)]
pub enum AdapterError {
    #[error("Codex runtime unreachable: {0}")]
    Unreachable(String),

    #[error("RPC error: {0}")]
    Rpc(String),

    #[error("Method not implemented: {0}")]
    NotImplemented(String),

    #[error("Profile Host error: {0}")]
    ProfileHost(#[from] open_web_codex_profile_host::ProfileHostError),

    #[error("Internal error: {0}")]
    Internal(String),
}

/// Health status returned by the adapter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStatus {
    pub ok: bool,
    pub version: String,
    pub name: String,
}

/// Events emitted by the adapter for durable platform projection.
#[derive(Debug, Clone)]
pub struct AdapterEvent {
    pub data: Vec<u8>,
}

/// Server-authorized workspace context for typed Runtime calls. The root is an
/// internal Runner path and must never be constructed from a browser payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizedWorkspace {
    pub id: String,
    pub root: PathBuf,
}

/// Stable result of starting a Codex Thread in an authorized workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartedThread {
    pub thread_id: String,
}

/// Browser-login handoff returned by the official app-server. The platform
/// exposes only the opaque login id and the URL the browser must open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartedProfileLogin {
    pub login_id: String,
    pub auth_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanceledProfileLogin {
    pub canceled: bool,
    pub status: String,
}

/// Typed, platform-safe projection of the official login completion event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileLoginStatus {
    pub completed: bool,
    pub success: Option<bool>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct TurnOptions {
    pub model: Option<String>,
    pub model_provider: Option<String>,
    pub effort: Option<String>,
    pub service_tier: Option<String>,
    pub access_mode: Option<String>,
    pub images: Vec<String>,
    pub collaboration_mode: Option<Value>,
}

#[derive(Debug, Clone)]
pub enum ProfileQuery {
    Account,
    RateLimits,
    Usage,
    CollaborationModes,
    Apps {
        cursor: Option<String>,
        limit: Option<u32>,
        thread_id: Option<String>,
    },
    McpServers {
        cursor: Option<String>,
        limit: Option<u32>,
        thread_id: Option<String>,
    },
    ExperimentalFeatures {
        cursor: Option<String>,
        limit: Option<u32>,
        thread_id: Option<String>,
    },
    Skills {
        workspace: AuthorizedWorkspace,
        force_reload: bool,
    },
    Config,
}

#[derive(Debug, Clone)]
pub enum ProfileMutation {
    SetExperimentalFeature {
        name: String,
        enabled: bool,
    },
    SetAgentCore {
        multi_agent_enabled: bool,
        max_threads: u32,
        max_depth: u32,
    },
    SetAgentDefinition {
        original_name: Option<String>,
        name: String,
        description: Option<String>,
        config_file: String,
    },
    RemoveAgentDefinition {
        name: String,
    },
}

#[derive(Debug, Clone)]
pub enum ReviewTarget {
    UncommittedChanges,
    BaseBranch { branch: String },
    Commit { sha: String, title: Option<String> },
    Custom { instructions: String },
}

/// Mode for the Codex adapter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AdapterMode {
    /// Use a real (or proxied) Codex runtime.
    Real,
    /// Use an in-memory fake that simulates responses.
    Fake,
}

impl std::fmt::Display for AdapterMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Real => write!(f, "real"),
            Self::Fake => write!(f, "fake"),
        }
    }
}

/// Abstract interface to the Codex runtime.
///
/// Platform server routes call through this trait instead of talking
/// directly to an app-server transport or the Codex process.
#[async_trait]
pub trait CodexAdapter: Send + Sync {
    /// Quick health probe.
    async fn health(&self) -> Result<HealthStatus, AdapterError>;

    /// Unique identity of the currently running app-server process. Runtime
    /// Server Request ids are valid only inside this instance.
    async fn runtime_instance_id(&self) -> Uuid;

    /// Transitional internal RPC call. Product routes should replace this
    /// with typed Thread/Turn operations; it is never a public browser
    /// contract.
    async fn rpc(&self, method: &str, params: Value) -> Result<Value, AdapterError>;

    /// Start a Thread in a server-owned, authorization-checked workspace.
    async fn start_thread(
        &self,
        workspace: &AuthorizedWorkspace,
    ) -> Result<StartedThread, AdapterError>;

    async fn fork_thread(
        &self,
        source_workspace: &AuthorizedWorkspace,
        target_workspace: &AuthorizedWorkspace,
        thread_id: &str,
    ) -> Result<StartedThread, AdapterError>;

    /// Read the authoritative persisted Thread from Codex.
    async fn read_thread(
        &self,
        workspace: &AuthorizedWorkspace,
        thread_id: &str,
    ) -> Result<Value, AdapterError>;

    /// Read all persisted Turns with full items from Codex pagination.
    async fn list_thread_turns(
        &self,
        workspace: &AuthorizedWorkspace,
        thread_id: &str,
    ) -> Result<Vec<Value>, AdapterError>;

    /// Read an MCP Resource through the official app-server API after the
    /// platform has authorized the owning Thread and hidden the source URI
    /// from the browser.
    async fn read_mcp_resource(
        &self,
        workspace: &AuthorizedWorkspace,
        thread_id: &str,
        server: &str,
        uri: &str,
    ) -> Result<Value, AdapterError>;

    /// Start a user turn in the authorized workspace bound to the Thread.
    async fn send_user_message(
        &self,
        workspace: &AuthorizedWorkspace,
        thread_id: &str,
        text: &str,
        options: &TurnOptions,
    ) -> Result<Value, AdapterError>;

    /// Interrupt the active Turn identified by the durable platform
    /// projection. The caller must authorize both the Thread and workspace.
    async fn interrupt_turn(
        &self,
        workspace: &AuthorizedWorkspace,
        thread_id: &str,
        turn_id: &str,
    ) -> Result<(), AdapterError>;

    /// Steer the active Turn while preserving the Run and Thread.
    async fn steer_turn(
        &self,
        workspace: &AuthorizedWorkspace,
        thread_id: &str,
        turn_id: &str,
        text: &str,
        images: &[String],
    ) -> Result<Value, AdapterError>;

    /// Internal response path for app-server initiated requests. Public routes
    /// must resolve a platform-owned durable request before calling this.
    async fn respond_to_server_request(
        &self,
        runtime_instance_id: Uuid,
        request_id: Value,
        result: Value,
    ) -> Result<(), AdapterError>;

    /// Read a fixed, typed Profile capability. The enum owns the app-server
    /// method selection so browser input can never choose a raw method.
    async fn query_profile(&self, query: ProfileQuery) -> Result<Value, AdapterError>;

    /// Apply a fixed Profile mutation selected by the platform. Browser input
    /// never supplies an app-server method or raw configuration key path.
    async fn mutate_profile(&self, mutation: ProfileMutation) -> Result<Value, AdapterError>;

    /// Start the official ChatGPT browser login flow for this Profile.
    async fn start_profile_login(&self) -> Result<StartedProfileLogin, AdapterError>;

    /// Cancel the active browser login for this Profile, if any.
    async fn cancel_profile_login(&self) -> Result<CanceledProfileLogin, AdapterError>;

    /// Read the completion state captured from the official app-server event.
    async fn profile_login_status(
        &self,
        login_id: &str,
    ) -> Result<ProfileLoginStatus, AdapterError>;

    async fn archive_thread(
        &self,
        workspace: &AuthorizedWorkspace,
        thread_id: &str,
    ) -> Result<(), AdapterError>;

    async fn set_thread_name(
        &self,
        workspace: &AuthorizedWorkspace,
        thread_id: &str,
        name: &str,
    ) -> Result<(), AdapterError>;

    /// Run a short, platform-authored background prompt in an authorized
    /// workspace and return only the final assistant text.
    async fn generate_text(
        &self,
        workspace: &AuthorizedWorkspace,
        prompt: &str,
        model: Option<&str>,
    ) -> Result<String, AdapterError>;

    async fn compact_thread(
        &self,
        workspace: &AuthorizedWorkspace,
        thread_id: &str,
    ) -> Result<Value, AdapterError>;

    async fn start_review(
        &self,
        workspace: &AuthorizedWorkspace,
        thread_id: &str,
        target: ReviewTarget,
    ) -> Result<Value, AdapterError>;

    async fn open_terminal(
        &self,
        workspace: &AuthorizedWorkspace,
        process_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), AdapterError>;

    async fn write_terminal(
        &self,
        workspace: &AuthorizedWorkspace,
        process_id: &str,
        data: &str,
    ) -> Result<(), AdapterError>;

    async fn resize_terminal(
        &self,
        workspace: &AuthorizedWorkspace,
        process_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), AdapterError>;

    async fn close_terminal(
        &self,
        workspace: &AuthorizedWorkspace,
        process_id: &str,
    ) -> Result<(), AdapterError>;

    /// Subscribe to the internal app-server event stream. The implementor sends
    /// frames through `sender` and returns when the subscription ends.
    async fn subscribe_events(
        &self,
        sender: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
    ) -> Result<(), AdapterError>;
}
