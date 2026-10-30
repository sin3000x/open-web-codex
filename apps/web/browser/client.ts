import type {
  Approval,
  ProviderCallMetric,
  Me,
  Project,
  ProviderCatalog,
  Run,
  RunEvent,
  RuntimeAgentActivity,
  RuntimeAgentExecution,
  McpFormContent,
  McpFormResponseAction,
  PendingMcpFormSummary,
  PendingUserInputSummary,
  Session,
  Task,
  Workspace,
  WorkspaceStatus,
  WorkspaceFileContent,
  WorkspaceFileUploadResponse,
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
  ThreadTokenUsageResponse,
  BrowserWorkspacePreference,
  CreateGitHubRepositoryResponse,
  MapsConfiguration,
  MapsProvider,
  RuntimeAgentProjection,
  ArtifactSummary,
  ArtifactContent,
  ExplicitResourceSelection,
  ResourceReferenceSummary,
  CopilotProfileStatus,
} from "./types";

type ClientOptions = {
  baseUrl?: string;
  token?: string;
};

export type ProviderCatalogFailure =
  | "authentication"
  | "not_found"
  | "rate_limited"
  | "upstream"
  | "timeout"
  | "network"
  | "invalid_json"
  | "incompatible_schema"
  | "empty_catalog";

export type PlatformRequestDetail = {
  providerCatalogFailure: ProviderCatalogFailure | null;
};

export class PlatformRequestError extends Error {
  readonly detail: PlatformRequestDetail;
  readonly code?: string;
  readonly status?: number;
  readonly kind?: string;

  constructor(message: string, detail: PlatformRequestDetail, metadata?: {
    code?: string;
    status?: number;
    kind?: string;
  }) {
    super(message);
    this.name = "PlatformRequestError";
    this.detail = detail;
    this.code = metadata?.code;
    this.status = metadata?.status;
    this.kind = metadata?.kind;
  }
}

const PROVIDER_CATALOG_FAILURES = new Set<ProviderCatalogFailure>([
  "authentication",
  "not_found",
  "rate_limited",
  "upstream",
  "timeout",
  "network",
  "invalid_json",
  "incompatible_schema",
  "empty_catalog",
]);

function providerCatalogFailure(value: unknown): ProviderCatalogFailure | null {
  return typeof value === "string" && PROVIDER_CATALOG_FAILURES.has(value as ProviderCatalogFailure)
    ? (value as ProviderCatalogFailure)
    : null;
}

export function isPlatformRequestError(error: unknown): error is PlatformRequestError {
  if (error instanceof PlatformRequestError) return true;
  if (!error || typeof error !== "object") return false;
  const detail = (error as { detail?: unknown }).detail;
  if (!detail || typeof detail !== "object") return false;
  const candidate = detail as Partial<PlatformRequestDetail>;
  return candidate.providerCatalogFailure === null
    || providerCatalogFailure(candidate.providerCatalogFailure) !== null;
}

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
    const isFormData =
      typeof FormData !== "undefined" && init?.body instanceof FormData;
    const response = await fetch(`${this.baseUrl}${path}`, {
      ...init,
      cache: "no-store",
      headers: {
        ...(init?.body && !isFormData ? { "content-type": "application/json" } : {}),
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
      const error = new PlatformRequestError(message, {
        providerCatalogFailure: providerCatalogFailure(record?.provider_catalog_failure),
      }, {
        code: message,
        status: response.status,
        kind: typeof record?.kind === "string" ? record.kind : undefined,
      });
      throw error;
    }
    return payload as T;
  }

  private async requestArtifactContent(path: string): Promise<ArtifactContent> {
    const response = await fetch(`${this.baseUrl}${path}`, {
      cache: "no-store",
      headers: {
        ...(this.token ? { authorization: `Bearer ${this.token}` } : {}),
      },
    });
    const text = await response.text();
    if (!response.ok) {
      let payload: Record<string, unknown> | null = null;
      try {
        const parsed = text ? JSON.parse(text) as unknown : null;
        payload = parsed && typeof parsed === "object" && !Array.isArray(parsed)
          ? parsed as Record<string, unknown>
          : null;
      } catch {
        payload = null;
      }
      const message = typeof payload?.message === "string"
        ? payload.message
        : `Request failed (HTTP ${response.status}).`;
      throw new PlatformRequestError(message, {
        providerCatalogFailure: null,
      }, {
        code: message,
        status: response.status,
        kind: typeof payload?.kind === "string" ? payload.kind : undefined,
      });
    }
    const mimeType = response.headers.get("content-type")
      ?.split(";", 1)[0]
      ?.trim()
      ?.toLowerCase();
    if (mimeType === "text/markdown") {
      return { kind: "markdown", mime_type: "text/markdown", text };
    }
    if (mimeType === "application/json" || mimeType === "application/geo+json") {
      let parsed: unknown;
      try {
        parsed = JSON.parse(text) as unknown;
      } catch {
        throw new Error("Artifact JSON content is invalid.");
      }
      if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
        throw new Error("Artifact JSON content is invalid.");
      }
      return {
        kind: "json",
        mime_type: mimeType,
        value: parsed as Record<string, unknown>,
      };
    }
    throw new Error("Artifact content type is unsupported.");
  }

  health() {
    return this.request<{ ok: boolean; schemaStatus: "current" | "not_current"; version: string }>("/api/health");
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
    approvalId?: string,
  ) {
    return this.request<MapsConfiguration>("/api/configuration/maps", {
      method: "PUT",
      body: JSON.stringify({ provider, apiKey, approvalId }),
    });
  }

  useMapsConfiguration(approvalId: string) {
    return this.request<MapsConfiguration>("/api/configuration/maps/use", {
      method: "POST",
      body: JSON.stringify({ approvalId }),
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

  listWorkspaces() {
    return this.request<Workspace[]>("/api/workspaces");
  }

  getWorkspace(workspaceId: string) {
    return this.request<Workspace>(`/api/workspaces/${encodeURIComponent(workspaceId)}`);
  }

  createWorkspace(input: {
    projectId: string;
    kind: Workspace["kind"];
    name?: string | null;
    sourceRef?: string | null;
    parentWorkspaceId?: string | null;
    copyAgentsMd?: boolean;
  }) {
    return this.request<Workspace>("/api/workspaces", {
      method: "POST",
      body: JSON.stringify({
        project_id: input.projectId,
        idempotency_key: createIdempotencyKey(),
        kind: input.kind,
        name: input.name?.trim() || null,
        source_ref: input.sourceRef?.trim() || null,
        parent_workspace_id: input.parentWorkspaceId ?? null,
        copy_agents_md: input.copyAgentsMd ?? false,
      }),
    });
  }

  removeWorkspace(workspaceId: string) {
    return this.request<{ status: string }>(
      `/api/workspaces/${encodeURIComponent(workspaceId)}`,
      { method: "DELETE" },
    );
  }

  listTasks(projectId: string) {
    return this.request<Task[]>(`/api/tasks?project_id=${encodeURIComponent(projectId)}`);
  }

  createTask(
    projectId: string,
    workspaceId: string,
    title: string,
    selection?: { providerId: string; modelId: string } | null,
    copilot?: { packageId: string } | null,
  ) {
    return this.request<Task>("/api/tasks", {
      method: "POST",
      body: JSON.stringify({
        project_id: projectId,
        workspace_id: workspaceId,
        title,
        model_provider: selection?.providerId ?? null,
        model: selection?.modelId ?? null,
        copilot_package_id: copilot?.packageId ?? null,
      }),
    });
  }

  copilotProfileStatus() {
    return this.request<CopilotProfileStatus>("/api/profile/copilots");
  }

  activateCopilot(packageId: string) {
    return this.request<CopilotProfileStatus>("/api/profile/copilots/activate", {
      method: "POST",
      body: JSON.stringify({ packageId }),
    });
  }

  deactivateCopilot(packageId: string) {
    return this.request<CopilotProfileStatus>("/api/profile/copilots/deactivate", {
      method: "POST",
      body: JSON.stringify({ packageId }),
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

  async readInlineVisualization(threadId: string, file: string) {
    if (!/^[0-9a-f-]{36}$/i.test(threadId)) {
      throw new Error("Inline visualization Thread is invalid.");
    }
    if (!/^[A-Za-z0-9_.-]{1,123}\.(?:html|png|jpe?g|gif|webp)$/.test(file)) {
      throw new Error("Inline visualization file is invalid.");
    }
    const response = await fetch(
      `${this.baseUrl}/api/threads/${encodeURIComponent(threadId)}/inline-visualizations/${encodeURIComponent(file)}`,
      {
        cache: "no-store",
        headers: this.token ? { authorization: `Bearer ${this.token}` } : undefined,
      },
    );
    if (!response.ok) {
      throw new Error(`Inline visualization is unavailable (HTTP ${response.status}).`);
    }
    const contentType = response.headers.get("content-type")
      ?.split(";", 1)[0]
      ?.trim()
      ?.toLowerCase() ?? "";
    if (![
      "text/html",
      "image/png",
      "image/jpeg",
      "image/gif",
      "image/webp",
    ].includes(contentType)) {
      throw new Error("Inline visualization content type is unsupported.");
    }
    return { blob: await response.blob(), contentType };
  }

  readRunThread(runId: string) {
    return this.request<ThreadHistoryResponse>(`/api/runs/${encodeURIComponent(runId)}/thread`);
  }

  listRunThreadTurns(runId: string) {
    return this.request<ThreadHistoryTurn[]>(
      `/api/runs/${encodeURIComponent(runId)}/thread/turns`,
    );
  }

  readRunThreadTokenUsage(runId: string) {
    return this.request<ThreadTokenUsageResponse>(
      `/api/runs/${encodeURIComponent(runId)}/thread/token-usage`,
    );
  }

  listRunAgentThreadTurns(runId: string, threadId: string) {
    return this.request<ThreadHistoryTurn[]>(
      `/api/runs/${encodeURIComponent(runId)}/agents/${encodeURIComponent(threadId)}/turns`,
    );
  }

  readReplyArtifact(path: string) {
    const finalArtifact = /^\/api\/artifacts\/[0-9a-f-]{36}\/content$/i.test(path);
    const inlineMapSource = /^\/api\/runs\/[0-9a-f-]{36}\/inline-maps\/[A-Za-z0-9_.-]{1,128}\/sources\/[A-Za-z0-9_.-]{1,128}$/i.test(path);
    if (!finalArtifact && !inlineMapSource) {
      return Promise.reject(new Error("Reply Artifact path is invalid."));
    }
    return this.request<Record<string, unknown>>(path);
  }

  readArtifactContent(artifactId: string) {
    return this.requestArtifactContent(
      `/api/artifacts/${encodeURIComponent(artifactId)}/content`,
    );
  }

  async downloadArtifact(artifactId: string) {
    const response = await fetch(
      `${this.baseUrl}/api/artifacts/${encodeURIComponent(artifactId)}/download`,
      {
        cache: "no-store",
        headers: this.token ? { authorization: `Bearer ${this.token}` } : undefined,
      },
    );
    if (!response.ok) {
      const text = await response.text();
      let message = `Request failed (HTTP ${response.status}).`;
      try {
        const payload = JSON.parse(text) as { message?: unknown };
        if (typeof payload.message === "string") message = payload.message;
      } catch {
        // Keep the typed fallback for non-JSON platform errors.
      }
      throw new Error(message);
    }
    const contentDisposition = response.headers.get("content-disposition") ?? "";
    const encodedFilename = contentDisposition.match(/filename\*=UTF-8''([^;]+)/i)?.[1];
    const quotedFilename = contentDisposition.match(/filename="([^"]+)"/i)?.[1];
    let filename = quotedFilename || `artifact-${artifactId}`;
    if (encodedFilename) {
      try {
        filename = decodeURIComponent(encodedFilename);
      } catch {
        // Use the ASCII fallback when decoding fails.
      }
    }
    return { blob: await response.blob(), filename };
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
    options: {
      idempotencyKey?: string;
      forkThreadId?: string | null;
      forkSourceRunId?: string | null;
    },
  ) {
    return this.request<{ run: Run }>(`/api/tasks/${encodeURIComponent(taskId)}/runs`, {
      method: "POST",
      body: JSON.stringify({
        idempotency_key: options.idempotencyKey ?? createIdempotencyKey(),
        fork_thread_id: options.forkThreadId ?? null,
        fork_source_run_id: options.forkSourceRunId ?? null,
      }),
    });
  }
  listRunAgents(runId: string) {
    return this.request<RuntimeAgentProjection[]>(
      `/api/runs/${encodeURIComponent(runId)}/agents`,
    );
  }

  listRunAgentActivities(runId: string) {
    return this.request<RuntimeAgentActivity[]>(
      `/api/runs/${encodeURIComponent(runId)}/agent-activities`,
    );
  }

  listRunAgentExecutions(runId: string) {
    return this.request<RuntimeAgentExecution[]>(
      `/api/runs/${encodeURIComponent(runId)}/agent-executions`,
    );
  }

  listRunProviderMetrics(runId: string) {
    return this.request<ProviderCallMetric[]>(
      `/api/runs/${encodeURIComponent(runId)}/provider-metrics`,
    );
  }

  listRunUserInputRequests(runId: string) {
    return this.request<PendingUserInputSummary[]>(
      `/api/runs/${encodeURIComponent(runId)}/user-input-requests`,
    );
  }

  listRunMcpFormRequests(runId: string) {
    return this.request<PendingMcpFormSummary[]>(
      `/api/runs/${encodeURIComponent(runId)}/mcp-form-requests`,
    );
  }

  listTaskArtifacts(taskId: string) {
    return this.request<ArtifactSummary[]>(
      `/api/tasks/${encodeURIComponent(taskId)}/artifacts`,
    );
  }

  listTaskResourceRefs(taskId: string) {
    return this.request<ResourceReferenceSummary[]>(
      `/api/tasks/${encodeURIComponent(taskId)}/resource-refs`,
    );
  }

  getArtifact(artifactId: string) {
    return this.request<ArtifactSummary>(
      `/api/artifacts/${encodeURIComponent(artifactId)}`,
    );
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
      mapCardRef?: string | null;
      selectedResources?: ExplicitResourceSelection[];
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
          mapCardRef: options.mapCardRef ?? null,
          selectedResources: options.selectedResources ?? [],
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

  respondMcpForm(
    id: string,
    action: McpFormResponseAction,
    content: McpFormContent | undefined,
    version: number,
  ) {
    return this.request<void>(`/api/approvals/${encodeURIComponent(id)}/mcp-form`, {
      method: "POST",
      body: JSON.stringify({ action, ...(content === undefined ? {} : { content }), version }),
    });
  }

  workspaceStatus(workspaceId: string) {
    return this.request<WorkspaceStatus>(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/status`,
    );
  }

  listWorkspaceGitRoots(workspaceId: string, depth: number) {
    const query = new URLSearchParams({ depth: String(depth) });
    return this.request<string[]>(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/git-roots?${query.toString()}`,
    );
  }

  setWorkspaceGitRoot(workspaceId: string, gitRoot: string | null) {
    return this.request<{ status: string }>(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/git-roots`,
      { method: "PUT", body: JSON.stringify({ gitRoot }) },
    );
  }

  listWorkspaceFiles(workspaceId: string) {
    return this.request<string[]>(`/api/workspaces/${encodeURIComponent(workspaceId)}/files`);
  }

  uploadWorkspaceFiles(
    workspaceId: string,
    files: File[],
    options: { overwrite?: boolean; paths?: string[] } = {},
  ) {
    if (files.length !== 1) {
      throw new Error("Upload exactly one Workspace file per request.");
    }
    if (options.paths && options.paths.length !== files.length) {
      throw new Error("Every uploaded file needs one Workspace-relative path.");
    }
    const body = new FormData();
    for (const [index, file] of files.entries()) {
      const relativePath = options.paths?.[index] || file.webkitRelativePath || file.name;
      body.append("files", file, relativePath);
    }
    const query = options.overwrite ? "?overwrite=true" : "";
    return this.request<WorkspaceFileUploadResponse>(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/files${query}`,
      { method: "POST", body },
    );
  }

  readWorkspaceFile(workspaceId: string, path: string) {
    const query = new URLSearchParams({ path });
    return this.request<WorkspaceFileContent>(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/files/content?${query.toString()}`,
    );
  }

  async downloadWorkspaceFile(workspaceId: string, path: string) {
    const query = new URLSearchParams({ path });
    const response = await fetch(
      `${this.baseUrl}/api/workspaces/${encodeURIComponent(workspaceId)}/files/download?${query.toString()}`,
      {
        cache: "no-store",
        headers: this.token ? { authorization: `Bearer ${this.token}` } : undefined,
      },
    );
    if (!response.ok) {
      const text = await response.text();
      let message = `Request failed (HTTP ${response.status}).`;
      try {
        const payload = JSON.parse(text) as { message?: unknown };
        if (typeof payload.message === "string") message = payload.message;
      } catch {
        // Keep the typed fallback for non-JSON platform errors.
      }
      throw new Error(message);
    }
    const contentDisposition = response.headers.get("content-disposition") ?? "";
    const encodedFilename = contentDisposition.match(/filename\*=UTF-8''([^;]+)/i)?.[1];
    const quotedFilename = contentDisposition.match(/filename="([^"]+)"/i)?.[1];
    let filename = quotedFilename || path.split("/").pop() || "download";
    if (encodedFilename) {
      try {
        filename = decodeURIComponent(encodedFilename);
      } catch {
        // Use the ASCII fallback or requested path name when decoding fails.
      }
    }
    return { blob: await response.blob(), filename };
  }

  deleteWorkspaceFile(workspaceId: string, path: string) {
    return this.request<{ status: string; path: string }>(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/files`,
      { method: "DELETE", body: JSON.stringify({ path }) },
    );
  }

  writeWorkspaceAgents(workspaceId: string, content: string) {
    return this.request<{ status: string }>(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/agents`,
      { method: "PUT", body: JSON.stringify({ content }) },
    );
  }

  workspaceDiffs(workspaceId: string) {
    return this.request<WorkspaceFileDiff[]>(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/diffs`,
    );
  }

  stageWorkspacePaths(workspaceId: string, paths: string[]) {
    return this.request<{ status: string }>(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/stage`,
      { method: "POST", body: JSON.stringify({ paths }) },
    );
  }

  stageAllWorkspacePaths(workspaceId: string) {
    return this.request<{ status: string }>(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/stage-all`,
      { method: "POST" },
    );
  }

  unstageWorkspacePaths(workspaceId: string, paths: string[]) {
    return this.request<{ status: string }>(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/unstage`,
      { method: "POST", body: JSON.stringify({ paths }) },
    );
  }

  revertWorkspacePaths(workspaceId: string, paths: string[]) {
    return this.request<{ status: string }>(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/revert`,
      { method: "POST", body: JSON.stringify({ paths }) },
    );
  }

  revertAllWorkspacePaths(workspaceId: string) {
    return this.request<{ status: string }>(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/revert-all`,
      { method: "POST" },
    );
  }

  listWorkspaceBranches(workspaceId: string) {
    return this.request<WorkspaceBranch[]>(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/branches`,
    );
  }

  checkoutWorkspaceBranch(workspaceId: string, name: string) {
    return this.request<{ status: string }>(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/branches/checkout`,
      { method: "POST", body: JSON.stringify({ name }) },
    );
  }

  createWorkspaceBranch(workspaceId: string, name: string) {
    return this.request<{ status: string }>(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/branches`,
      { method: "POST", body: JSON.stringify({ name }) },
    );
  }

  renameWorkspaceBranch(workspaceId: string, name: string) {
    return this.request<{ status: string; name: string }>(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/branch/rename`,
      { method: "POST", body: JSON.stringify({ name }) },
    );
  }

  applyWorkspaceChanges(workspaceId: string) {
    return this.request<{ status: string }>(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/apply`,
      { method: "POST" },
    );
  }

  renameWorkspaceUpstream(workspaceId: string, oldBranch: string, newBranch: string) {
    return this.request<{ status: string }>(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/upstream/rename`,
      {
        method: "POST",
        body: JSON.stringify({ old_branch: oldBranch, new_branch: newBranch }),
      },
    );
  }

  openTerminal(workspaceId: string, terminalId: string, cols: number, rows: number) {
    return this.request<{ id: string }>(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/terminals`,
      {
        method: "POST",
        body: JSON.stringify({ terminal_id: terminalId, cols, rows }),
      },
    );
  }

  writeTerminal(workspaceId: string, terminalId: string, data: string) {
    return this.request<{ status: string }>(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/terminals/${encodeURIComponent(terminalId)}/write`,
      { method: "POST", body: JSON.stringify({ data }) },
    );
  }

  resizeTerminal(workspaceId: string, terminalId: string, cols: number, rows: number) {
    return this.request<{ status: string }>(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/terminals/${encodeURIComponent(terminalId)}/resize`,
      { method: "POST", body: JSON.stringify({ cols, rows }) },
    );
  }

  closeTerminal(workspaceId: string, terminalId: string) {
    return this.request<{ status: string }>(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/terminals/${encodeURIComponent(terminalId)}`,
      { method: "DELETE" },
    );
  }

  workspaceLog(workspaceId: string, limit = 40) {
    const query = new URLSearchParams({ limit: String(limit) });
    return this.request<WorkspaceLog>(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/log?${query.toString()}`,
    );
  }

  workspaceCommitDiffs(workspaceId: string, sha: string) {
    return this.request<WorkspaceCommitDiff[]>(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/commits/${encodeURIComponent(sha)}/diff`,
    );
  }

  githubIssues(workspaceId: string) {
    return this.request<GitHubIssues>(`/api/workspaces/${encodeURIComponent(workspaceId)}/github/issues`);
  }

  createGithubRepository(
    workspaceId: string,
    repo: string,
    visibility: "private" | "public",
    branch?: string | null,
  ) {
    return this.request<CreateGitHubRepositoryResponse>(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/github/repository`,
      {
        method: "POST",
        body: JSON.stringify({ repo, visibility, branch: branch ?? null }),
      },
    );
  }

  githubPullRequests(workspaceId: string) {
    return this.request<GitHubPullRequests>(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/github/pull-requests`,
    );
  }

  githubPullRequestDiff(workspaceId: string, number: number) {
    return this.request<GitHubPullRequestDiff[]>(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/github/pull-requests/${number}/diff`,
    );
  }

  githubPullRequestComments(workspaceId: string, number: number) {
    return this.request<GitHubPullRequestComment[]>(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/github/pull-requests/${number}/comments`,
    );
  }

  checkoutGithubPullRequest(workspaceId: string, number: number) {
    return this.request<{ status: string }>(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/github/pull-requests/${number}/checkout`,
      { method: "POST" },
    );
  }

  workspaceRemote(workspaceId: string) {
    return this.request<string | null>(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/remote`,
    );
  }

  workspaceRemoteOperation(workspaceId: string, operation: "fetch" | "pull" | "push" | "sync") {
    return this.request<{ status: string }>(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/${operation}`,
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

  commitWorkspace(workspaceId: string, selectedPaths: string[], message: string) {
    return this.request<{ workspace_id: string; commit: string }>(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/commit`,
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

  updateProviderModel(
    providerId: string,
    modelId: string,
    contextWindow: number,
    supportsSearchTool?: boolean,
  ) {
    return this.request<ProviderCatalog>(
      `/api/providers/${encodeURIComponent(providerId)}/models/${encodeURIComponent(modelId)}`,
      {
        method: "PATCH",
        body: JSON.stringify({
          contextWindow,
          ...(supportsSearchTool === undefined ? {} : { supportsSearchTool }),
        }),
      },
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
