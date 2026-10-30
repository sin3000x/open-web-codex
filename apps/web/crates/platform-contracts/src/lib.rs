pub mod error;
pub mod event;
pub mod idempotency;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Platform-level health response.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HealthResponse {
    pub ok: bool,
    #[serde(rename = "schemaStatus")]
    pub schema_status: String,
    pub version: String,
    pub started_at: DateTime<Utc>,
    pub uptime_seconds: u64,
}

/// Generic pagination cursor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cursor {
    pub before: Option<Uuid>,
    pub after: Option<Uuid>,
    pub limit: u32,
}

impl Default for Cursor {
    fn default() -> Self {
        Self {
            before: None,
            after: None,
            limit: 50,
        }
    }
}

/// Platform event projection cursor (opaque to clients).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventCursor(pub String);

// ── Project ──────────────────────────────────────────────────────────

/// Database/API project representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: Uuid,
    pub name: String,
    pub git_url: String,
    pub default_branch: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Request body to create a project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateProjectRequest {
    pub name: String,
    pub git_url: String,
    #[serde(default)]
    pub default_branch: Option<String>,
}

/// Request body to create a server-managed empty project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateManagedProjectRequest {
    pub name: String,
}

// ── Task ──────────────────────────────────────────────────────────────

/// Database/API task representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: Uuid,
    pub project_id: Uuid,
    /// Immutable authorized execution root selected when the Task is created.
    /// Runs always derive their Workspace from this value.
    pub workspace_id: Uuid,
    pub title: String,
    pub status: String,
    pub model_provider: Option<String>,
    pub model: Option<String>,
    /// Immutable application-discovered Copilot package selected for this
    /// Task. The browser never submits a package path or Runtime config.
    pub copilot_package_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Request body to create a task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateTaskRequest {
    pub project_id: Uuid,
    /// Stable Workspace identity only. The server resolves the corresponding
    /// authorized root; no path is accepted from the browser.
    pub workspace_id: Uuid,
    pub title: String,
    #[serde(default)]
    pub model_provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub copilot_package_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelSelection {
    pub provider_id: String,
    pub model_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTaskModelSelectionRequest {
    pub provider_id: String,
    pub model_id: String,
}

// ── Auth ────────────────────────────────────────────────────────────

/// Public user representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: Uuid,
    pub name: String,
    pub username: String,
    pub email: String,
    pub role: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Request body for one-time bootstrap.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapRequest {
    pub name: String,
    pub username: String,
    pub email: String,
    pub password: String,
}

/// Response from bootstrap.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapResponse {
    pub organization: Organization,
    pub user: User,
    pub session_token: String,
}

/// Request body for login.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub organization_id: Option<Uuid>,
}

/// Response from login.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginResponse {
    pub user: User,
    pub organization: Organization,
    pub membership_role: String,
    pub session_token: String,
}

/// Response from /api/me.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeResponse {
    pub id: Uuid,
    pub name: String,
    pub username: String,
    pub email: String,
    pub role: String,
    pub organization_id: Uuid,
    pub organization_role: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectOrganizationRequest {
    pub organization_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionOrganization {
    pub organization: Organization,
    pub role: String,
}

// ── Organization ────────────────────────────────────────────────────

/// Database/API organization representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Organization {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Request to create an organization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateOrganizationRequest {
    pub name: String,
    pub slug: Option<String>,
}

/// Organization membership record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Membership {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub user_id: Uuid,
    pub role: String,
    pub created_at: DateTime<Utc>,
}

/// Member info returned in member list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberInfo {
    pub user_id: Uuid,
    pub name: String,
    pub email: String,
    pub role: String,
}

/// Request to add a member.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddMemberRequest {
    pub email: String,
    pub role: Option<String>,
}

// ── Run ────────────────────────────────────────────────────────────

/// Stable, browser-safe classification for a terminal or interrupted Run.
/// Unknown persisted values remain observable without exposing raw diagnostics.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunFailureCode {
    InvalidRun,
    ResourceNotFound,
    RunConflict,
    LeaseLost,
    DatabaseError,
    GitWorkspaceError,
    CodexUnavailable,
    RunCancelled,
    InterruptFailed,
    LeaseExpired,
    UnknownFailure,
}

impl RunFailureCode {
    pub fn from_persisted(value: &str) -> Self {
        match value {
            "invalid_run" => Self::InvalidRun,
            "resource_not_found" => Self::ResourceNotFound,
            "run_conflict" => Self::RunConflict,
            "lease_lost" => Self::LeaseLost,
            "database_error" => Self::DatabaseError,
            "git_workspace_error" => Self::GitWorkspaceError,
            "codex_unavailable" => Self::CodexUnavailable,
            "run_cancelled" => Self::RunCancelled,
            "interrupt_failed" => Self::InterruptFailed,
            "lease_expired" => Self::LeaseExpired,
            _ => Self::UnknownFailure,
        }
    }
}

/// Database/API run representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Run {
    pub id: Uuid,
    pub task_id: Uuid,
    pub status: String,
    pub failure_code: Option<RunFailureCode>,
    pub codex_thread_id: Option<String>,
    pub active_turn_id: Option<String>,
    /// Immutable copy of the owning Task's Workspace, enforced by the
    /// database composite foreign key.
    pub workspace_id: Uuid,
    pub attempt: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// One authorized Project/Task/Run/Thread mapping for browser navigation.
/// The browser never needs to infer a Run by scanning Tasks independently.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectThreadContext {
    pub project: Project,
    pub task: Task,
    pub run: Run,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadHistoryStatus {
    pub r#type: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub active_flags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadHistoryError {
    pub message: String,
    pub additional_details: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadHistoryTurn {
    pub id: String,
    pub status: String,
    pub items: Vec<serde_json::Value>,
    pub error: Option<ThreadHistoryError>,
    pub started_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub duration_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadHistory {
    pub id: String,
    pub name: Option<String>,
    pub preview: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub status: ThreadHistoryStatus,
    pub turns: Vec<ThreadHistoryTurn>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadHistoryResponse {
    pub thread: ThreadHistory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadTokenUsageBreakdown {
    pub total_tokens: u64,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_output_tokens: u64,
}

/// Latest Runtime-reported usage for one Codex Thread, read from the durable
/// `thread/tokenUsage/updated` projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadTokenUsage {
    pub total: ThreadTokenUsageBreakdown,
    pub last: ThreadTokenUsageBreakdown,
    pub model_context_window: Option<u64>,
}

/// `token_usage` is `None` until the Runtime has reported usage for the Thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadTokenUsageResponse {
    pub token_usage: Option<ThreadTokenUsage>,
}

/// Request to start a run.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartRunRequest {
    pub idempotency_key: String,
    #[serde(default)]
    pub fork_thread_id: Option<String>,
    #[serde(default)]
    pub fork_source_run_id: Option<Uuid>,
}

#[cfg(test)]
mod task_workspace_contract_tests {
    use super::StartRunRequest;

    #[test]
    fn start_run_accepts_only_the_standard_or_fork_contract() {
        let valid = serde_json::json!({
            "idempotency_key": "run-request-1",
            "fork_thread_id": "thread-source",
            "fork_source_run_id": "018f854d-2d2c-7363-99a9-804e6cc4a99e"
        });
        assert!(serde_json::from_value::<StartRunRequest>(valid).is_ok());

        for retired_field in ["workspace_id", "readiness_fingerprint", "agent", "purpose"] {
            let mut request = serde_json::json!({
                "idempotency_key": "run-request-1"
            });
            request[retired_field] = serde_json::Value::Null;
            assert!(
                serde_json::from_value::<StartRunRequest>(request).is_err(),
                "retired field {retired_field} must be rejected"
            );
        }
    }
}

/// Safe per-call provider metrics. Prompt, completion content and reasoning
/// are intentionally excluded from persistence and browser DTOs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCallMetric {
    pub id: Uuid,
    pub run_id: Option<Uuid>,
    pub provider_id: String,
    pub model_id: String,
    pub input_tokens: Option<i64>,
    pub cached_input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub tool_schema_tokens: Option<i64>,
    pub latency_ms: Option<i64>,
    pub first_token_ms: Option<i64>,
    pub compaction_count: i32,
    pub terminal_status: String,
    pub created_at: DateTime<Utc>,
}

/// Response from starting a run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartRunResponse {
    pub run: Run,
}

/// Rebuildable, browser-safe view of one Runtime-owned Thread in a root Run's
/// collaboration tree. This resource cannot create, message or stop an Agent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeAgentProjection {
    pub run_id: Uuid,
    pub thread_id: String,
    pub parent_thread_id: Option<String>,
    pub source_kind: String,
    pub agent_path: Option<String>,
    pub agent_nickname: Option<String>,
    pub agent_role: Option<String>,
    pub status_type: Option<String>,
    pub active_flags: Vec<String>,
    pub is_root: bool,
    pub first_observed_at: DateTime<Utc>,
    pub last_observed_at: DateTime<Utc>,
}

/// Browser-safe description of one observable step performed by a
/// Runtime-owned Agent Thread.
///
/// Activities are derived from persisted Run events. They may include bounded,
/// redacted Runtime-provided reasoning text, but omit encrypted reasoning,
/// tool arguments, tool results and host-local identifiers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeAgentActivityKind {
    Assignment,
    Guidance,
    TurnStarted,
    TurnCompleted,
    ToolStarted,
    ToolCompleted,
    ToolFailed,
    Reasoning,
    Reporting,
    Waiting,
    InputRequested,
    InputAnswered,
    Completed,
    Failed,
    Interrupted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeAgentActivityStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Waiting,
}

/// Typed, bounded description of the Runtime item operation represented by an
/// Agent activity.  The item itself remains owned by Codex; this is only the
/// browser-safe descriptor used by the Platform projection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuntimeAgentActivitySubject {
    McpTool {
        server: Option<String>,
        tool: Option<String>,
    },
    RuntimeTool {
        namespace: Option<String>,
        tool: Option<String>,
    },
    WorkspaceAction {
        action: String,
        path: Option<String>,
    },
    WebSearch,
    ImageView,
    ImageGeneration,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeAgentExecutionStatus {
    Pending,
    Running,
    Waiting,
    WaitingForInput,
    Completed,
    Failed,
    Rejected,
    Cancelled,
    Timeout,
    Interrupted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeAgentActivity {
    pub run_id: Uuid,
    pub sequence: i64,
    pub thread_id: String,
    pub turn_id: Option<String>,
    pub item_id: Option<String>,
    pub kind: RuntimeAgentActivityKind,
    pub status: RuntimeAgentActivityStatus,
    pub subject: Option<RuntimeAgentActivitySubject>,
    pub title: String,
    pub detail: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Browser-safe, durable read model of one child Agent task execution.
///
/// The row is derived from Runtime collaboration and Turn events. It is not an
/// Agent scheduler and never owns model-visible Thread state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeAgentExecution {
    pub id: Uuid,
    pub run_id: Uuid,
    pub thread_id: String,
    pub turn_id: Option<String>,
    pub ordinal: i32,
    pub task: Option<String>,
    pub status: RuntimeAgentExecutionStatus,
    pub current_behavior: String,
    pub latest_progress: Option<String>,
    pub display_title: String,
    pub result_summary: Option<String>,
    pub waiting_approval_id: Option<Uuid>,
    pub wait_cycle_count: i32,
    pub first_observed_sequence: i64,
    pub last_observed_sequence: i64,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Durable Artifact lifecycle owned by the Platform.
///
/// This is deliberately narrower than a Runtime or MCP lifecycle: only final
/// user deliverables may enter it, and an intermediate Resource is never an
/// Artifact state.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactState {
    Pending,
    Materializing,
    Ready,
    Failed,
}

impl ArtifactState {
    /// Parse the constrained value persisted by the `artifacts.state` check.
    /// Invalid persisted data is rejected by the owning Server projection
    /// rather than being silently presented as a different state.
    pub fn from_persisted(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "materializing" => Some(Self::Materializing),
            "ready" => Some(Self::Ready),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }

    pub fn is_ready(self) -> bool {
        matches!(self, Self::Ready)
    }
}

/// Allowlisted, browser-safe materialization failure classification.
///
/// The persisted value is an internal diagnostic code. Only this enum and its
/// fixed summary may cross the browser boundary; unknown or future values are
/// intentionally collapsed to `unknown`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactFailureCode {
    SizeLimit,
    WorkspaceReadFailed,
    SizeMismatch,
    ArtifactSchemaUnsupported,
    ArtifactJsonInvalid,
    ArtifactBundleInvalid,
    ArtifactBundleContractMismatch,
    ArtifactContentUnsafe,
    ArtifactDeliveryInvalid,
    ArtifactProjectionFailed,
    Unknown,
}

impl ArtifactFailureCode {
    pub fn from_persisted(value: &str) -> Self {
        match value {
            "size_limit" => Self::SizeLimit,
            "workspace_read_failed" => Self::WorkspaceReadFailed,
            "size_mismatch" => Self::SizeMismatch,
            "artifact_schema_unsupported" => Self::ArtifactSchemaUnsupported,
            "artifact_json_invalid" => Self::ArtifactJsonInvalid,
            "artifact_bundle_invalid" => Self::ArtifactBundleInvalid,
            "artifact_bundle_contract_mismatch" => Self::ArtifactBundleContractMismatch,
            "artifact_content_unsafe" => Self::ArtifactContentUnsafe,
            "artifact_delivery_invalid" => Self::ArtifactDeliveryInvalid,
            "artifact_projection_failed" => Self::ArtifactProjectionFailed,
            _ => Self::Unknown,
        }
    }

    pub fn summary(self) -> &'static str {
        match self {
            Self::SizeLimit => "Artifact exceeds the server safety limit",
            Self::WorkspaceReadFailed => "Artifact source could not be read",
            Self::SizeMismatch => "Artifact size did not match its declaration",
            Self::ArtifactSchemaUnsupported => "Artifact schema is unsupported",
            Self::ArtifactJsonInvalid => "Artifact content is not valid JSON",
            Self::ArtifactBundleInvalid => "Artifact bundle is invalid",
            Self::ArtifactBundleContractMismatch => {
                "Artifact content does not match its declared contract"
            }
            Self::ArtifactContentUnsafe => "Artifact content is not safe to display",
            Self::ArtifactDeliveryInvalid => "Artifact descriptor is invalid",
            Self::ArtifactProjectionFailed => "Artifact could not be registered",
            Self::Unknown => "Artifact materialization failed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactFailureSummary {
    pub code: ArtifactFailureCode,
    pub message: String,
}

impl ArtifactFailureSummary {
    pub fn from_persisted(value: &str) -> Self {
        let code = ArtifactFailureCode::from_persisted(value);
        Self {
            message: code.summary().to_string(),
            code,
        }
    }
}

/// Browser-safe view of one independently authorized, durable Artifact.
///
/// Runtime MCP server names and Resource URIs remain internal. Producer
/// identifiers are provenance only and do not control Artifact lifetime.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactSummary {
    pub id: Uuid,
    pub task_id: Uuid,
    pub artifact_schema: String,
    pub display_name: String,
    pub mime_type: String,
    pub expected_size: Option<i64>,
    pub byte_size: Option<i64>,
    pub content_sha256: Option<String>,
    pub state: ArtifactState,
    pub failure: Option<ArtifactFailureSummary>,
    /// Present only while the Artifact is ready. These are same-origin API
    /// capabilities, not storage paths or provider URLs.
    pub content_url: Option<String>,
    pub download_url: Option<String>,
    pub producer_run_id: Uuid,
    pub producer_thread_id: String,
    pub producer_turn_id: String,
    pub producer_item_id: String,
    pub producer_agent_role: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// An independently authorized execution root. Runtime-local paths remain
/// server-side and are never serialized into this browser-facing projection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub kind: String,
    pub state: String,
    pub source_ref: String,
    pub branch_name: Option<String>,
    pub parent_workspace_id: Option<Uuid>,
    pub group_workspace_id: Option<Uuid>,
    pub managed: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateWorkspaceRequest {
    pub project_id: Uuid,
    pub idempotency_key: String,
    pub kind: String,
    pub name: Option<String>,
    pub source_ref: Option<String>,
    pub parent_workspace_id: Option<Uuid>,
    #[serde(default)]
    pub copy_agents_md: bool,
}

/// A safe Git change projection. Paths are always workspace-relative.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceFileChange {
    pub path: String,
    pub status: String,
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
    pub binary: bool,
    pub size_bytes: Option<u64>,
    pub large: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceStatus {
    pub workspace_id: Uuid,
    pub branch: String,
    pub head_commit: String,
    pub changes: Vec<WorkspaceFileChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitWorkspaceRequest {
    pub selected_paths: Vec<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitWorkspaceResponse {
    pub workspace_id: Uuid,
    pub commit: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFileContent {
    pub content: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFileDiff {
    pub path: String,
    pub diff: String,
    pub is_binary: bool,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspacePathQuery {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspacePathRequest {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFileUploadResponse {
    pub status: String,
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspacePathsRequest {
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceBranchRequest {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameWorkspaceUpstreamRequest {
    pub old_branch: String,
    pub new_branch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenTerminalRequest {
    pub terminal_id: String,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteTerminalRequest {
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResizeTerminalRequest {
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceBranch {
    pub name: String,
    pub last_commit: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceLogEntry {
    pub sha: String,
    pub summary: String,
    pub author: String,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceLog {
    pub total: u64,
    pub entries: Vec<WorkspaceLogEntry>,
    pub ahead: u64,
    pub behind: u64,
    pub ahead_entries: Vec<WorkspaceLogEntry>,
    pub behind_entries: Vec<WorkspaceLogEntry>,
    pub upstream: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceCommitDiff {
    pub path: String,
    pub status: String,
    pub diff: String,
    pub is_binary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubIssue {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubIssues {
    pub total: usize,
    pub issues: Vec<GitHubIssue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubUser {
    pub login: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubPullRequest {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub updated_at: String,
    pub created_at: String,
    pub body: String,
    pub head_ref_name: String,
    pub base_ref_name: String,
    pub is_draft: bool,
    pub author: Option<GitHubUser>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubPullRequests {
    pub total: usize,
    pub pull_requests: Vec<GitHubPullRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubPullRequestDiff {
    pub path: String,
    pub status: String,
    pub diff: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubPullRequestComment {
    pub id: u64,
    pub body: String,
    pub created_at: String,
    pub url: String,
    pub author: Option<GitHubUser>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateGitHubRepositoryRequest {
    pub repo: String,
    pub visibility: String,
    pub branch: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateGitHubRepositoryResponse {
    pub status: String,
    pub repo: String,
    pub remote_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub push_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_branch_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceLogQuery {
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileListQuery {
    pub run_id: Option<Uuid>,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
    pub force_reload: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileProjection {
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileLoginStartResponse {
    pub login_id: String,
    pub auth_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileLoginCancelResponse {
    pub canceled: bool,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileLoginStatusResponse {
    pub completed: bool,
    pub success: Option<bool>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserWorkspacePreference {
    pub workspace_id: Uuid,
    pub settings: serde_json::Value,
    pub runtime_codex_args: Option<String>,
}

// ── Platform configuration ─────────────────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MapsProvider {
    Mapbox,
    Google,
}

/// Browser-visible status for the single selected maps provider. A Mapbox
/// public token is returned only while Mapbox is active because Mapbox GL must
/// use it in the browser. Google credentials are never returned.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MapsConfiguration {
    pub configured: bool,
    pub provider: Option<MapsProvider>,
    pub mapbox_access_token: Option<String>,
    pub can_configure: bool,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateMapsConfigurationRequest {
    pub provider: MapsProvider,
    pub api_key: String,
    /// An authorized pending Platform approval. The server resolves the
    /// credential handoff URL from this record; the browser never receives or
    /// submits that URL.
    pub approval_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UseMapsConfigurationRequest {
    /// An authorized pending Platform approval. The server resolves the
    /// credential handoff URL from this record; the browser never receives or
    /// submits that URL.
    pub approval_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateBrowserWorkspaceSettingsRequest {
    pub settings: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetWorkspaceRuntimeCodexArgsRequest {
    pub codex_args: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetWorkspaceRuntimeCodexArgsResponse {
    pub applied_codex_args: Option<String>,
    pub respawned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeSetupStatus {
    pub should_run: bool,
    pub script: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetWorkspaceGitRootRequest {
    pub git_root: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetThreadNameRequest {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceGitRootsQuery {
    pub depth: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalUsageQuery {
    pub days: Option<u32>,
    pub workspace_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalUsageDay {
    pub day: String,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub agent_time_ms: u64,
    pub agent_runs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalUsageTotals {
    pub last7_days_tokens: u64,
    pub last30_days_tokens: u64,
    pub average_daily_tokens: u64,
    pub cache_hit_rate_percent: f64,
    pub peak_day: Option<String>,
    pub peak_day_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalUsageModel {
    pub model: String,
    pub tokens: u64,
    pub share_percent: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalUsageSnapshot {
    pub updated_at: i64,
    pub days: Vec<LocalUsageDay>,
    pub totals: LocalUsageTotals,
    pub top_models: Vec<LocalUsageModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetExperimentalFeatureRequest {
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileTextFile {
    pub exists: bool,
    pub content: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteProfileTextFileRequest {
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSummary {
    pub name: String,
    pub description: Option<String>,
    pub developer_instructions: Option<String>,
    pub config_file: String,
    pub resolved_path: String,
    pub managed_by_app: bool,
    pub file_exists: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentsSettings {
    pub config_path: String,
    pub multi_agent_enabled: bool,
    pub max_threads: u32,
    pub max_depth: u32,
    pub agents: Vec<AgentSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetAgentsCoreRequest {
    pub multi_agent_enabled: bool,
    pub max_threads: u32,
    pub max_depth: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateAgentRequest {
    pub name: String,
    pub description: Option<String>,
    pub developer_instructions: Option<String>,
    pub template: Option<String>,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAgentRequest {
    pub name: String,
    pub description: Option<String>,
    pub developer_instructions: Option<String>,
    pub rename_managed_file: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteAgentQuery {
    pub delete_managed_file: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptEntry {
    pub name: String,
    pub path: String,
    pub description: Option<String>,
    pub argument_hint: Option<String>,
    pub content: String,
    pub scope: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptListQuery {
    pub run_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatePromptRequest {
    pub run_id: Uuid,
    pub scope: String,
    pub name: String,
    pub description: Option<String>,
    pub argument_hint: Option<String>,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdatePromptRequest {
    pub run_id: Uuid,
    pub path: String,
    pub name: String,
    pub description: Option<String>,
    pub argument_hint: Option<String>,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletePromptRequest {
    pub run_id: Uuid,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovePromptRequest {
    pub run_id: Uuid,
    pub path: String,
    pub scope: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateTextRequest {
    pub kind: String,
    pub input: String,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateTextResponse {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RememberApprovalRuleRequest {
    pub run_id: Uuid,
    pub command: Vec<String>,
}

// ── Messages ──────────────────────────────────────────────────────

/// One explicit, provenance-bound MCP Resource selected by the user. The
/// Platform validates every field against its bounded projection before it is
/// passed back to Codex as an exact provider-owned reference.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExplicitResourceSelection {
    pub producer_event_id: Uuid,
    pub ordinal: i32,
    pub server: String,
    pub uri: String,
    pub resource_schema: String,
}

/// Safe browser selector row for an already completed official Tool Item. It
/// deliberately omits provider Resource content and business interpretation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ResourceReferenceSummary {
    pub producer_event_id: Uuid,
    pub ordinal: i32,
    pub server: String,
    pub uri: String,
    pub resource_schema: String,
    pub display_name: String,
    pub producer_tool: String,
    pub created_at: DateTime<Utc>,
}

/// Request to send a user message to a task's active thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendMessageRequest {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub model_provider: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub service_tier: Option<String>,
    #[serde(default)]
    pub access_mode: Option<String>,
    #[serde(default)]
    pub images: Vec<String>,
    /// One explicit current-Run inline map card selected by the user. The
    /// Platform resolves its exact provider-owned presentation spec after
    /// authorization; this is not a generic Resource attachment channel.
    #[serde(default)]
    #[serde(rename = "mapCardRef")]
    pub map_card_ref: Option<String>,
    /// Exact Resources explicitly selected from authorized producer Item
    /// provenance. This is bounded message input, not a Task-to-Task API.
    #[serde(default)]
    #[serde(rename = "selectedResources")]
    pub selected_resources: Vec<ExplicitResourceSelection>,
    #[serde(default)]
    pub collaboration_mode: Option<serde_json::Value>,
}

/// Response from sending a message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendMessageResponse {
    pub status: String,
    pub thread_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub thread_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterruptRunRequest {
    pub turn_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SteerRunRequest {
    pub turn_id: String,
    pub text: String,
    #[serde(default)]
    pub images: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ReviewTarget {
    UncommittedChanges,
    BaseBranch { branch: String },
    Commit { sha: String, title: Option<String> },
    Custom { instructions: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartReviewRequest {
    pub target: ReviewTarget,
    pub delivery: Option<String>,
}

// ── Approvals ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ApprovalDecision {
    Accept,
    AcceptForSession,
    Decline,
    Cancel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecideApprovalRequest {
    pub decision: ApprovalDecision,
    pub version: i64,
}

/// Browser-safe identity for the Runtime owner of a generic approval.
///
/// The Runtime request id and complete request payload remain server-side.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ApprovalRequestSource {
    Root,
    Agent {
        execution_id: Option<Uuid>,
        display_title: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PendingApprovalState {
    Pending,
    Dispatching,
    DeliveryUnknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PendingApprovalCapability {
    Network,
    FilesystemRead,
    FilesystemWrite,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PendingApprovalFileChangeAction {
    Write,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PendingApprovalUnavailableReason {
    UnsupportedRequest,
    InvalidPermissions,
}

/// Bounded, typed subject details for a Runtime-owned generic approval.
///
/// These fields are safe presentation facts only. Raw Runtime command lines,
/// absolute paths, permission payloads and credential-bearing URLs are never
/// included in this contract.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum PendingApprovalSubject {
    Command {
        action: String,
        path: Option<String>,
    },
    FileChange {
        action: PendingApprovalFileChangeAction,
        path: Option<String>,
    },
    Permissions {
        capabilities: Vec<PendingApprovalCapability>,
    },
    Url {
        server: Option<String>,
        available: bool,
    },
    Unavailable {
        reason: PendingApprovalUnavailableReason,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PendingApprovalSummary {
    pub id: Uuid,
    pub run_id: Uuid,
    pub source: ApprovalRequestSource,
    pub thread_id: String,
    pub turn_id: Option<String>,
    pub item_id: Option<String>,
    pub subject: PendingApprovalSubject,
    pub state: PendingApprovalState,
    pub attempted_decision: Option<ApprovalDecision>,
    pub version: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInputAnswer {
    pub answers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UserInputOptionSummary {
    pub label: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UserInputQuestionSummary {
    pub id: String,
    pub header: String,
    pub question: String,
    pub is_other: bool,
    pub is_secret: bool,
    pub options: Vec<UserInputOptionSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum UserInputRequestSource {
    Root,
    Agent {
        execution_id: Uuid,
        display_title: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PendingUserInputSummary {
    pub id: Uuid,
    pub run_id: Uuid,
    pub source: UserInputRequestSource,
    pub questions: Vec<UserInputQuestionSummary>,
    pub state: String,
    pub version: i64,
    pub auto_resolution_ms: Option<i64>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RespondUserInputRequest {
    pub answers: std::collections::BTreeMap<String, UserInputAnswer>,
    pub version: i64,
}

/// Browser-safe identity for the Runtime owner of an MCP form request.
///
/// Runtime Thread and request ids intentionally remain server-side.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum McpFormRequestSource {
    Root,
    Agent {
        execution_id: Uuid,
        display_title: String,
    },
}

#[cfg(test)]
mod approval_source_serialization_tests {
    use super::{McpFormRequestSource, UserInputRequestSource};
    use serde_json::json;
    use uuid::Uuid;

    #[test]
    fn agent_sources_serialize_browser_field_names() {
        let expected = json!({
            "kind": "agent",
            "executionId": Uuid::nil(),
            "displayTitle": "Data Agent"
        });

        assert_eq!(
            serde_json::to_value(UserInputRequestSource::Agent {
                execution_id: Uuid::nil(),
                display_title: "Data Agent".to_string(),
            })
            .unwrap(),
            expected
        );
        assert_eq!(
            serde_json::to_value(McpFormRequestSource::Agent {
                execution_id: Uuid::nil(),
                display_title: "Data Agent".to_string(),
            })
            .unwrap(),
            expected
        );
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct McpFormOptionSummary {
    pub value: String,
    pub label: String,
}

/// The bounded form field subset rendered by the phase-one browser.
///
/// This is a Platform product DTO, not a passthrough of arbitrary JSON Schema.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum McpFormFieldSchema {
    String {
        default: Option<String>,
        min_length: Option<u32>,
        max_length: Option<u32>,
    },
    Number {
        default: Option<f64>,
        minimum: Option<f64>,
        maximum: Option<f64>,
    },
    Integer {
        default: Option<i64>,
        minimum: Option<i64>,
        maximum: Option<i64>,
    },
    Boolean {
        default: Option<bool>,
    },
    SingleSelect {
        options: Vec<McpFormOptionSummary>,
        default: Option<String>,
    },
    MultiSelect {
        options: Vec<McpFormOptionSummary>,
        default: Option<Vec<String>>,
        min_items: Option<u32>,
        max_items: Option<u32>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct McpFormFieldSummary {
    pub name: String,
    pub title: String,
    pub description: String,
    pub required: bool,
    pub schema: McpFormFieldSchema,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PendingMcpFormSummary {
    pub id: Uuid,
    pub run_id: Uuid,
    pub source: McpFormRequestSource,
    pub server_name: String,
    pub message: String,
    pub fields: Vec<McpFormFieldSummary>,
    pub state: String,
    pub version: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum McpFormResponseAction {
    Accept,
    Decline,
    Cancel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RespondMcpFormRequest {
    pub action: McpFormResponseAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<std::collections::BTreeMap<String, serde_json::Value>>,
    pub version: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalSummary {
    pub id: Uuid,
    pub run_id: Uuid,
    pub thread_id: String,
    pub request_type: String,
    pub item_id: Option<String>,
    pub reason: Option<String>,
    pub command: Option<String>,
    pub state: String,
    pub version: i64,
    pub created_at: DateTime<Utc>,
    pub decided_at: Option<DateTime<Utc>>,
}

// ── Model Providers ──────────────────────────────────────────────

/// Stable platform projection of a Codex model Provider kind.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ProviderKind {
    BuiltIn,
    Local,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderModelSummary {
    pub model_id: String,
    pub model_name: Option<String>,
    pub max_token_len: Option<i64>,
    pub max_output_tokens: Option<i64>,
    pub show_in_picker: bool,
    pub context_window: Option<i64>,
    #[serde(default)]
    pub supports_search_tool: bool,
}

/// Provider facts returned to browsers. Credential values are intentionally
/// absent; only the configured environment-variable name may be projected.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSummary {
    pub id: String,
    pub name: String,
    pub base_url: Option<String>,
    pub env_key: Option<String>,
    pub wire_api: String,
    /// Explicit configured capability for function-tool calls on this
    /// Provider. This is a declaration, not a live capability probe.
    pub supports_function_tools: bool,
    pub kind: ProviderKind,
    pub is_current: bool,
    pub model_count: usize,
    pub can_edit: bool,
    pub can_delete: bool,
    pub can_fetch_models: bool,
    pub models: Vec<ProviderModelSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCatalog {
    pub data: Vec<ProviderSummary>,
    pub current_provider_id: String,
    #[serde(default)]
    pub current_model_id: Option<String>,
}

// ── Copilot Profile installation ──────────────────────────────────────────

/// Current lifecycle projection for one Profile installation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CopilotInstallationState {
    Installed,
    Configured,
    Unavailable,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CopilotInstallationSummary {
    pub package_id: String,
    pub source_revision: String,
    pub active: bool,
    pub state: CopilotInstallationState,
    pub restart_required: bool,
    pub managed_skill_ids: Vec<String>,
    pub managed_agent_role_ids: Vec<String>,
    pub runtime_discovered_skill_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_code: Option<String>,
}

/// Activates one package from the server's explicitly registered application
/// sources. Filesystem paths are intentionally absent from this browser DTO.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivateCopilotRequest {
    pub package_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AvailableCopilotPackage {
    pub package_id: String,
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CopilotProfileStatus {
    pub packages: Vec<AvailableCopilotPackage>,
    #[serde(default)]
    pub installations: Vec<CopilotInstallationSummary>,
}

#[cfg(test)]
mod copilot_installation_contract_tests {
    use super::*;

    #[test]
    fn activation_accepts_only_a_package_id_and_never_a_server_path() {
        let request: ActivateCopilotRequest = serde_json::from_value(serde_json::json!({
            "packageId": "private-package"
        }))
        .expect("typed activation");
        assert_eq!(request.package_id, "private-package");
        assert!(
            serde_json::from_value::<ActivateCopilotRequest>(serde_json::json!({
                "packageId": "private-package",
                "packageRoot": "/private/server/package"
            }))
            .is_err()
        );
    }

    #[test]
    fn copilot_installation_has_configured_state_without_ready_or_mcp_fields() {
        let summary = CopilotInstallationSummary {
            package_id: "sample".to_string(),
            source_revision: "a".repeat(64),
            active: true,
            state: CopilotInstallationState::Configured,
            restart_required: false,
            managed_skill_ids: vec!["root".to_string()],
            managed_agent_role_ids: vec!["worker".to_string()],
            runtime_discovered_skill_ids: vec!["root".to_string()],
            failure_code: None,
        };
        let value = serde_json::to_value(&summary).expect("serialize installation summary");
        assert_eq!(value["state"], "configured");
        assert!(value.get("agentRolesConfigured").is_none());
        assert!(value.get("runtimeDiscoveredMcpServerIds").is_none());
        assert!(
            serde_json::from_value::<CopilotInstallationState>(serde_json::json!("ready")).is_err()
        );
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "mode",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ProviderCredentialInput {
    Preserve,
    Environment {
        env_key: String,
    },
    Direct {
        api_key: String,
    },
    #[serde(rename = "none")]
    NoCredential,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertProviderRequest {
    pub name: String,
    pub base_url: String,
    pub wire_api: String,
    pub credentials: ProviderCredentialInput,
    /// Browser-projected capability echo. Provider Service derives the
    /// effective value from `wire_api`: Chat is enabled, Responses is disabled.
    #[serde(default)]
    pub supports_function_tools: Option<bool>,
    #[serde(default)]
    pub select: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProviderModelRequest {
    pub context_window: i64,
    /// Explicit native ToolSearch capability for this exact model.
    #[serde(default)]
    pub supports_search_tool: Option<bool>,
}

// ── Events ────────────────────────────────────────────────────────

/// A persisted run event returned by the task events endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunEvent {
    pub id: Uuid,
    pub sequence: i64,
    pub run_id: Uuid,
    pub event_type: String,
    pub projection_version: i16,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub item_id: Option<String>,
    pub payload: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// Query parameters for listing task events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListTaskEventsParams {
    pub limit: Option<i64>,
    pub after_sequence: Option<i64>,
}
