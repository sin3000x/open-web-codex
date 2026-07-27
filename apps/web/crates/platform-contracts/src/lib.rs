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
    pub title: String,
    pub status: String,
    pub model_provider: Option<String>,
    pub model: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Request body to create a task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateTaskRequest {
    pub project_id: Uuid,
    pub title: String,
    #[serde(default)]
    pub model_provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
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

/// Database/API run representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Run {
    pub id: Uuid,
    pub task_id: Uuid,
    pub status: String,
    pub codex_thread_id: Option<String>,
    pub active_turn_id: Option<String>,
    pub workspace_id: Option<Uuid>,
    pub source_ref: Option<String>,
    pub workspace_kind: String,
    pub workspace_name: Option<String>,
    pub workspace_parent_run_id: Option<Uuid>,
    pub workspace_group_run_id: Option<Uuid>,
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

/// Request to start a run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartRunRequest {
    pub idempotency_key: String,
    pub git_ref: Option<String>,
    #[serde(default)]
    pub workspace_kind: Option<String>,
    #[serde(default)]
    pub workspace_name: Option<String>,
    #[serde(default)]
    pub workspace_parent_run_id: Option<Uuid>,
    #[serde(default)]
    pub workspace_group_run_id: Option<Uuid>,
    #[serde(default)]
    pub copy_agents_md: bool,
    #[serde(default)]
    pub fork_thread_id: Option<String>,
    #[serde(default)]
    pub fork_source_run_id: Option<Uuid>,
}

/// Response from starting a run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartRunResponse {
    pub run: Run,
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
pub struct RunWorkspaceStatus {
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
    pub elicitation_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UseMapsConfigurationRequest {
    pub elicitation_url: String,
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

/// Request to send a user message to a task's active thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendMessageRequest {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInputAnswer {
    pub answers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RespondUserInputRequest {
    pub answers: std::collections::BTreeMap<String, UserInputAnswer>,
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
    #[serde(default)]
    pub select: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProviderModelRequest {
    pub context_window: i64,
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
