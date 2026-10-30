export type User = {
  id: string;
  name: string;
  username: string;
  email: string;
  role: string;
};

export type Organization = {
  id: string;
  name: string;
  slug: string;
};

export type Session = {
  user: User;
  organization: Organization;
  session_token: string;
  membership_role?: string;
};

export type Me = User & {
  organization_id: string;
  organization_role: string;
};

export type Project = {
  id: string;
  name: string;
  git_url: string;
  default_branch: string;
  created_at: string;
  updated_at: string;
};

export type Task = {
  id: string;
  project_id: string;
  workspace_id: string;
  title: string;
  status: string;
  model_provider: string | null;
  model: string | null;
  copilot_package_id: string | null;
  created_at: string;
  updated_at: string;
};

export type AvailableCopilotPackage = {
  packageId: string;
  available: boolean;
  displayName?: string;
};

export type CopilotInstallationState =
  | "installed"
  | "configured"
  | "unavailable"
  | "failed";

export type CopilotInstallationSummary = {
  packageId: string;
  sourceRevision: string;
  active: boolean;
  state: CopilotInstallationState;
  restartRequired: boolean;
  managedSkillIds: string[];
  managedAgentRoleIds: string[];
  runtimeDiscoveredSkillIds: string[];
  failureCode?: string;
};

export type CopilotProfileStatus = {
  packages: AvailableCopilotPackage[];
  installations: CopilotInstallationSummary[];
};

export type ExplicitResourceSelection = {
  producerEventId: string;
  ordinal: number;
  server: string;
  uri: string;
  resourceSchema: string;
};

export type ResourceReferenceSummary = ExplicitResourceSelection & {
  displayName: string;
  producerTool: string;
  createdAt: string;
};

export type Run = {
  id: string;
  task_id: string;
  status: string;
  failure_code: RunFailureCode | null;
  codex_thread_id: string | null;
  active_turn_id: string | null;
  workspace_id: string;
  attempt: number;
  created_at: string;
  updated_at: string;
};

export type RunFailureCode =
  | "invalid_run"
  | "resource_not_found"
  | "run_conflict"
  | "lease_lost"
  | "database_error"
  | "git_workspace_error"
  | "codex_unavailable"
  | "run_cancelled"
  | "interrupt_failed"
  | "lease_expired"
  | "unknown_failure";

export type RuntimeAgentProjection = {
  run_id: string;
  thread_id: string;
  parent_thread_id: string | null;
  source_kind: string;
  agent_path: string | null;
  agent_nickname: string | null;
  agent_role: string | null;
  status_type: string | null;
  active_flags: string[];
  is_root: boolean;
  first_observed_at: string;
  last_observed_at: string;
};

export type RuntimeAgentActivityKind =
  | "assignment"
  | "guidance"
  | "turn_started"
  | "turn_completed"
  | "tool_started"
  | "tool_completed"
  | "tool_failed"
  | "reasoning"
  | "reporting"
  | "waiting"
  | "input_requested"
  | "input_answered"
  | "completed"
  | "failed"
  | "rejected"
  | "cancelled"
  | "timeout"
  | "interrupted";

export type RuntimeAgentActivitySubject =
  | { kind: "mcp_tool"; server: string | null; tool: string | null }
  | { kind: "runtime_tool"; namespace: string | null; tool: string | null }
  | { kind: "workspace_action"; action: string; path: string | null }
  | { kind: "web_search" }
  | { kind: "image_view" }
  | { kind: "image_generation" };

export type RuntimeAgentActivity = {
  run_id: string;
  sequence: number;
  thread_id: string;
  turn_id: string | null;
  item_id: string | null;
  kind: RuntimeAgentActivityKind;
  status: "pending" | "running" | "completed" | "failed" | "waiting";
  subject: RuntimeAgentActivitySubject | null;
  title: string;
  detail: string | null;
  created_at: string;
};

export type RuntimeAgentExecution = {
  id: string;
  run_id: string;
  thread_id: string;
  turn_id: string | null;
  ordinal: number;
  task: string | null;
  status: RuntimeAgentExecutionStatus;
  current_behavior: string;
  latest_progress: string | null;
  display_title: string;
  result_summary: string | null;
  waiting_approval_id: string | null;
  wait_cycle_count: number;
  first_observed_sequence: number;
  last_observed_sequence: number;
  started_at: string | null;
  completed_at: string | null;
  created_at: string;
  updated_at: string;
};

export type RuntimeAgentExecutionStatus =
  | "pending"
  | "running"
  | "waiting"
  | "waiting_for_input"
  | "completed"
  | "failed"
  | "rejected"
  | "cancelled"
  | "timeout"
  | "interrupted";

export type ProviderCallMetric = {
  id: string;
  runId: string | null;
  providerId: string;
  modelId: string;
  inputTokens: number | null;
  cachedInputTokens: number | null;
  outputTokens: number | null;
  toolSchemaTokens: number | null;
  latencyMs: number | null;
  firstTokenMs: number | null;
  compactionCount: number;
  terminalStatus: string;
  createdAt: string;
};

export type UserInputOptionSummary = {
  label: string;
  description: string;
};

export type UserInputQuestionSummary = {
  id: string;
  header: string;
  question: string;
  isOther: boolean;
  isSecret: boolean;
  options: UserInputOptionSummary[];
};

export type UserInputRequestSource =
  | { kind: "root" }
  | { kind: "agent"; executionId: string; displayTitle: string };

export type PendingUserInputSummary = {
  id: string;
  runId: string;
  source: UserInputRequestSource;
  questions: UserInputQuestionSummary[];
  state: string;
  version: number;
  autoResolutionMs: number | null;
  createdAt: string;
};

export type ApprovalRequestSource =
  | { kind: "root" }
  | { kind: "agent"; executionId: string | null; displayTitle: string };

export type ApprovalDecision =
  | "accept"
  | "acceptForSession"
  | "decline"
  | "cancel";

export type PendingApprovalState = "pending" | "dispatching" | "delivery_unknown";

export type PendingApprovalCapability =
  | "network"
  | "filesystem_read"
  | "filesystem_write";

export type PendingApprovalFileChangeAction = "write";

export type PendingApprovalUnavailableReason =
  | "unsupported_request"
  | "invalid_permissions";

export type PendingApprovalSubject =
  | {
      kind: "command";
      action: string;
      path: string | null;
    }
  | {
      kind: "file_change";
      action: PendingApprovalFileChangeAction;
      path: string | null;
    }
  | { kind: "permissions"; capabilities: PendingApprovalCapability[] }
  | {
      kind: "url";
      server: string | null;
      available: boolean;
    }
  | {
      kind: "unavailable";
      reason: PendingApprovalUnavailableReason;
    };

export type PendingApprovalSummary = {
  id: string;
  runId: string;
  source: ApprovalRequestSource;
  threadId: string;
  turnId: string | null;
  itemId: string | null;
  subject: PendingApprovalSubject;
  state: PendingApprovalState;
  attemptedDecision: ApprovalDecision | null;
  version: number;
  createdAt: string;
};

export type McpFormRequestSource =
  | { kind: "root" }
  | { kind: "agent"; executionId: string; displayTitle: string };

export type McpFormOptionSummary = {
  value: string;
  label: string;
};

export type McpFormFieldSchema =
  | { kind: "string"; default: string | null; minLength: number | null; maxLength: number | null }
  | { kind: "number"; default: number | null; minimum: number | null; maximum: number | null }
  | { kind: "integer"; default: number | null; minimum: number | null; maximum: number | null }
  | { kind: "boolean"; default: boolean | null }
  | { kind: "singleSelect"; options: McpFormOptionSummary[]; default: string | null }
  | {
      kind: "multiSelect";
      options: McpFormOptionSummary[];
      default: string[] | null;
      minItems: number | null;
      maxItems: number | null;
    };

export type McpFormFieldSummary = {
  name: string;
  title: string;
  description: string;
  required: boolean;
  schema: McpFormFieldSchema;
};

export type PendingMcpFormSummary = {
  id: string;
  runId: string;
  source: McpFormRequestSource;
  serverName: string;
  message: string;
  fields: McpFormFieldSummary[];
  state: string;
  version: number;
  createdAt: string;
};

export type McpFormResponseAction = "accept" | "decline" | "cancel";

export type McpFormContent = Record<string, string | number | boolean | string[]>;

export type ArtifactState = "pending" | "materializing" | "ready" | "failed";

export type ArtifactFailureCode =
  | "size_limit"
  | "workspace_read_failed"
  | "size_mismatch"
  | "artifact_schema_unsupported"
  | "artifact_json_invalid"
  | "artifact_bundle_invalid"
  | "artifact_bundle_contract_mismatch"
  | "unknown";

export type ArtifactFailureSummary = {
  code: ArtifactFailureCode;
  message: string;
};

export type ArtifactSummary = {
  id: string;
  task_id: string;
  artifact_schema: string;
  display_name: string;
  mime_type: string;
  expected_size: number | null;
  byte_size: number | null;
  content_sha256: string | null;
  state: ArtifactState;
  failure: ArtifactFailureSummary | null;
  content_url: string | null;
  download_url: string | null;
  producer_run_id: string;
  producer_thread_id: string;
  producer_turn_id: string;
  producer_item_id: string;
  producer_agent_role: string | null;
  created_at: string;
  updated_at: string;
};

export type ArtifactContent =
  | {
      kind: "json";
      mime_type: "application/json" | "application/geo+json";
      value: Record<string, unknown>;
    }
  | {
      kind: "markdown";
      mime_type: "text/markdown";
      text: string;
    };

export type Workspace = {
  id: string;
  project_id: string;
  name: string;
  kind: "main" | "worktree" | "clone";
  state: "creating" | "ready" | "retained" | "removing" | "removed" | "cleanup_failed";
  source_ref: string;
  branch_name: string | null;
  parent_workspace_id: string | null;
  group_workspace_id: string | null;
  managed: boolean;
  created_at: string;
  updated_at: string;
};

export type ProjectThreadContext = {
  project: Project;
  task: Task;
  run: Run;
};

export type ThreadHistoryTurn = {
  id: string;
  status: string;
  items: Record<string, unknown>[];
  error?: { message: string; additionalDetails?: string | null } | null;
  startedAt?: number | null;
  completedAt?: number | null;
  durationMs?: number | null;
};

export type ThreadTokenUsageBreakdown = {
  totalTokens: number;
  inputTokens: number;
  cachedInputTokens: number;
  outputTokens: number;
  reasoningOutputTokens: number;
};

export type ThreadTokenUsageResponse = {
  tokenUsage: {
    total: ThreadTokenUsageBreakdown;
    last: ThreadTokenUsageBreakdown;
    modelContextWindow: number | null;
  } | null;
};

export type ThreadHistoryResponse = {
  thread: {
    id: string;
    name?: string | null;
    preview: string;
    createdAt: number;
    updatedAt: number;
    status: { type: string; activeFlags?: string[] };
    turns: ThreadHistoryTurn[];
  };
};

export type RunEvent = {
  id: string;
  sequence: number;
  run_id: string;
  event_type: string;
  projection_version: number;
  thread_id: string | null;
  turn_id: string | null;
  item_id: string | null;
  payload: {
    schemaVersion?: number;
    lifecycle?: string;
    itemType?: string | null;
    data?: unknown;
  };
  created_at: string;
};

export type Approval = {
  id: string;
  runId: string;
  threadId: string;
  requestType: string;
  itemId?: string | null;
  reason?: string | null;
  command?: string | null;
  state: string;
  version: number;
  createdAt: string;
  decidedAt?: string | null;
};

export type WorkspaceChange = {
  path: string;
  status: string;
  additions: number | null;
  deletions: number | null;
  binary: boolean;
  size_bytes: number | null;
  large: boolean;
};

export type WorkspaceStatus = {
  workspace_id: string;
  branch: string;
  head_commit: string;
  changes: WorkspaceChange[];
};

export type WorkspaceFileContent = {
  content: string;
  truncated: boolean;
};

export type WorkspaceFileUploadResponse = {
  status: string;
  paths: string[];
};

export type WorkspaceFileDiff = {
  path: string;
  diff: string;
  isBinary: boolean;
  truncated: boolean;
};

export type WorkspaceBranch = {
  name: string;
  lastCommit: number;
};

export type WorkspaceLogEntry = {
  sha: string;
  summary: string;
  author: string;
  timestamp: number;
};

export type WorkspaceLog = {
  total: number;
  entries: WorkspaceLogEntry[];
  ahead: number;
  behind: number;
  aheadEntries: WorkspaceLogEntry[];
  behindEntries: WorkspaceLogEntry[];
  upstream: string | null;
};

export type WorkspaceCommitDiff = {
  path: string;
  status: string;
  diff: string;
  isBinary: boolean;
};

export type ProviderModel = {
  modelId: string;
  modelName?: string | null;
  showInPicker: boolean;
  contextWindow?: number | null;
  supportsSearchTool: boolean;
};

export type Provider = {
  id: string;
  name: string;
  wireApi: string;
  supportsFunctionTools: boolean;
  kind: "builtIn" | "local" | "custom";
  isCurrent: boolean;
  modelCount: number;
  models: ProviderModel[];
};

export type ProviderCatalog = {
  data: Provider[];
  currentProviderId: string;
  currentModelId?: string | null;
};

export type ModelSelection = {
  providerId: string;
  modelId: string;
};

export type ProfileTextFile = {
  exists: boolean;
  content: string;
  truncated: boolean;
};

export type ProfileLoginStart = { loginId: string; authUrl: string };
export type ProfileLoginCancel = { canceled: boolean; status: string };
export type ProfileLoginStatus = {
  completed: boolean;
  success: boolean | null;
  error: string | null;
};

export type BrowserWorkspacePreference = {
  workspaceId: string;
  settings: Record<string, unknown>;
  runtimeCodexArgs: string | null;
};

export type MapsProvider = "mapbox" | "google";

export type MapsConfiguration = {
  configured: boolean;
  provider: MapsProvider | null;
  mapboxAccessToken: string | null;
  canConfigure: boolean;
  updatedAt: string | null;
};

export type AgentSummary = {
  name: string;
  description: string | null;
  developerInstructions: string | null;
  configFile: string;
  resolvedPath: string;
  managedByApp: boolean;
  fileExists: boolean;
};

export type AgentsSettings = {
  configPath: string;
  multiAgentEnabled: boolean;
  maxThreads: number;
  maxDepth: number;
  agents: AgentSummary[];
};

export type PromptEntry = {
  name: string;
  path: string;
  description: string | null;
  argumentHint: string | null;
  content: string;
  scope: "workspace" | "global";
};

export type GitHubIssue = {
  number: number;
  title: string;
  url: string;
  updatedAt: string;
};

export type GitHubIssues = { total: number; issues: GitHubIssue[] };
export type GitHubUser = { login: string };

export type GitHubPullRequest = {
  number: number;
  title: string;
  url: string;
  updatedAt: string;
  createdAt: string;
  body: string;
  headRefName: string;
  baseRefName: string;
  isDraft: boolean;
  author: GitHubUser | null;
};

export type GitHubPullRequests = { total: number; pullRequests: GitHubPullRequest[] };
export type GitHubPullRequestDiff = { path: string; status: string; diff: string };
export type GitHubPullRequestComment = {
  id: number;
  body: string;
  createdAt: string;
  url: string;
  author: GitHubUser | null;
};

export type CreateGitHubRepositoryResponse = {
  status: "ok" | "partial";
  repo: string;
  remoteUrl: string | null;
  pushError?: string | null;
  defaultBranchError?: string | null;
};
