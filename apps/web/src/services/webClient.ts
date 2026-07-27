import { PlatformClient } from "../../browser/client";
import type { Approval, Project, Run, RunEvent } from "../../browser/types";
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

type EventSubscriptionStatus = {
  onOpen?: () => void;
  onError?: () => void;
};

type ThreadContext = {
  projectId: string;
  taskId: string;
  runId: string;
};

type JsonRecord = Record<string, unknown>;

function isRecord(value: unknown): value is JsonRecord {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function sameOriginBaseUrl() {
  return typeof window === "undefined" ? "" : window.location.origin;
}

function projectWorkspace(project: Project): WorkspaceInfo {
  return {
    id: project.id,
    name: project.name,
    path: project.git_url,
    connected: true,
    kind: "main",
    settings: { sidebarCollapsed: false },
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
    const needsGenericApprovalCard = requestMethod === "item/fileChange/requestApproval"
      || requestMethod === "item/permissions/requestApproval"
      || requestMethod === "mcpServer/elicitation/request";
    const method = needsGenericApprovalCard
      ? "item/commandExecution/requestApproval"
      : requestMethod;
    const params = needsGenericApprovalCard
      ? {
          ...requestParams,
          command: requestMethod === "mcpServer/elicitation/request"
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
  if (run.active_turn_id) return { type: "active", activeFlags: [] as string[] };
  if (run.status === "failed") return { type: "error" };
  if (run.status === "running" || run.status === "completed") return { type: "idle" };
  return { type: run.status };
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
  private readonly selectedRunByProject = new Map<string, string>();

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
    return (await this.platform.listProjects()).map(projectWorkspace);
  }

  async addWorkspace(path: string) {
    const name = path.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || "Workspace";
    return projectWorkspace(await this.platform.createProject(name, path, "main"));
  }

  async createWorkspace(name: string, _parentDir?: string) {
    return projectWorkspace(await this.platform.createManagedProject(name));
  }

  async removeWorkspace(id: string) {
    await this.platform.deleteProject(id);
  }

  async connectWorkspace(_workspaceId: string) {
    return undefined;
  }

  private async indexProjectThreads(projectId: string) {
    const rows = await this.platform.listProjectThreadContexts(projectId);
    return rows.flatMap(({ project, task, run }) => {
      if (run.workspace_kind !== "main" || !run.codex_thread_id || task.status === "archived") {
        return [];
      }
      const context = { projectId, taskId: task.id, runId: run.id };
      this.threadContexts.set(run.codex_thread_id, context);
      return [{ project, task, run, threadId: run.codex_thread_id, context }];
    });
  }

  private async findThreadContext(threadId: string): Promise<ThreadContext> {
    const cached = this.threadContexts.get(threadId);
    if (cached) return cached;
    for (const project of await this.platform.listProjects()) {
      const found = (await this.indexProjectThreads(project.id))
        .find((entry) => entry.threadId === threadId);
      if (found) return found.context;
    }
    throw new Error("Thread is not available in an authorized project");
  }

  private async waitForThread(projectId: string, taskId: string, runId: string) {
    for (let attempt = 0; attempt < 600; attempt += 1) {
      const run = await this.platform.getRun(runId);
      if (run.codex_thread_id && run.workspace_id) {
        this.threadContexts.set(run.codex_thread_id, { projectId, taskId, runId });
        return run;
      }
      if (["failed", "cancelled"].includes(run.status)) {
        throw new Error(`Run ${run.status} before its Codex Thread was ready`);
      }
      await new Promise((resolve) => window.setTimeout(resolve, 200));
    }
    throw new Error("Timed out waiting for the Codex Thread to become ready");
  }

  private async readyRunForWorkspace(
    workspaceId: string,
    threadId?: string | null,
  ): Promise<Run | null> {
    if (threadId) {
      const context = await this.findThreadContext(threadId);
      if (context.projectId !== workspaceId) {
        throw new Error("Thread is not part of the selected project");
      }
      this.selectedRunByProject.set(workspaceId, context.runId);
      return await this.platform.getRun(context.runId);
    }
    const indexed = await this.indexProjectThreads(workspaceId);
    const available = indexed
      .map((entry) => entry.run)
      .filter((run) => Boolean(run.workspace_id) && !["failed", "cancelled"].includes(run.status));
    const selectedRunId = this.selectedRunByProject.get(workspaceId);
    const run = (selectedRunId ? available.find((candidate) => candidate.id === selectedRunId) : null)
      ?? available.find((candidate) => candidate.active_turn_id || candidate.status === "running")
      ?? available[0]
      ?? null;
    if (run) this.selectedRunByProject.set(workspaceId, run.id);
    return run;
  }

  private async requireRunForWorkspace(workspaceId: string) {
    const run = await this.readyRunForWorkspace(workspaceId);
    if (!run) throw new Error("This project does not have a ready Run workspace yet");
    return run;
  }

  private async threadRecord(threadId: string) {
    const context = await this.findThreadContext(threadId);
    this.selectedRunByProject.set(context.projectId, context.runId);
    const [task, project, run, history] = await Promise.all([
      this.platform.getTask(context.taskId),
      this.platform.getProject(context.projectId),
      this.platform.getRun(context.runId),
      this.platform.readRunThread(context.runId),
    ]);
    const thread = history.thread;
    return {
      id: threadId,
      name: threadDisplayName(thread.name ?? task.title),
      preview: thread.preview || threadDisplayName(task.title),
      cwd: project.git_url,
      createdAt: thread.createdAt || task.created_at,
      updatedAt: thread.updatedAt || run.updated_at,
      activeTurnId: run.active_turn_id,
      modelProvider: task.model_provider,
      model: task.model,
      status: thread.status,
      turns: thread.turns,
    };
  }

  async startThread(workspaceId: string) {
    const task = await this.platform.createTask(workspaceId, "Thread");
    const { run } = await this.platform.startRun(task.id);
    const ready = await this.waitForThread(workspaceId, task.id, run.id);
    return { thread: await this.threadRecord(ready.codex_thread_id as string) };
  }

  async listThreads(workspaceId: string) {
    const entries = await this.indexProjectThreads(workspaceId);
    return {
      data: entries.map(({ project, task, run, threadId }) => ({
        id: threadId,
        name: threadDisplayName(task.title),
        preview: threadDisplayName(task.title),
        cwd: project.git_url,
        createdAt: task.created_at,
        updatedAt: run.updated_at,
        activeTurnId: run.active_turn_id,
        modelProvider: task.model_provider,
        model: task.model,
        status: runtimeThreadStatus(run).type,
      })),
      nextCursor: null,
    };
  }

  async archiveThread(_workspaceId: string, threadId: string) {
    const context = await this.findThreadContext(threadId);
    this.selectedRunByProject.set(context.projectId, context.runId);
    return await this.platform.archiveRunThread(context.runId);
  }

  async listModelProviders(_workspaceId: string) {
    return await this.platform.listProviders();
  }

  async writeModelProvider(_workspaceId: string, input: JsonRecord) {
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
    return await this.platform.upsertProvider(id, {
      name: String(input.name ?? ""),
      baseUrl: String(input.baseUrl ?? ""),
      wireApi: typeof input.wireApi === "string" ? input.wireApi : "responses",
      credentials,
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
    this.selectedRunByProject.set(context.projectId, context.runId);
    return await this.platform.listRunThreadTurns(context.runId);
  }

  async listWorkspaceFiles(workspaceId: string, threadId?: string | null) {
    const run = await this.readyRunForWorkspace(workspaceId, threadId);
    return run ? await this.platform.listWorkspaceFiles(run.id) : [];
  }

  async readWorkspaceFile(workspaceId: string, path: string, threadId?: string | null) {
    const run = threadId
      ? await this.readyRunForWorkspace(workspaceId, threadId)
      : await this.requireRunForWorkspace(workspaceId);
    if (!run) throw new Error("This project does not have a ready Run workspace yet");
    return await this.platform.readWorkspaceFile(run.id, path);
  }

  async getGitStatus(workspaceId: string, threadId?: string | null) {
    const run = await this.readyRunForWorkspace(workspaceId, threadId);
    if (!run) return { files: [] as GitFileStatus[] };
    const status = await this.platform.workspaceStatus(run.id);
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
  ) {
    const context = await this.findThreadContext(threadId);
    this.selectedRunByProject.set(context.projectId, context.runId);
    const response = await this.platform.sendMessage(context.taskId, text, {
      model,
      modelProvider,
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
    this.selectedRunByProject.set(context.projectId, context.runId);
    return await this.platform.interruptRun(context.runId, turnId);
  }

  async steerTurn(_workspaceId: string, threadId: string, turnId: string, text: string) {
    const context = await this.findThreadContext(threadId);
    this.selectedRunByProject.set(context.projectId, context.runId);
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
      const context = await this.findThreadContext(event.thread_id);
      const previous = this.taskEventSequences.get(context.taskId) ?? 0;
      if (event.sequence <= previous) return;
      this.taskEventSequences.set(context.taskId, event.sequence);
      onEvent({ workspace_id: context.projectId, message });
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
              for (const project of await this.platform.listProjects()) {
                await this.indexProjectThreads(project.id);
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
    for (const project of await this.platform.listProjects()) {
      const contexts = await this.indexProjectThreads(project.id);
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
          onEvent({ workspace_id: context.projectId, message });
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
