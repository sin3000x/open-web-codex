import { PlatformClient } from "../../browser/client";
import type {
  Approval,
  ArtifactContent,
  ArtifactSummary,
  McpFormContent,
  McpFormResponseAction,
  PendingMcpFormSummary,
  Run,
  RunEvent,
  RuntimeAgentActivity,
  RuntimeAgentExecution,
  RuntimeAgentProjection,
  PendingUserInputSummary,
  Task,
  ThreadHistoryTurn,
  Workspace,
} from "../../browser/types";
import type { AppServerEvent, GitFileStatus, WorkspaceInfo } from "../types";

type WebClientOptions = {
  baseUrl?: string;
  token?: string;
};

export type GatewayHealth = {
  ok: boolean;
  name: string;
  version: string;
};

export type SupervisorOverviewData = {
  taskTitle: string;
  agents: RuntimeAgentProjection[];
  activities: RuntimeAgentActivity[];
  executions: RuntimeAgentExecution[];
  artifacts: ArtifactSummary[];
};

type EventSubscriptionStatus = {
  onOpen?: () => void;
  onError?: () => void;
};

type ThreadContext = {
  workspaceId: string;
  projectId: string;
  taskId: string;
  runId: string;
  rootThreadId: string;
};

type JsonRecord = Record<string, unknown>;

type ThreadStartDraft = {
  taskId: string | null;
  taskPromise: Promise<Task> | null;
  runIdempotencyKey: string;
  acceptedRunId: string | null;
};

export type AcceptedThreadStart = {
  taskId: string;
  runId: string;
};

export type CopilotTaskSelection = {
  packageId: string;
};

function isRecord(value: unknown): value is JsonRecord {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function sameOriginBaseUrl() {
  return typeof window === "undefined" ? "" : window.location.origin;
}

function newIdempotencyKey() {
  const randomUUID = globalThis.crypto?.randomUUID;
  return typeof randomUUID === "function"
    ? randomUUID.call(globalThis.crypto)
    : `thread-start-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

function threadStartDraftKey(
  workspaceId: string,
  operationId: string,
) {
  return `${workspaceId}:${operationId}`;
}

function platformWorkspace(workspace: Workspace): WorkspaceInfo {
  return {
    id: workspace.id,
    name: workspace.name,
    // WorkspaceInfo is shared with the original desktop UI, where `path` is a
    // local directory. The browser must receive only safe display metadata;
    // the server-side root remains authoritative and private.
    path: workspace.name,
    connected: workspace.state === "ready" || workspace.state === "retained",
    kind: workspace.kind === "worktree" ? "worktree" : "main",
    parentId: workspace.parent_workspace_id,
    worktree: workspace.kind === "worktree"
      ? { branch: workspace.branch_name ?? workspace.source_ref }
      : null,
    settings: {
      sidebarCollapsed: false,
      cloneSourceWorkspaceId: workspace.kind === "clone"
        ? workspace.parent_workspace_id
        : null,
    },
  };
}

function threadDisplayName(value: unknown): string {
  const name = typeof value === "string" ? value.trim() : "";
  return !name || name === "New Agent" ? "Thread" : name;
}

function rawItem(event: RunEvent): JsonRecord | null {
  if (!event.item_id || !event.payload.itemType) return null;
  const data = isRecord(event.payload.data) ? event.payload.data : {};
  return { id: event.item_id, type: event.payload.itemType, ...data };
}

function runtimeMessage(event: RunEvent): JsonRecord | null {
  const data = isRecord(event.payload.data) ? event.payload.data : {};
  const base = {
    threadId: event.thread_id,
    ...(event.turn_id ? { turnId: event.turn_id } : {}),
    ...(event.item_id ? { itemId: event.item_id } : {}),
  };
  if (event.event_type === "platform.approval.requested") {
    const requestMethod = typeof data.requestMethod === "string" ? data.requestMethod : null;
    const requestParams = isRecord(data.requestParams) ? data.requestParams : null;
    const approvalId = typeof data.approvalId === "string" ? data.approvalId : null;
    if (!requestMethod || !requestParams || !approvalId) return null;
    if (requestMethod === "item/tool/requestUserInput") {
      return {
        method: "platform/userInputRequested",
        params: {
          ...base,
          runId: event.run_id,
          approvalId,
        },
      };
    }
    if (
      requestMethod === "mcpServer/elicitation/request"
      && requestParams.mode === "form"
    ) {
      return {
        method: "platform/mcpFormRequested",
        params: {
          ...base,
          runId: event.run_id,
          approvalId,
        },
      };
    }
    const needsGenericApprovalCard = requestMethod === "item/fileChange/requestApproval"
      || requestMethod === "item/permissions/requestApproval"
      || (requestMethod === "mcpServer/elicitation/request" && requestParams.mode === "url");
    const method = needsGenericApprovalCard
      ? "item/commandExecution/requestApproval"
      : requestMethod;
    const params = needsGenericApprovalCard
      ? {
          ...requestParams,
          command: requestParams.credentialKind === "maps"
            ? "Map provider and API key required"
            : requestMethod === "mcpServer/elicitation/request"
            && typeof requestParams.message === "string"
            && requestParams.message.trim()
            ? requestParams.message
            : typeof requestParams.reason === "string" && requestParams.reason.trim()
            ? requestParams.reason
            : requestMethod === "item/fileChange/requestApproval"
              ? "Approve the requested file changes"
              : requestMethod === "item/permissions/requestApproval"
                ? "Approve the requested permissions"
                : typeof requestParams.serverName === "string" && requestParams.serverName.trim()
                  ? `Allow the ${requestParams.serverName} MCP server request?`
                  : "Allow the MCP server request?",
        }
      : requestParams;
    return {
      method,
      id: approvalId,
      params: { ...base, ...params },
    };
  }
  if (event.event_type === "platform.approval.resolved") {
    const approvalId = typeof data.approvalId === "string"
      ? data.approvalId
      : typeof data.requestId === "string" ? data.requestId : null;
    if (!approvalId) return null;
    const requestMethod = typeof data.requestMethod === "string" ? data.requestMethod : null;
    if (
      requestMethod === "mcpServer/elicitation/request"
      && data.requestMode === "form"
    ) {
      return {
        method: "platform/mcpFormResolved",
        params: { ...base, runId: event.run_id, approvalId },
      };
    }
    if (requestMethod !== "item/tool/requestUserInput") {
      return {
        method: "serverRequest/resolved",
        params: { ...base, requestId: approvalId, approvalStatus: data.approvalStatus },
      };
    }
    return {
      method: "platform/userInputResolved",
      params: { ...base, runId: event.run_id, approvalId },
    };
  }
  if (event.event_type === "platform.artifact.changed") {
    if (data.sourceType !== "platform/artifact/changed") return null;
    return {
      method: "platform/artifact/changed",
      params: { ...base, ...data },
    };
  }
  if (event.event_type === "codex.item.started" || event.event_type === "codex.item.completed") {
    const item = rawItem(event);
    if (!item) return null;
    return {
      method: event.event_type === "codex.item.started" ? "item/started" : "item/completed",
      params: { ...base, item },
    };
  }
  if (event.event_type === "codex.item.delta") {
    if (typeof data.sourceType !== "string") return null;
    return {
      method: data.sourceType,
      params: {
        ...base,
        itemId: event.item_id,
        delta: data.delta,
        summaryIndex: data.summaryIndex,
        contentIndex: data.contentIndex,
      },
    };
  }
  const lifecycleMethod = {
    "codex.turn.started": "turn/started",
    "codex.turn.completed": "turn/completed",
    "codex.thread.started": "thread/started",
    "codex.thread.completed": "thread/completed",
    "codex.thread.failed": "thread/failed",
  }[event.event_type];
  if (lifecycleMethod) return { method: lifecycleMethod, params: { ...base, ...data } };
  if (typeof data.sourceType === "string") {
    return { method: data.sourceType, params: { ...base, ...data } };
  }
  return null;
}

function runtimeThreadStatus(run: Run) {
  if (run.status === "recovery_pending") return { type: "idle", activeFlags: [] as string[] };
  if (run.active_turn_id) return { type: "active", activeFlags: [] as string[] };
  if (run.status === "failed") return { type: "error" };
  if (run.status === "running" || run.status === "completed") return { type: "idle" };
  return { type: run.status };
}

function runtimeStatusType(status: unknown): string | null {
  if (typeof status === "string") return status;
  if (!isRecord(status)) return null;
  return typeof status.type === "string" ? status.type : null;
}

function activeTurnIdForThread(run: Run, status: unknown): string | null {
  const statusType = runtimeStatusType(status);
  if (statusType && statusType !== "active" && statusType !== "running") return null;
  return run.status === "recovery_pending" ? null : run.active_turn_id;
}

function statusForThread(run: Run, status: unknown): unknown {
  return run.status === "recovery_pending" ? runtimeThreadStatus(run) : status;
}

/**
 * WebApp's typed client for the authenticated platform Server.
 *
 * The class name is retained so the restored UI does not change, but there is
 * no CodexMonitor daemon or browser RPC gateway behind it.
 */
export class CodexMonitorWebClient {
  private readonly platform: PlatformClient;
  private readonly threadContexts = new Map<string, ThreadContext>();
  private readonly taskEventSequences = new Map<string, number>();
  private readonly selectedRunByWorkspace = new Map<string, string>();
  private readonly threadStartDrafts = new Map<string, ThreadStartDraft>();

  constructor(options: WebClientOptions = {}) {
    this.platform = new PlatformClient({
      baseUrl: options.baseUrl ?? sameOriginBaseUrl(),
      token: options.token,
    });
  }

  setToken(token: string) {
    this.platform.setToken(token);
  }

  setBaseUrl(baseUrl: string) {
    this.platform.setBaseUrl(baseUrl);
  }

  async health(): Promise<GatewayHealth> {
    const health = await this.platform.health();
    return { ok: health.ok, name: "open-web-codex-server", version: health.version };
  }

  async listWorkspaces() {
    return (await this.platform.listWorkspaces()).map(platformWorkspace);
  }

  async addWorkspace(path: string) {
    const name = path.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || "Workspace";
    const project = await this.platform.createProject(name, path, "main");
    return platformWorkspace(await this.platform.createWorkspace({
      projectId: project.id,
      kind: "main",
      name,
    }));
  }

  async createWorkspace(name: string, _parentDir?: string) {
    const project = await this.platform.createManagedProject(name);
    return platformWorkspace(await this.platform.createWorkspace({
      projectId: project.id,
      kind: "main",
      name,
    }));
  }

  async removeWorkspace(id: string) {
    await this.platform.removeWorkspace(id);
  }

  async connectWorkspace(_workspaceId: string) {
    return undefined;
  }

  private async indexWorkspaceThreads(workspace: Workspace) {
    const rows = await this.platform.listProjectThreadContexts(workspace.project_id);
    return rows.flatMap(({ project, task, run }) => {
      if (run.workspace_id !== workspace.id || !run.codex_thread_id || task.status === "archived") {
        return [];
      }
      const context = {
        workspaceId: workspace.id,
        projectId: workspace.project_id,
        taskId: task.id,
        runId: run.id,
        rootThreadId: run.codex_thread_id,
      };
      this.threadContexts.set(run.codex_thread_id, context);
      return [{ project, task, run, threadId: run.codex_thread_id, context }];
    });
  }

  private async findThreadContext(threadId: string): Promise<ThreadContext> {
    const cached = this.threadContexts.get(threadId);
    if (cached) return cached;
    for (const workspace of await this.platform.listWorkspaces()) {
      const found = (await this.indexWorkspaceThreads(workspace))
        .find((entry) => entry.threadId === threadId);
      if (found) return found.context;
    }
    throw new Error("Thread is not available in an authorized project");
  }

  async taskIdForThread(threadId: string): Promise<string> {
    return (await this.findThreadContext(threadId)).taskId;
  }

  async runIdForThread(threadId: string): Promise<string> {
    return (await this.findThreadContext(threadId)).runId;
  }

  private async findRunEventContext(runId: string): Promise<ThreadContext> {
    const cached = [...this.threadContexts.values()]
      .find((context) => context.runId === runId);
    if (cached) return cached;
    for (const workspace of await this.platform.listWorkspaces()) {
      const found = (await this.indexWorkspaceThreads(workspace))
        .find((entry) => entry.run.id === runId);
      if (found) return found.context;
    }
    throw new Error("Run event is not available in an authorized project");
  }

  private async waitForThread(
    workspaceId: string,
    projectId: string,
    taskId: string,
    runId: string,
  ) {
    let lastTransientError: unknown = null;
    for (let attempt = 0; attempt < 600; attempt += 1) {
      try {
        const run = await this.platform.getRun(runId);
        lastTransientError = null;
        if (run.codex_thread_id && run.workspace_id) {
          this.threadContexts.set(run.codex_thread_id, {
            workspaceId,
            projectId,
            taskId,
            runId,
            rootThreadId: run.codex_thread_id,
          });
          return run;
        }
        if (["failed", "cancelled"].includes(run.status)) {
          const failure = run.failure_code
            ? ` Failure code: ${run.failure_code}.`
            : "";
          throw Object.assign(
            new Error(
              `Run ${run.status} before its Codex Thread was ready.${failure}`,
            ),
            { code: "run_terminal" },
          );
        }
      } catch (error) {
        if (
          error &&
          typeof error === "object" &&
          "code" in error &&
          (error as { code?: unknown }).code === "run_terminal"
        ) {
          throw error;
        }
        lastTransientError = error;
      }
      await new Promise((resolve) => globalThis.setTimeout(resolve, 200));
    }
    const detail =
      lastTransientError instanceof Error && lastTransientError.message
        ? ` Last platform error: ${lastTransientError.message}`
        : "";
    throw new Error(
      `Timed out waiting for the accepted Run to create its Codex Thread.${detail}`,
    );
  }

  private async readyRunForWorkspace(
    workspaceId: string,
    threadId?: string | null,
  ): Promise<Run | null> {
    if (threadId) {
      const context = await this.findThreadContext(threadId);
      if (context.workspaceId !== workspaceId) {
        throw new Error("Thread is not part of the selected Workspace");
      }
      this.selectedRunByWorkspace.set(workspaceId, context.runId);
      return await this.platform.getRun(context.runId);
    }
    const workspace = await this.platform.getWorkspace(workspaceId);
    const indexed = await this.indexWorkspaceThreads(workspace);
    const available = indexed
      .map((entry) => entry.run)
      .filter((run) => Boolean(run.workspace_id)
        && !["failed", "cancelled", "recovery_pending"].includes(run.status));
    const selectedRunId = this.selectedRunByWorkspace.get(workspaceId);
    const run = (selectedRunId ? available.find((candidate) => candidate.id === selectedRunId) : null)
      ?? available.find((candidate) => candidate.active_turn_id || candidate.status === "running")
      ?? available[0]
      ?? null;
    if (run) this.selectedRunByWorkspace.set(workspaceId, run.id);
    return run;
  }

  private async threadRecord(threadId: string) {
    const context = await this.findThreadContext(threadId);
    this.selectedRunByWorkspace.set(context.workspaceId, context.runId);
    const [task, workspace, run, history] = await Promise.all([
      this.platform.getTask(context.taskId),
      this.platform.getWorkspace(context.workspaceId),
      this.platform.getRun(context.runId),
      this.platform.readRunThread(context.runId),
    ]);
    const thread = history.thread;
    const status = statusForThread(run, thread.status);
    return {
      id: threadId,
      name: threadDisplayName(thread.name ?? task.title),
      preview: thread.preview || threadDisplayName(task.title),
      cwd: workspace.name,
      createdAt: thread.createdAt || task.created_at,
      updatedAt: thread.updatedAt || run.updated_at,
      activeTurnId: activeTurnIdForThread(run, status),
      modelProvider: task.model_provider,
      model: task.model,
      status,
      turns: thread.turns,
    };
  }

  async startThread(
    workspaceId: string,
    options: {
      operationId: string;
      providerId: string;
      modelId: string;
      copilot?: CopilotTaskSelection | null;
      onRunAccepted?: (accepted: AcceptedThreadStart) => void;
    },
  ) {
    const operationId = options.operationId.trim();
    if (!operationId || operationId.length > 256) {
      throw new Error("Thread start operation identity is invalid.");
    }
    const workspace = await this.platform.getWorkspace(workspaceId);
    const draftKey = threadStartDraftKey(workspaceId, operationId);
    let draft = this.threadStartDrafts.get(draftKey);
    if (!draft) {
      draft = {
        taskId: null,
        taskPromise: null,
        runIdempotencyKey: newIdempotencyKey(),
        acceptedRunId: null,
      };
      this.threadStartDrafts.set(draftKey, draft);
    }
    const taskWasReused = draft.taskId !== null;
    let task: Task;
    if (draft.taskId) {
      task = await this.platform.getTask(draft.taskId);
    } else {
      draft.taskPromise ??= this.platform.createTask(
        workspace.project_id,
        workspace.id,
        "Thread",
        {
          providerId: options.providerId,
          modelId: options.modelId,
        },
        options.copilot,
      );
      try {
        task = await draft.taskPromise;
        draft.taskId = task.id;
      } catch (error) {
        if (this.threadStartDrafts.get(draftKey) === draft) {
          this.threadStartDrafts.delete(draftKey);
        }
        throw error;
      } finally {
        draft.taskPromise = null;
      }
    }
    if (
      taskWasReused &&
      !draft.acceptedRunId &&
      (task.model_provider !== options.providerId || task.model !== options.modelId)
    ) {
      await this.platform.updateTaskModelSelection(
        task.id,
        options.providerId,
        options.modelId,
      );
    }
    if (
      options.copilot &&
      task.copilot_package_id !== options.copilot.packageId
    ) {
      throw new Error("Thread start operation was already bound to another Copilot package.");
    }
    const run = draft.acceptedRunId
      ? await this.platform.getRun(draft.acceptedRunId)
      : (
          await this.platform.startRun(task.id, {
            idempotencyKey: draft.runIdempotencyKey,
          })
        ).run;
    draft.acceptedRunId = run.id;
    options.onRunAccepted?.({ taskId: task.id, runId: run.id });
    try {
      const ready = await this.waitForThread(
        workspaceId,
        workspace.project_id,
        task.id,
        run.id,
      );
      if (this.threadStartDrafts.get(draftKey) === draft) {
        this.threadStartDrafts.delete(draftKey);
      }
      return { thread: await this.threadRecord(ready.codex_thread_id as string) };
    } catch (error) {
      if (
        error &&
        typeof error === "object" &&
        "code" in error &&
        (error as { code?: unknown }).code === "run_terminal" &&
        this.threadStartDrafts.get(draftKey) === draft
      ) {
        this.threadStartDrafts.delete(draftKey);
      }
      throw error;
    }
  }

  copilotProfileStatus() {
    return this.platform.copilotProfileStatus();
  }

  activateCopilot(packageId: string) {
    return this.platform.activateCopilot(packageId);
  }

  deactivateCopilot(packageId: string) {
    return this.platform.deactivateCopilot(packageId);
  }

  async getSupervisorOverview(
    threadId: string,
  ): Promise<SupervisorOverviewData | null> {
    const context = await this.findThreadContext(threadId);
    const [task, agents, activities, executions, artifacts] = await Promise.all([
      this.platform.getTask(context.taskId),
      this.platform.listRunAgents(context.runId),
      this.platform.listRunAgentActivities(context.runId),
      this.platform.listRunAgentExecutions(context.runId),
      this.platform.listTaskArtifacts(context.taskId),
    ]);
    return { taskTitle: task.title, agents, activities, executions, artifacts };
  }

  async listRunUserInputRequests(runId: string): Promise<PendingUserInputSummary[]> {
    return this.platform.listRunUserInputRequests(runId);
  }

  async listThreadUserInputRequests(threadId: string): Promise<PendingUserInputSummary[]> {
    const context = await this.findThreadContext(threadId);
    return this.platform.listRunUserInputRequests(context.runId);
  }

  listRunMcpFormRequests(runId: string): Promise<PendingMcpFormSummary[]> {
    return this.platform.listRunMcpFormRequests(runId);
  }

  async listThreadMcpFormRequests(threadId: string): Promise<PendingMcpFormSummary[]> {
    const context = await this.findThreadContext(threadId);
    return this.platform.listRunMcpFormRequests(context.runId);
  }

  respondToUserInput(
    approvalId: string,
    version: number,
    answers: Record<string, { answers: string[] }>,
  ): Promise<void> {
    return this.platform.respondUserInput(approvalId, answers, version);
  }

  respondToMcpForm(
    approvalId: string,
    version: number,
    action: McpFormResponseAction,
    content?: McpFormContent,
  ): Promise<void> {
    return this.platform.respondMcpForm(approvalId, action, content, version);
  }

  async listAgentThreadTurns(
    rootThreadId: string,
    agentThreadId: string,
  ): Promise<ThreadHistoryTurn[]> {
    const context = await this.findThreadContext(rootThreadId);
    return this.platform.listRunAgentThreadTurns(context.runId, agentThreadId);
  }

  readArtifactContent(artifactId: string): Promise<ArtifactContent> {
    return this.platform.readArtifactContent(artifactId);
  }

  downloadArtifact(artifactId: string): Promise<{ blob: Blob; filename: string }> {
    return this.platform.downloadArtifact(artifactId);
  }

  async listThreads(workspaceId: string) {
    const workspace = await this.platform.getWorkspace(workspaceId);
    const entries = await this.indexWorkspaceThreads(workspace);
    return {
      data: entries.map(({ task, run, threadId }) => ({
        id: threadId,
        name: threadDisplayName(task.title),
        preview: threadDisplayName(task.title),
        cwd: workspace.name,
        createdAt: task.created_at,
        updatedAt: run.updated_at,
        activeTurnId: run.status === "recovery_pending" ? null : run.active_turn_id,
        modelProvider: task.model_provider,
        model: task.model,
        status: runtimeThreadStatus(run).type,
      })),
      nextCursor: null,
    };
  }

  async archiveThread(_workspaceId: string, threadId: string) {
    const context = await this.findThreadContext(threadId);
    this.selectedRunByWorkspace.set(context.workspaceId, context.runId);
    return await this.platform.archiveRunThread(context.runId);
  }

  async listModelProviders() {
    return await this.platform.listProviders();
  }

  async writeModelProvider(input: JsonRecord) {
    const id = String(input.id ?? "").trim();
    if (!id) throw new Error("Provider id is required");
    const action = typeof input.action === "string" ? input.action : "upsert";
    if (action === "select") return await this.platform.selectProvider(id);
    if (action === "delete") return await this.platform.deleteProvider(id);
    if (action === "fetch") return await this.platform.refreshProviderModels(id);
    if (action === "contexts") {
      const contexts = Array.isArray(input.contexts)
        ? input.contexts.flatMap((value) => {
            if (!isRecord(value)) return [];
            const modelId = typeof value.modelId === "string" ? value.modelId.trim() : "";
            const contextWindow = Number(value.contextWindow);
            return modelId && Number.isSafeInteger(contextWindow) && contextWindow >= 1_024
              ? [{ modelId, contextWindow }]
              : [];
          })
        : [];
      if (contexts.length === 0) throw new Error("At least one valid model context is required");
      let response: unknown = null;
      // Keep updates sequential: each Runtime write reads and replaces the
      // Provider model catalog, so parallel writes could discard a sibling edit.
      for (const context of contexts) {
        response = await this.platform.updateProviderModel(
          id,
          context.modelId,
          context.contextWindow,
        );
      }
      return response;
    }
    if (action === "context") {
      return await this.platform.updateProviderModel(
        id,
        String(input.modelId ?? ""),
        Number(input.contextWindow),
      );
    }
    if (action === "capability") {
      return await this.platform.updateProviderModel(
        id,
        String(input.modelId ?? ""),
        Number(input.contextWindow),
        input.supportsSearchTool === true,
      );
    }
    const credentialMode = typeof input.credentialMode === "string"
      ? input.credentialMode
      : "preserve";
    const credentials = credentialMode === "environment"
      ? { mode: "environment", envKey: String(input.envKey ?? "") }
      : credentialMode === "direct"
        ? { mode: "direct", apiKey: String(input.apiKey ?? "") }
        : credentialMode === "none"
          ? { mode: "none" }
          : { mode: "preserve" };
    const wireApi = typeof input.wireApi === "string" ? input.wireApi : "responses";
    return await this.platform.upsertProvider(id, {
      name: String(input.name ?? ""),
      baseUrl: String(input.baseUrl ?? ""),
      wireApi,
      credentials,
      supportsFunctionTools: wireApi === "chat",
      select: input.select === true,
    });
  }

  async selectProviderModel(
    _workspaceId: string,
    providerId: string,
    modelId: string,
  ) {
    return await this.platform.selectProviderModel(providerId, modelId);
  }

  async updateThreadModelSelection(
    _workspaceId: string,
    threadId: string,
    providerId: string,
    modelId: string,
  ) {
    const context = await this.findThreadContext(threadId);
    return await this.platform.updateTaskModelSelection(
      context.taskId,
      providerId,
      modelId,
    );
  }

  async listModels(
    _workspaceId: string,
    providerId?: string | null,
    preferredModelId?: string | null,
  ) {
    const [catalog, config] = providerId
      ? [await this.platform.listProviders(), null]
      : await Promise.all([
        this.platform.listProviders(),
        this.platform.getConfigModel(),
      ]);
    const selectedProviderId = providerId ?? catalog.currentProviderId;
    const provider = catalog.data.find((entry) => entry.id === selectedProviderId);
    const visibleModels = (provider?.models ?? []).filter((model) => model.showInPicker !== false);
    const configuredModel = preferredModelId?.trim()
      || (selectedProviderId === catalog.currentProviderId ? catalog.currentModelId?.trim() : null)
      || config?.model?.trim()
      || null;
    if (configuredModel) {
      const selectedIndex = visibleModels.findIndex((model) => model.modelId === configuredModel);
      if (selectedIndex > 0) {
        visibleModels.unshift(...visibleModels.splice(selectedIndex, 1));
      }
    }
    return {
      data: visibleModels.map((model, index) => ({
        id: model.modelId,
        model: model.modelId,
        displayName: model.modelName ?? model.modelId,
        description: "",
        supportedReasoningEfforts: [],
        defaultReasoningEffort: null,
        isDefault: configuredModel ? model.modelId === configuredModel : index === 0,
      })),
    };
  }

  async listMcpServerStatus(workspaceId: string, threadId?: string | null) {
    const run = await this.readyRunForWorkspace(workspaceId, threadId);
    return run ? await this.platform.profileMcpServers(run.id, null, 100) : { data: [] };
  }

  async getAccountRateLimits(_workspaceId: string) {
    return await this.platform.profileRateLimits();
  }

  async resumeThread(_workspaceId: string, threadId: string) {
    return { thread: await this.threadRecord(threadId) };
  }

  async readThread(_workspaceId: string, threadId: string) {
    return { thread: await this.threadRecord(threadId) };
  }

  async listThreadTurns(_workspaceId: string, threadId: string) {
    const context = await this.findThreadContext(threadId);
    this.selectedRunByWorkspace.set(context.workspaceId, context.runId);
    return await this.platform.listRunThreadTurns(context.runId);
  }

  async listWorkspaceFiles(workspaceId: string, threadId?: string | null) {
    if (threadId) await this.readyRunForWorkspace(workspaceId, threadId);
    return await this.platform.listWorkspaceFiles(workspaceId);
  }

  async uploadWorkspaceFiles(
    workspaceId: string,
    files: File[],
    threadId?: string | null,
    options: { overwrite?: boolean; paths?: string[] } = {},
  ) {
    if (threadId) await this.readyRunForWorkspace(workspaceId, threadId);
    return await this.platform.uploadWorkspaceFiles(workspaceId, files, options);
  }

  async readWorkspaceFile(workspaceId: string, path: string, threadId?: string | null) {
    if (threadId) await this.readyRunForWorkspace(workspaceId, threadId);
    return await this.platform.readWorkspaceFile(workspaceId, path);
  }

  async downloadWorkspaceFile(workspaceId: string, path: string, threadId?: string | null) {
    if (threadId) await this.readyRunForWorkspace(workspaceId, threadId);
    return await this.platform.downloadWorkspaceFile(workspaceId, path);
  }

  async deleteWorkspaceFile(workspaceId: string, path: string, threadId?: string | null) {
    if (threadId) await this.readyRunForWorkspace(workspaceId, threadId);
    return await this.platform.deleteWorkspaceFile(workspaceId, path);
  }

  async getGitStatus(workspaceId: string, threadId?: string | null) {
    if (threadId) await this.readyRunForWorkspace(workspaceId, threadId);
    const status = await this.platform.workspaceStatus(workspaceId);
    return {
      files: status.changes.map((change) => ({
        path: change.path,
        status: change.status,
        additions: change.additions ?? 0,
        deletions: change.deletions ?? 0,
      })) as GitFileStatus[],
    };
  }

  async sendUserMessage(
    _workspaceId: string,
    threadId: string,
    text: string,
    model?: string | null,
    modelProvider?: string | null,
    mapCardRef?: string | null,
  ) {
    const context = await this.findThreadContext(threadId);
    this.selectedRunByWorkspace.set(context.workspaceId, context.runId);
    const response = await this.platform.sendMessage(context.taskId, text, {
      model,
      modelProvider,
      mapCardRef,
    });
    return {
      status: response.status,
      threadId: response.thread_id,
      threadName: response.thread_name ?? null,
      turn: { id: response.turn_id, status: "inProgress" },
    };
  }

  async interruptTurn(_workspaceId: string, threadId: string, turnId: string) {
    const context = await this.findThreadContext(threadId);
    this.selectedRunByWorkspace.set(context.workspaceId, context.runId);
    return await this.platform.interruptRun(context.runId, turnId);
  }

  async steerTurn(_workspaceId: string, threadId: string, turnId: string, text: string) {
    const context = await this.findThreadContext(threadId);
    this.selectedRunByWorkspace.set(context.workspaceId, context.runId);
    return await this.platform.steerRun(context.runId, turnId, text);
  }

  subscribeAppServerEvents(
    onEvent: (event: AppServerEvent) => void,
    status: EventSubscriptionStatus = {},
  ) {
    let delivery = Promise.resolve();
    let hasBeenOnline = false;
    const enqueue = (operation: () => Promise<void>) => {
      delivery = delivery.then(operation).catch(() => {
        status.onError?.();
      });
    };
    const deliver = async (event: RunEvent) => {
      if (!event.thread_id) return;
      const message = runtimeMessage(event);
      if (!message) return;
      const context = await this.findRunEventContext(event.run_id);
      const previous = this.taskEventSequences.get(context.taskId) ?? 0;
      if (event.sequence <= previous) return;
      this.taskEventSequences.set(context.taskId, event.sequence);
      onEvent({
        workspace_id: context.workspaceId,
        run_id: context.runId,
        root_thread_id: context.rootThreadId,
        message,
      });
    };
    const replayDurableEvents = async () => {
      const contexts = new Map(
        [...this.threadContexts.values()].map((context) => [context.taskId, context]),
      );
      for (const [taskId] of contexts) {
        const afterSequence = this.taskEventSequences.get(taskId) ?? 0;
        for (const event of await this.platform.listAllEvents(taskId, afterSequence)) {
          await deliver(event);
        }
      }
    };
    const establishInitialEventBaseline = async () => {
      const taskIds = [...new Set(
        [...this.threadContexts.values()].map((context) => context.taskId),
      )];
      const sequences = await Promise.all(
        taskIds.map(async (taskId) => ({
          taskId,
          sequence: await this.platform.latestEventSequence(taskId),
        })),
      );
      for (const { taskId, sequence } of sequences) {
        this.taskEventSequences.set(
          taskId,
          Math.max(this.taskEventSequences.get(taskId) ?? 0, sequence),
        );
      }
    };
    return this.platform.subscribe(
      (event) => {
        enqueue(() => deliver(event));
      },
      (state) => {
        if (state === "online") {
          status.onOpen?.();
          enqueue(async () => {
            if (!hasBeenOnline) {
              hasBeenOnline = true;
              for (const workspace of await this.platform.listWorkspaces()) {
                await this.indexWorkspaceThreads(workspace);
              }
              // The project/thread context and authoritative Codex history are
              // the initial UI snapshot. Establish a durable cursor at that
              // snapshot instead of replaying every historical delta through
              // React on each page refresh. The WebSocket is already connected,
              // so newer events remain queued behind this baseline.
              await establishInitialEventBaseline();
              await this.replayPendingApprovals(onEvent);
              return;
            }
            await replayDurableEvents();
            await this.replayPendingApprovals(onEvent);
          });
        }
        if (state === "resync") enqueue(replayDurableEvents);
        if (state === "offline") status.onError?.();
      },
    );
  }

  private async replayPendingApprovals(onEvent: (event: AppServerEvent) => void) {
    const pending = new Set(
      (await this.platform.listApprovals())
        .filter((approval) => approval.state === "pending" || approval.state === "delivery_unknown")
        .map((approval) => approval.id),
    );
    if (pending.size === 0) return;
    for (const workspace of await this.platform.listWorkspaces()) {
      const contexts = await this.indexWorkspaceThreads(workspace);
      const tasks = new Map(contexts.map((entry) => [entry.task.id, entry.context]));
      for (const [taskId, context] of tasks) {
        for (const event of await this.platform.listAllEvents(taskId)) {
          if (event.event_type !== "platform.approval.requested") continue;
          const data = isRecord(event.payload.data) ? event.payload.data : {};
          if (typeof data.approvalId !== "string" || !pending.has(data.approvalId)) continue;
          const message = runtimeMessage(event);
          if (!message) continue;
          this.taskEventSequences.set(
            context.taskId,
            Math.max(this.taskEventSequences.get(context.taskId) ?? 0, event.sequence),
          );
          onEvent({
            workspace_id: context.workspaceId,
            run_id: context.runId,
            root_thread_id: context.rootThreadId,
            message,
          });
        }
      }
    }
  }

  async resolveApproval(workspaceId: string, threadId: string, decision: string) {
    const approval = (await this.platform.listApprovals())
      .find((entry) => entry.threadId === threadId
        && (entry.state === "pending" || entry.state === "delivery_unknown"));
    if (!approval) throw new Error("Approval is no longer pending");
    await this.platform.decideApproval(
      approval.id,
      decision === "accept" ? "accept" : "decline",
      approval.version,
    );
    return { workspaceId };
  }

  async respondToServerRequest(
    _workspaceId: string,
    requestId: number | string,
    result: JsonRecord,
  ) {
    const approval = await this.pendingApproval(String(requestId));
    if (isRecord(result.answers)) {
      await this.platform.respondUserInput(
        approval.id,
        result.answers as Record<string, { answers: string[] }>,
        approval.version,
      );
      return {};
    }
    await this.platform.decideApproval(
      approval.id,
      result.decision === "accept" ? "accept" : "decline",
      approval.version,
    );
    return {};
  }

  private async pendingApproval(id: string): Promise<Approval> {
    const approval = (await this.platform.listApprovals())
      .find((entry) => entry.id === id
        && (entry.state === "pending" || entry.state === "delivery_unknown"));
    if (!approval) throw new Error("Approval is no longer pending");
    return approval;
  }
}
