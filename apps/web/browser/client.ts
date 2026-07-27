import type {
  Approval,
  Me,
  Project,
  ProviderCatalog,
  Run,
  RunEvent,
  Session,
  Task,
  WorkspaceStatus,
  WorkspaceFileContent,
  WorkspaceFileDiff,
  WorkspaceBranch,
  WorkspaceLog,
  WorkspaceCommitDiff,
  ProfileTextFile,
  AgentsSettings,
  PromptEntry,
  GitHubIssues,
  GitHubPullRequests,
  GitHubPullRequestDiff,
  GitHubPullRequestComment,
  ProfileLoginStart,
  ProfileLoginCancel,
  ProfileLoginStatus,
  ProjectThreadContext,
  ThreadHistoryResponse,
  ThreadHistoryTurn,
  BrowserWorkspacePreference,
  CreateGitHubRepositoryResponse,
  MapsConfiguration,
  MapsProvider,
} from "./types";

type ClientOptions = {
  baseUrl?: string;
  token?: string;
};

type LiveEnvelope =
  | { type: "ready"; version: number }
  | { type: "run.event"; version: number; event: RunEvent }
  | { type: "resyncRequired"; version: number }
  | { type: "error"; version: number; code: string };

function defaultBaseUrl() {
  return (import.meta.env.VITE_PLATFORM_API_URL ?? "").replace(/\/$/, "");
}

const createIdempotencyKey = () => {
  const randomUUID = globalThis.crypto?.randomUUID;
  if (typeof randomUUID === "function") {
    return randomUUID.call(globalThis.crypto);
  }
  return `idempotency-${Date.now()}-${Math.random().toString(16).slice(2)}`;
};

export class PlatformClient {
  private baseUrl: string;
  private token: string;

  constructor(options: ClientOptions = {}) {
    this.baseUrl = (options.baseUrl ?? defaultBaseUrl()).replace(/\/$/, "");
    this.token = options.token?.trim() ?? "";
  }

  setToken(token: string) {
    this.token = token.trim();
  }

  setBaseUrl(baseUrl: string) {
    this.baseUrl = baseUrl.trim().replace(/\/$/, "");
  }

  private async request<T>(path: string, init?: RequestInit): Promise<T> {
    const response = await fetch(`${this.baseUrl}${path}`, {
      ...init,
      cache: "no-store",
      headers: {
        ...(init?.body ? { "content-type": "application/json" } : {}),
        ...(this.token ? { authorization: `Bearer ${this.token}` } : {}),
        ...init?.headers,
      },
    });
    const text = await response.text();
    let payload: unknown = null;
    if (text) {
      try {
        payload = JSON.parse(text) as unknown;
      } catch {
        throw new Error(
          `Server returned a non-JSON response for ${path} (HTTP ${response.status}, ${response.url}).`,
        );
      }
    }
    if (!response.ok) {
      const record = payload && typeof payload === "object"
        ? payload as Record<string, unknown>
        : null;
      const message = typeof record?.message === "string"
        ? record.message
        : `Request failed (HTTP ${response.status}).`;
      throw new Error(message);
    }
    return payload as T;
  }

  health() {
    return this.request<{ ok: boolean; version: string }>("/api/health");
  }

  bootstrap(name: string, username: string, email: string, password: string) {
    return this.request<Session>("/api/bootstrap", {
      method: "POST",
      body: JSON.stringify({ name, username, email, password }),
    });
  }

  login(username: string, password: string) {
    return this.request<Session>("/api/sessions", {
      method: "POST",
      body: JSON.stringify({ username, password }),
    });
  }

  createLocalSession() {
    return this.request<Session>("/api/sessions/local", {
      method: "POST",
    });
  }

  me() {
    return this.request<Me>("/api/me");
  }

  listBrowserWorkspacePreferences() {
    return this.request<BrowserWorkspacePreference[]>("/api/browser-workspace-preferences");
  }

  updateBrowserWorkspaceSettings(workspaceId: string, settings: Record<string, unknown>) {
    return this.request<BrowserWorkspacePreference>(
      `/api/browser-workspace-preferences/${encodeURIComponent(workspaceId)}`,
      { method: "PUT", body: JSON.stringify({ settings }) },
    );
  }

  setWorkspaceRuntimeCodexArgs(workspaceId: string, codexArgs: string | null) {
    return this.request<{ appliedCodexArgs: string | null; respawned: boolean }>(
      `/api/browser-workspace-preferences/${encodeURIComponent(workspaceId)}/runtime-codex-args`,
      { method: "PUT", body: JSON.stringify({ codexArgs }) },
    );
  }

  worktreeSetupStatus(workspaceId: string) {
    return this.request<{ shouldRun: boolean; script: string | null }>(
      `/api/browser-workspace-preferences/${encodeURIComponent(workspaceId)}/worktree-setup`,
    );
  }

  markWorktreeSetupRan(workspaceId: string) {
    return this.request<{ status: string }>(
      `/api/browser-workspace-preferences/${encodeURIComponent(workspaceId)}/worktree-setup`,
      { method: "POST" },
    );
  }

  getMapsConfiguration() {
    return this.request<MapsConfiguration>("/api/configuration/maps");
  }

  updateMapsConfiguration(
    provider: MapsProvider,
    apiKey: string,
    elicitationUrl?: string,
  ) {
    return this.request<MapsConfiguration>("/api/configuration/maps", {
      method: "PUT",
      body: JSON.stringify({ provider, apiKey, elicitationUrl }),
    });
  }

  useMapsConfiguration(elicitationUrl: string) {
    return this.request<MapsConfiguration>("/api/configuration/maps/use", {
      method: "POST",
      body: JSON.stringify({ elicitationUrl }),
    });
  }

  listProjects() {
    return this.request<Project[]>("/api/projects");
  }

  getProject(projectId: string) {
    return this.request<Project>(`/api/projects/${encodeURIComponent(projectId)}`);
  }

  listProjectThreadContexts(projectId: string) {
    return this.request<ProjectThreadContext[]>(
      `/api/projects/${encodeURIComponent(projectId)}/thread-contexts`,
    );
  }

  deleteProject(projectId: string) {
    return this.request<{ deleted: boolean; id: string }>(
      `/api/projects/${encodeURIComponent(projectId)}`,
      { method: "DELETE" },
    );
  }

  createProject(name: string, gitUrl: string, defaultBranch: string) {
    return this.request<Project>("/api/projects", {
      method: "POST",
      body: JSON.stringify({ name, git_url: gitUrl, default_branch: defaultBranch }),
    });
  }

  createManagedProject(name: string) {
    return this.request<Project>("/api/projects/managed", {
      method: "POST",
      body: JSON.stringify({ name }),
    });
  }

  listTasks(projectId: string) {
    return this.request<Task[]>(`/api/tasks?project_id=${encodeURIComponent(projectId)}`);
  }

  createTask(
    projectId: string,
    title: string,
    selection?: { providerId: string; modelId: string } | null,
  ) {
    return this.request<Task>("/api/tasks", {
      method: "POST",
      body: JSON.stringify({
        project_id: projectId,
        title,
        model_provider: selection?.providerId ?? null,
        model: selection?.modelId ?? null,
      }),
    });
  }

  updateTaskModelSelection(taskId: string, providerId: string, modelId: string) {
    return this.request<{ providerId: string; modelId: string }>(
      `/api/tasks/${encodeURIComponent(taskId)}/model-selection`,
      {
        method: "PUT",
        body: JSON.stringify({ providerId, modelId }),
      },
    );
  }

  getTask(taskId: string) {
    return this.request<Task>(`/api/tasks/${encodeURIComponent(taskId)}`);
  }

  listRuns(taskId: string) {
    return this.request<Run[]>(`/api/runs?task_id=${encodeURIComponent(taskId)}`);
  }

  getRun(runId: string) {
    return this.request<Run>(`/api/runs/${encodeURIComponent(runId)}`);
  }

  readRunThread(runId: string) {
    return this.request<ThreadHistoryResponse>(`/api/runs/${encodeURIComponent(runId)}/thread`);
  }

  listRunThreadTurns(runId: string) {
    return this.request<ThreadHistoryTurn[]>(
      `/api/runs/${encodeURIComponent(runId)}/thread/turns`,
    );
  }

  readReplyArtifact(path: string) {
    if (!/^\/api\/runs\/[0-9a-f-]+\/artifacts\/[0-9a-f-]+$/i.test(path)) {
      return Promise.reject(new Error("Reply Artifact path is invalid."));
    }
    return this.request<Record<string, unknown>>(path);
  }

  archiveRunThread(runId: string) {
    return this.request<{ status: string }>(
      `/api/runs/${encodeURIComponent(runId)}/thread/archive`,
      { method: "POST" },
    );
  }

  setRunThreadName(runId: string, name: string) {
    return this.request<{ status: string; name: string }>(
      `/api/runs/${encodeURIComponent(runId)}/thread/name`,
      { method: "PUT", body: JSON.stringify({ name }) },
    );
  }

  startRun(
    taskId: string,
    gitRef?: string,
    workspace?: {
      kind: "main" | "worktree" | "clone";
      name?: string | null;
      parentRunId?: string | null;
      groupRunId?: string | null;
      copyAgentsMd?: boolean;
      forkThreadId?: string | null;
      forkSourceRunId?: string | null;
    },
  ) {
    return this.request<{ run: Run }>(`/api/tasks/${encodeURIComponent(taskId)}/runs`, {
      method: "POST",
      body: JSON.stringify({
        idempotency_key: createIdempotencyKey(),
        git_ref: gitRef?.trim() || null,
        workspace_kind: workspace?.kind ?? "main",
        workspace_name: workspace?.name?.trim() || null,
        workspace_parent_run_id: workspace?.parentRunId ?? null,
        workspace_group_run_id: workspace?.groupRunId ?? null,
        copy_agents_md: workspace?.copyAgentsMd ?? false,
        fork_thread_id: workspace?.forkThreadId ?? null,
        fork_source_run_id: workspace?.forkSourceRunId ?? null,
      }),
    });
  }

  cancelRun(runId: string) {
    return this.request<Run>(`/api/runs/${encodeURIComponent(runId)}/cancel`, {
      method: "POST",
    });
  }

  sendMessage(
    taskId: string,
    text: string,
    options: {
      model?: string | null;
      modelProvider?: string | null;
      effort?: string | null;
      serviceTier?: string | null;
      accessMode?: string | null;
      images?: string[];
      collaborationMode?: Record<string, unknown> | null;
    } = {},
  ) {
    return this.request<{
      status: string;
      thread_id: string;
      turn_id: string;
      thread_name?: string | null;
    }>(
      `/api/tasks/${encodeURIComponent(taskId)}/messages`,
      {
        method: "POST",
        body: JSON.stringify({
          text,
          model: options.model ?? null,
          model_provider: options.modelProvider ?? null,
          effort: options.effort ?? null,
          service_tier: options.serviceTier ?? null,
          access_mode: options.accessMode ?? null,
          images: options.images ?? [],
          collaboration_mode: options.collaborationMode ?? null,
        }),
      },
    );
  }

  interruptRun(runId: string, turnId: string) {
    return this.request<{ status: string }>(`/api/runs/${encodeURIComponent(runId)}/interrupt`, {
      method: "POST",
      body: JSON.stringify({ turn_id: turnId }),
    });
  }

  steerRun(runId: string, turnId: string, text: string, images: string[] = []) {
    return this.request<{ status: string }>(`/api/runs/${encodeURIComponent(runId)}/steer`, {
      method: "POST",
      body: JSON.stringify({ turn_id: turnId, text, images }),
    });
  }

  compactRunThread(runId: string) {
    return this.request<{ status: string }>(`/api/runs/${encodeURIComponent(runId)}/compact`, {
      method: "POST",
    });
  }

  startRunReview(
    runId: string,
    target: Record<string, unknown>,
    delivery?: "inline" | "detached" | null,
  ) {
    return this.request<Record<string, unknown>>(
      `/api/runs/${encodeURIComponent(runId)}/review`,
      {
        method: "POST",
        body: JSON.stringify({ target, delivery: delivery ?? "inline" }),
      },
    );
  }

  listEvents(taskId: string, afterSequence?: number, limit = 200) {
    const query = new URLSearchParams({ limit: String(Math.min(200, Math.max(1, limit))) });
    if (afterSequence != null) query.set("after_sequence", String(afterSequence));
    return this.request<RunEvent[]>(
      `/api/tasks/${encodeURIComponent(taskId)}/events?${query.toString()}`,
    );
  }

  async latestEventSequence(taskId: string) {
    const events = await this.listEvents(taskId, undefined, 1);
    return events[events.length - 1]?.sequence ?? 0;
  }

  async listAllEvents(taskId: string, afterSequence = 0) {
    const events: RunEvent[] = [];
    let cursor = afterSequence;
    let hasAnotherPage = true;
    while (hasAnotherPage) {
      const page = await this.listEvents(taskId, cursor, 200);
      events.push(...page);
      hasAnotherPage = page.length === 200;
      if (!hasAnotherPage) break;
      const next = page[page.length - 1]?.sequence;
      if (typeof next !== "number" || next <= cursor) {
        throw new Error("Task event replay did not advance its sequence cursor");
      }
      cursor = next;
    }
    return events;
  }

  listApprovals() {
    return this.request<Approval[]>("/api/approvals");
  }

  decideApproval(id: string, decision: "accept" | "decline", version: number) {
    return this.request<void>(`/api/approvals/${encodeURIComponent(id)}/decision`, {
      method: "POST",
      body: JSON.stringify({ decision, version }),
    });
  }

  respondUserInput(
    id: string,
    answers: Record<string, { answers: string[] }>,
    version: number,
  ) {
    return this.request<void>(`/api/approvals/${encodeURIComponent(id)}/user-input`, {
      method: "POST",
      body: JSON.stringify({ answers, version }),
    });
  }

  workspaceStatus(runId: string) {
    return this.request<WorkspaceStatus>(
      `/api/runs/${encodeURIComponent(runId)}/workspace/status`,
    );
  }

  listWorkspaceGitRoots(runId: string, depth: number) {
    const query = new URLSearchParams({ depth: String(depth) });
    return this.request<string[]>(
      `/api/runs/${encodeURIComponent(runId)}/workspace/git-roots?${query.toString()}`,
    );
  }

  setWorkspaceGitRoot(runId: string, gitRoot: string | null) {
    return this.request<{ status: string }>(
      `/api/runs/${encodeURIComponent(runId)}/workspace/git-roots`,
      { method: "PUT", body: JSON.stringify({ gitRoot }) },
    );
  }

  listWorkspaceFiles(runId: string) {
    return this.request<string[]>(`/api/runs/${encodeURIComponent(runId)}/workspace/files`);
  }

  readWorkspaceFile(runId: string, path: string) {
    const query = new URLSearchParams({ path });
    return this.request<WorkspaceFileContent>(
      `/api/runs/${encodeURIComponent(runId)}/workspace/files/content?${query.toString()}`,
    );
  }

  writeWorkspaceAgents(runId: string, content: string) {
    return this.request<{ status: string }>(
      `/api/runs/${encodeURIComponent(runId)}/workspace/agents`,
      { method: "PUT", body: JSON.stringify({ content }) },
    );
  }

  workspaceDiffs(runId: string) {
    return this.request<WorkspaceFileDiff[]>(
      `/api/runs/${encodeURIComponent(runId)}/workspace/diffs`,
    );
  }

  stageWorkspacePaths(runId: string, paths: string[]) {
    return this.request<{ status: string }>(
      `/api/runs/${encodeURIComponent(runId)}/workspace/stage`,
      { method: "POST", body: JSON.stringify({ paths }) },
    );
  }

  stageAllWorkspacePaths(runId: string) {
    return this.request<{ status: string }>(
      `/api/runs/${encodeURIComponent(runId)}/workspace/stage-all`,
      { method: "POST" },
    );
  }

  unstageWorkspacePaths(runId: string, paths: string[]) {
    return this.request<{ status: string }>(
      `/api/runs/${encodeURIComponent(runId)}/workspace/unstage`,
      { method: "POST", body: JSON.stringify({ paths }) },
    );
  }

  revertWorkspacePaths(runId: string, paths: string[]) {
    return this.request<{ status: string }>(
      `/api/runs/${encodeURIComponent(runId)}/workspace/revert`,
      { method: "POST", body: JSON.stringify({ paths }) },
    );
  }

  revertAllWorkspacePaths(runId: string) {
    return this.request<{ status: string }>(
      `/api/runs/${encodeURIComponent(runId)}/workspace/revert-all`,
      { method: "POST" },
    );
  }

  listWorkspaceBranches(runId: string) {
    return this.request<WorkspaceBranch[]>(
      `/api/runs/${encodeURIComponent(runId)}/workspace/branches`,
    );
  }

  checkoutWorkspaceBranch(runId: string, name: string) {
    return this.request<{ status: string }>(
      `/api/runs/${encodeURIComponent(runId)}/workspace/branches/checkout`,
      { method: "POST", body: JSON.stringify({ name }) },
    );
  }

  createWorkspaceBranch(runId: string, name: string) {
    return this.request<{ status: string }>(
      `/api/runs/${encodeURIComponent(runId)}/workspace/branches`,
      { method: "POST", body: JSON.stringify({ name }) },
    );
  }

  renameWorkspaceBranch(runId: string, name: string) {
    return this.request<{ status: string; name: string }>(
      `/api/runs/${encodeURIComponent(runId)}/workspace/branch/rename`,
      { method: "POST", body: JSON.stringify({ name }) },
    );
  }

  removeDerivedWorkspace(runId: string) {
    return this.request<{ status: string }>(
      `/api/runs/${encodeURIComponent(runId)}/workspace`,
      { method: "DELETE" },
    );
  }

  applyDerivedWorkspace(runId: string) {
    return this.request<{ status: string }>(
      `/api/runs/${encodeURIComponent(runId)}/workspace/apply`,
      { method: "POST" },
    );
  }

  renameWorkspaceUpstream(runId: string, oldBranch: string, newBranch: string) {
    return this.request<{ status: string }>(
      `/api/runs/${encodeURIComponent(runId)}/workspace/upstream/rename`,
      {
        method: "POST",
        body: JSON.stringify({ old_branch: oldBranch, new_branch: newBranch }),
      },
    );
  }

  openTerminal(runId: string, terminalId: string, cols: number, rows: number) {
    return this.request<{ id: string }>(
      `/api/runs/${encodeURIComponent(runId)}/terminals`,
      {
        method: "POST",
        body: JSON.stringify({ terminal_id: terminalId, cols, rows }),
      },
    );
  }

  writeTerminal(runId: string, terminalId: string, data: string) {
    return this.request<{ status: string }>(
      `/api/runs/${encodeURIComponent(runId)}/terminals/${encodeURIComponent(terminalId)}/write`,
      { method: "POST", body: JSON.stringify({ data }) },
    );
  }

  resizeTerminal(runId: string, terminalId: string, cols: number, rows: number) {
    return this.request<{ status: string }>(
      `/api/runs/${encodeURIComponent(runId)}/terminals/${encodeURIComponent(terminalId)}/resize`,
      { method: "POST", body: JSON.stringify({ cols, rows }) },
    );
  }

  closeTerminal(runId: string, terminalId: string) {
    return this.request<{ status: string }>(
      `/api/runs/${encodeURIComponent(runId)}/terminals/${encodeURIComponent(terminalId)}`,
      { method: "DELETE" },
    );
  }

  workspaceLog(runId: string, limit = 40) {
    const query = new URLSearchParams({ limit: String(limit) });
    return this.request<WorkspaceLog>(
      `/api/runs/${encodeURIComponent(runId)}/workspace/log?${query.toString()}`,
    );
  }

  workspaceCommitDiffs(runId: string, sha: string) {
    return this.request<WorkspaceCommitDiff[]>(
      `/api/runs/${encodeURIComponent(runId)}/workspace/commits/${encodeURIComponent(sha)}/diff`,
    );
  }

  githubIssues(runId: string) {
    return this.request<GitHubIssues>(`/api/runs/${encodeURIComponent(runId)}/github/issues`);
  }

  createGithubRepository(
    runId: string,
    repo: string,
    visibility: "private" | "public",
    branch?: string | null,
  ) {
    return this.request<CreateGitHubRepositoryResponse>(
      `/api/runs/${encodeURIComponent(runId)}/github/repository`,
      {
        method: "POST",
        body: JSON.stringify({ repo, visibility, branch: branch ?? null }),
      },
    );
  }

  githubPullRequests(runId: string) {
    return this.request<GitHubPullRequests>(
      `/api/runs/${encodeURIComponent(runId)}/github/pull-requests`,
    );
  }

  githubPullRequestDiff(runId: string, number: number) {
    return this.request<GitHubPullRequestDiff[]>(
      `/api/runs/${encodeURIComponent(runId)}/github/pull-requests/${number}/diff`,
    );
  }

  githubPullRequestComments(runId: string, number: number) {
    return this.request<GitHubPullRequestComment[]>(
      `/api/runs/${encodeURIComponent(runId)}/github/pull-requests/${number}/comments`,
    );
  }

  checkoutGithubPullRequest(runId: string, number: number) {
    return this.request<{ status: string }>(
      `/api/runs/${encodeURIComponent(runId)}/github/pull-requests/${number}/checkout`,
      { method: "POST" },
    );
  }

  workspaceRemote(runId: string) {
    return this.request<string | null>(
      `/api/runs/${encodeURIComponent(runId)}/workspace/remote`,
    );
  }

  workspaceRemoteOperation(runId: string, operation: "fetch" | "pull" | "push" | "sync") {
    return this.request<{ status: string }>(
      `/api/runs/${encodeURIComponent(runId)}/workspace/${operation}`,
      { method: "POST" },
    );
  }

  profileAccount() {
    return this.request<{ data: Record<string, unknown> }>("/api/profile/account")
      .then((response) => response.data);
  }

  startProfileLogin() {
    return this.request<ProfileLoginStart>("/api/profile/login", { method: "POST" });
  }

  cancelProfileLogin() {
    return this.request<ProfileLoginCancel>("/api/profile/login", { method: "DELETE" });
  }

  profileLoginStatus(loginId: string) {
    return this.request<ProfileLoginStatus>(
      `/api/profile/login/${encodeURIComponent(loginId)}`,
    );
  }

  profileRateLimits() {
    return this.request<{ data: Record<string, unknown> }>("/api/profile/rate-limits")
      .then((response) => response.data);
  }

  profileUsage(days: number, workspaceId?: string | null) {
    const query = new URLSearchParams({ days: String(days) });
    if (workspaceId) query.set("workspaceId", workspaceId);
    return this.request<{
      updatedAt: number;
      days: Array<{ day: string; inputTokens: number; cachedInputTokens: number; outputTokens: number; totalTokens: number; agentTimeMs: number; agentRuns: number }>;
      totals: { last7DaysTokens: number; last30DaysTokens: number; averageDailyTokens: number; cacheHitRatePercent: number; peakDay: string | null; peakDayTokens: number };
      topModels: Array<{ model: string; tokens: number; sharePercent: number }>;
    }>(`/api/profile/usage?${query.toString()}`);
  }

  profileCollaborationModes() {
    return this.request<{ data: Record<string, unknown> }>("/api/profile/collaboration-modes")
      .then((response) => response.data);
  }

  profileSkills(runId: string, forceReload = false) {
    const query = new URLSearchParams({ runId, forceReload: String(forceReload) });
    return this.request<{ data: Record<string, unknown> }>(
      `/api/profile/skills?${query.toString()}`,
    ).then((response) => response.data);
  }

  profileApps(runId: string, cursor?: string | null, limit?: number | null) {
    return this.profileList("apps", runId, cursor, limit);
  }

  profileRuntimeStatus() {
    return this.request<{ data: Record<string, unknown> }>("/api/profile/runtime-status")
      .then((response) => response.data);
  }

  profileMcpServers(runId: string, cursor?: string | null, limit?: number | null) {
    return this.profileList("mcp-servers", runId, cursor, limit);
  }

  profileExperimentalFeatures(runId: string, cursor?: string | null, limit?: number | null) {
    return this.profileList("experimental-features", runId, cursor, limit);
  }

  setExperimentalFeature(name: string, enabled: boolean) {
    return this.request<{ status: string }>(
      `/api/profile/features/${encodeURIComponent(name)}`,
      { method: "PUT", body: JSON.stringify({ enabled }) },
    );
  }

  readProfileFile(kind: "agents" | "config") {
    return this.request<ProfileTextFile>(`/api/profile/files/${kind}`);
  }

  writeProfileFile(kind: "agents" | "config", content: string) {
    return this.request<{ status: string }>(`/api/profile/files/${kind}`, {
      method: "PUT",
      body: JSON.stringify({ content }),
    });
  }

  getAgents() {
    return this.request<AgentsSettings>("/api/profile/agents");
  }

  getConfigModel() {
    return this.request<{ model: string | null }>("/api/profile/config/model");
  }

  setAgentsCore(input: {
    multiAgentEnabled: boolean;
    maxThreads: number;
    maxDepth: number;
  }) {
    return this.request<AgentsSettings>("/api/profile/agents/settings", {
      method: "PUT",
      body: JSON.stringify(input),
    });
  }

  createAgent(input: Record<string, unknown>) {
    return this.request<AgentsSettings>("/api/profile/agents", {
      method: "POST",
      body: JSON.stringify(input),
    });
  }

  updateAgent(originalName: string, input: Record<string, unknown>) {
    return this.request<AgentsSettings>(`/api/profile/agents/${encodeURIComponent(originalName)}`, {
      method: "PATCH",
      body: JSON.stringify(input),
    });
  }

  deleteAgent(name: string, deleteManagedFile: boolean) {
    const query = new URLSearchParams({ deleteManagedFile: String(deleteManagedFile) });
    return this.request<AgentsSettings>(
      `/api/profile/agents/${encodeURIComponent(name)}?${query.toString()}`,
      { method: "DELETE" },
    );
  }

  readAgentConfig(name: string) {
    return this.request<string>(`/api/profile/agents/${encodeURIComponent(name)}/config`);
  }

  writeAgentConfig(name: string, content: string) {
    return this.request<{ status: string }>(
      `/api/profile/agents/${encodeURIComponent(name)}/config`,
      { method: "PUT", body: JSON.stringify({ content }) },
    );
  }

  listPrompts(runId: string) {
    const query = new URLSearchParams({ runId });
    return this.request<PromptEntry[]>(`/api/profile/prompts?${query.toString()}`);
  }

  createPrompt(input: Record<string, unknown>) {
    return this.request<PromptEntry>("/api/profile/prompts", {
      method: "POST",
      body: JSON.stringify(input),
    });
  }

  updatePrompt(input: Record<string, unknown>) {
    return this.request<PromptEntry>("/api/profile/prompts", {
      method: "PUT",
      body: JSON.stringify(input),
    });
  }

  deletePrompt(input: Record<string, unknown>) {
    return this.request<{ status: string }>("/api/profile/prompts", {
      method: "DELETE",
      body: JSON.stringify(input),
    });
  }

  movePrompt(input: Record<string, unknown>) {
    return this.request<PromptEntry>("/api/profile/prompts/move", {
      method: "POST",
      body: JSON.stringify(input),
    });
  }

  generateText(runId: string, kind: "runMetadata" | "agentDescription" | "commitMessage", input: string, model?: string | null) {
    return this.request<{ text: string }>(`/api/runs/${encodeURIComponent(runId)}/generate`, {
      method: "POST",
      body: JSON.stringify({ kind, input, model: model ?? null }),
    });
  }

  rememberApprovalRule(runId: string, command: string[]) {
    return this.request<{ ok: boolean }>("/api/profile/approval-rules", {
      method: "POST",
      body: JSON.stringify({ runId, command }),
    });
  }

  private profileList(
    resource: "apps" | "mcp-servers" | "experimental-features",
    runId: string,
    cursor?: string | null,
    limit?: number | null,
  ) {
    const query = new URLSearchParams({ runId });
    if (cursor) query.set("cursor", cursor);
    if (limit != null) query.set("limit", String(limit));
    return this.request<{ data: Record<string, unknown> }>(
      `/api/profile/${resource}?${query.toString()}`,
    ).then((response) => response.data);
  }

  commitWorkspace(runId: string, selectedPaths: string[], message: string) {
    return this.request<{ workspace_id: string; commit: string }>(
      `/api/runs/${encodeURIComponent(runId)}/workspace/commit`,
      {
        method: "POST",
        body: JSON.stringify({ selected_paths: selectedPaths, message }),
      },
    );
  }

  listProviders() {
    return this.request<ProviderCatalog>("/api/providers");
  }

  selectProvider(id: string) {
    return this.request<ProviderCatalog>(`/api/providers/${encodeURIComponent(id)}/select`, {
      method: "POST",
    });
  }

  selectProviderModel(providerId: string, modelId: string) {
    return this.request<ProviderCatalog>(
      `/api/providers/${encodeURIComponent(providerId)}/models/${encodeURIComponent(modelId)}/select`,
      { method: "POST" },
    );
  }

  upsertProvider(id: string, input: Record<string, unknown>) {
    return this.request<ProviderCatalog>(`/api/providers/${encodeURIComponent(id)}`, {
      method: "PUT",
      body: JSON.stringify(input),
    });
  }

  deleteProvider(id: string) {
    return this.request<ProviderCatalog>(`/api/providers/${encodeURIComponent(id)}`, {
      method: "DELETE",
    });
  }

  refreshProviderModels(id: string) {
    return this.request<ProviderCatalog>(
      `/api/providers/${encodeURIComponent(id)}/models/refresh`,
      { method: "POST" },
    );
  }

  updateProviderModel(providerId: string, modelId: string, contextWindow: number) {
    return this.request<ProviderCatalog>(
      `/api/providers/${encodeURIComponent(providerId)}/models/${encodeURIComponent(modelId)}`,
      { method: "PATCH", body: JSON.stringify({ contextWindow }) },
    );
  }

  subscribe(
    onEvent: (event: RunEvent) => void,
    onState: (state: "connecting" | "online" | "offline" | "resync") => void,
  ) {
    let disposed = false;
    let socket: WebSocket | null = null;
    let retry: number | null = null;
    let attempts = 0;
    const connect = () => {
      if (disposed || !this.token) return;
      onState("connecting");
      const endpoint = new URL(`${this.baseUrl || window.location.origin}/api/events/ws`);
      endpoint.protocol = endpoint.protocol === "https:" ? "wss:" : "ws:";
      socket = new WebSocket(endpoint);
      socket.onopen = () => {
        socket?.send(JSON.stringify({ type: "authenticate", token: this.token }));
      };
      socket.onmessage = (message) => {
        const envelope = JSON.parse(String(message.data)) as LiveEnvelope;
        if (envelope.type === "ready") {
          attempts = 0;
          onState("online");
        } else if (envelope.type === "run.event") {
          onEvent(envelope.event);
        } else if (envelope.type === "resyncRequired") {
          onState("resync");
        }
      };
      socket.onclose = () => {
        if (disposed) return;
        onState("offline");
        const delay = Math.min(10_000, 500 * 2 ** attempts++);
        retry = window.setTimeout(connect, delay);
      };
      socket.onerror = () => socket?.close();
    };
    connect();
    return () => {
      disposed = true;
      if (retry != null) window.clearTimeout(retry);
      socket?.close();
    };
  }
}
