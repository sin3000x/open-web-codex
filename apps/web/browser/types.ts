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
  title: string;
  status: string;
  model_provider: string | null;
  model: string | null;
  created_at: string;
  updated_at: string;
};

export type Run = {
  id: string;
  task_id: string;
  status: string;
  codex_thread_id: string | null;
  active_turn_id: string | null;
  workspace_id: string | null;
  source_ref: string | null;
  workspace_kind: "main" | "worktree" | "clone";
  workspace_name: string | null;
  workspace_parent_run_id: string | null;
  workspace_group_run_id: string | null;
  attempt: number;
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
};

export type Provider = {
  id: string;
  name: string;
  wireApi: string;
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
