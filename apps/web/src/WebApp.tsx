import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { AppServerEvent, RequestUserInputRequest, RequestUserInputResponse, ThreadTokenUsage, WorkspaceInfo } from "./types";
import { CodexMonitorWebClient } from "./services/webClient";
import Layout from "./components/Layout";
import Sidebar from "./components/Sidebar";
import Conversation from "./components/Conversation";
import FileManager from "./components/FileManager";
import type { GoalInfo } from "./components/Conversation/GoalBanner";
import type { QueuedFollowUp } from "./components/Conversation/FollowUpQueue";
import type { ModelProviderSummary, ModelSummary } from "./components/Conversation/Composer";
import { parseModelListResponse } from "./features/models/utils/modelListResponse";
import { agentMessagePhase, appendTerminalInteractionOutput, buildWebThreadHistory, commandText, isUserThreadItem, mergeWebThreadHistory, unwrapWebRpcResult, webLogEntryFromThreadItem } from "./utils/webThreadHistory";
import { normalizeTokenUsage } from "./features/threads/utils/threadNormalize";
import { normalizePlanUpdate } from "./features/threads/utils/threadNormalize";
import { parseWebTurnDiff } from "./utils/webTurnDiff";
import { isWebAppServerRecoveryEvent, parseCodexStderr, parseWebAppServerError } from "./utils/webAppServerError";
import { parseWebUserInputRequest } from "./utils/webUserInput";
import { summarizeWebAppServerEvent } from "./utils/webAppServerEventSummary";
import { stripLeadingProviderSentinel } from "./utils/providerText";
import { mergeRateLimits, parseInitialMcpServers, parseInitialRateLimits } from "./utils/webInitialStatus";
import { appendWebLogEntry } from "./utils/webApprovalLog";
import { rememberAppServerEvent } from "./utils/webAppServerEventDedup";
import { getAppServerThreadId } from "./utils/appServerEvents";
import { finalizeInterruptedTurnEntries } from "./utils/webInterruptedTurn";
import {
  parseInlineVisualizationArtifact,
  type InlineVisualizationArtifact,
} from "./utils/replyCards";
import {
  isApprovalOutcome,
  parseApprovalStatus,
  type ApprovalStatus,
} from "./utils/approvalStatus";
import "./styles/web.css";
import "./styles/web-refactor.css";

/* ─────────── Types ─────────── */

export type AgentMessagePhase = "commentary" | "final_answer";

export type LogEntry = {
  id: string;
  level: "event" | "error" | "info" | "user" | "assistant" | "system";
  text: string;
  messagePhase?: AgentMessagePhase;
  approvalId?: string;
  approvalRequestId?: number | string;
  approvalStatus?: ApprovalStatus;
  approvalMode?: string;
  approvalUrl?: string;
  approvalServerName?: string;
  approvalTool?: string;
  kind?: "reasoning" | "tool" | "diff" | "approval" | "command_exec" | "connection";
  toolType?: string;
  toolTitle?: string;
  toolStatus?: string;
  toolDetail?: string;
  toolOutput?: string;
  inlineArtifacts?: InlineVisualizationArtifact[];
  reasoningSummary?: string;
  filePath?: string;
  diffTitle?: string;
  diffLines?: { type: "add" | "del" | "ctx"; text: string }[];
  meta?: string;
  streaming?: boolean;
  cmdExitCode?: number;
  cmdDurationMs?: number;
  cmdCwd?: string;
  cmdOutput?: string;
  cmdActions?: { type: string; path: string }[];
};

type WebEventLogEntry = Omit<LogEntry, "id"> & { id?: string };

type GatewayState = "checking" | "online" | "offline";

type ThreadInfo = {
  id: string;
  label: string;
  updatedAt: number;
  modelProvider?: string | null;
  model?: string | null;
  turnCount?: number;
  status?: string;
  optimistic?: boolean;
  creationStatus?: "creating" | "failed";
  creationError?: string;
};

type ThreadTranscriptCacheEntry = {
  messages: LogEntry[];
  updatedAt: number;
  status: string;
  thinking: boolean;
  activeTurnId: string | null;
  turnStartedAt: number | null;
};

function parseThreadStatus(value: unknown): string {
  if (typeof value === "string") return value;
  if (value && typeof value === "object") {
    const type = (value as Record<string, unknown>).type;
    if (typeof type === "string") return type;
  }
  return "idle";
}

/* ─────────── Helpers ─────────── */

function extractThreadId(result: Record<string, unknown> | null | undefined) {
  if (!result) return null;
  const candidates = [result.threadId, result.thread_id, result.id];
  for (const c of candidates) if (typeof c === "string" && c.trim()) return c;
  const thread = result.thread;
  if (thread && typeof thread === "object") {
    const r = thread as Record<string, unknown>;
    if (typeof r.id === "string") return r.id;
    if (typeof r.threadId === "string") return r.threadId;
  }
  // Handle nested start_thread response: {result: {thread: {id: "..."}}}
  const inner = result.result;
  if (inner && typeof inner === "object") {
    return extractThreadId(inner as Record<string, unknown>);
  }
  return null;
}

function normalizeThreadName(value: unknown): string | null {
  const name = typeof value === "string" ? value.trim() : "";
  if (!name) return null;
  return name === "New Agent" ? "Thread" : name;
}

function extractThreadName(result: Record<string, unknown> | null | undefined): string | null {
  if (!result) return null;
  const thread = result.thread;
  if (thread && typeof thread === "object") {
    const record = thread as Record<string, unknown>;
    const candidates = [record.name, record.threadName, record.thread_name, record.title];
    for (const candidate of candidates) {
      const name = normalizeThreadName(candidate);
      if (name) return name;
    }
  }
  const candidates = [result.name, result.threadName, result.thread_name, result.title];
  for (const candidate of candidates) {
    const name = normalizeThreadName(candidate);
    if (name) return name;
  }
  const inner = result.result;
  return inner && typeof inner === "object"
    ? extractThreadName(inner as Record<string, unknown>)
    : null;
}

const newLogId = () =>
  crypto.randomUUID?.() ?? `${Date.now()}-${Math.random().toString(36).slice(2)}`;

function parseThreadUpdatedAt(value: unknown): number {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string") {
    const parsed = Date.parse(value);
    if (Number.isFinite(parsed)) return parsed;
  }
  return 0;
}

function parseModelProviderCatalog(value: unknown): {
  providers: ModelProviderSummary[];
  currentProviderId: string | null;
  currentModelId: string | null;
} {
  const payload = unwrapWebRpcResult(value);
  const record = payload && typeof payload === "object"
    ? payload as Record<string, unknown>
    : {};
  const currentProviderId = typeof record.currentProviderId === "string"
    ? record.currentProviderId
    : null;
  const currentModelId = typeof record.currentModelId === "string"
    ? record.currentModelId
    : null;
  const rawProviders = Array.isArray(record.data) ? record.data : [];
  const providers = rawProviders.flatMap((value): ModelProviderSummary[] => {
    if (!value || typeof value !== "object") return [];
    const provider = value as Record<string, unknown>;
    if (typeof provider.id !== "string" || typeof provider.name !== "string") return [];
    const models = Array.isArray(provider.models) ? provider.models.flatMap((value) => {
      if (!value || typeof value !== "object") return [];
      const model = value as Record<string, unknown>;
      if (typeof model.modelId !== "string" || model.showInPicker === false) return [];
      return [{
        modelId: model.modelId,
        modelName: typeof model.modelName === "string" ? model.modelName : null,
        contextWindow: typeof model.contextWindow === "number" ? model.contextWindow : null,
      }];
    }) : [];
    return [{
      id: provider.id,
      name: provider.name,
      kind: provider.kind === "local" || provider.kind === "custom" ? provider.kind : "builtIn",
      isCurrent: provider.id === currentProviderId,
      modelCount: models.length,
      baseUrl: typeof provider.baseUrl === "string" ? provider.baseUrl : null,
      envKey: typeof provider.envKey === "string" ? provider.envKey : null,
      wireApi: typeof provider.wireApi === "string" ? provider.wireApi : "responses",
      canEdit: provider.canEdit === true,
      canDelete: provider.canDelete === true,
      canFetchModels: provider.canFetchModels === true,
      models,
    }];
  });
  return { providers, currentProviderId, currentModelId };
}

function modelSummariesForProvider(
  provider: ModelProviderSummary | undefined,
): ModelSummary[] {
  return (provider?.models ?? []).map((model) => ({
    id: model.modelId,
    model: model.modelId,
    displayName: model.modelName ?? model.modelId,
  }));
}

/* ─────────── Component ─────────── */

export default function WebApp() {
  console.log('[open-web-codex] build:', '2026-07-12T21:20:00Z');
  const [baseUrl, setBaseUrl] = useState(
    localStorage.getItem("codexMonitorWebBaseUrl") ?? "http://127.0.0.1:4733",
  );
  const [token, setToken] = useState(sessionStorage.getItem("codexMonitorWebToken") ?? "");
  const [workspaces, setWorkspaces] = useState<WorkspaceInfo[]>([]);
  const [activeWorkspaceId, setActiveWorkspaceId] = useState<string | null>(null);
  const [activeThreadId, setActiveThreadId] = useState<string | null>(null);
  const [threadsByWorkspace, setThreadsByWorkspace] = useState<Record<string, ThreadInfo[]>>({});
  const [threadLoading, setThreadLoading] = useState(false);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [stopping, setStopping] = useState(false);
  const [queuedFollowUps, setQueuedFollowUps] = useState<QueuedFollowUp[]>([]);
  const [steeringFollowUpId, setSteeringFollowUpId] = useState<string | null>(null);
  const [userInputRequests, setUserInputRequests] = useState<RequestUserInputRequest[]>([]);
  const [submittingUserInputId, setSubmittingUserInputId] = useState<number | string | null>(null);
  const [messages, setMessages] = useState<LogEntry[]>([]);
  const [gatewayState, setGatewayState] = useState<GatewayState>("checking");
  const [gatewayVersion, setGatewayVersion] = useState<string | null>(null);
  const [thinking, setThinking] = useState(false);
  const [turnStartedAt, setTurnStartedAt] = useState<number | null>(null);
  const [tokenUsage, setTokenUsage] = useState<ThreadTokenUsage | null>(null);
  const [threadStatus, setThreadStatus] = useState<string>("idle");
  const [activeTurnId, setActiveTurnId] = useState<string | null>(null);
  const [threadSettings, setThreadSettings] = useState<Record<string, unknown> | null>(null);
  const [rateLimits, setRateLimits] = useState<Record<string, unknown> | null>(null);
  const [goal, setGoal] = useState<GoalInfo | null>(null);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(() =>
    typeof window !== "undefined" && window.matchMedia("(max-width: 760px)").matches,
  );
  const [filePanelOpen, setFilePanelOpen] = useState(false);
  const [filePanelWidth, setFilePanelWidth] = useState(() => {
    if (typeof window === "undefined") return 360;
    const stored = Number(window.localStorage.getItem("open-web-codex:file-panel-width:v1"));
    return Number.isFinite(stored) && stored >= 260 && stored <= 720 ? stored : 360;
  });
  const [selectedFilePath, setSelectedFilePath] = useState<string | null>(null);
  const [mcpServers, setMcpServers] = useState<Record<string, {name: string; status: string; error?: string | null; failureReason?: string | null}>>({});
  const [modelProviders, setModelProviders] = useState<ModelProviderSummary[]>([]);
  const [currentProviderId, setCurrentProviderId] = useState<string | null>(null);
  const [providerModels, setProviderModels] = useState<ModelSummary[]>([]);
  const [selectedProviderModelId, setSelectedProviderModelId] = useState<string | null>(null);
  const [catalogLoading, setCatalogLoading] = useState(false);
  const [catalogError, setCatalogError] = useState<string | null>(null);
  const activeThreadModelSelectionRef = useRef<{
    threadId: string;
    providerId: string;
    modelId: string;
  } | null>(null);
  const [theme, setTheme] = useState<"light" | "dark">(() =>
    localStorage.getItem("open-web-codex:theme") === "light" ? "light" : "dark",
  );

  useEffect(() => {
    localStorage.setItem("open-web-codex:theme", theme);
  }, [theme]);

  useEffect(() => {
    const narrowScreen = window.matchMedia("(max-width: 760px)");
    const syncSidebarWithViewport = (event: MediaQueryListEvent) => {
      setSidebarCollapsed(event.matches);
    };
    narrowScreen.addEventListener("change", syncSidebarWithViewport);
    return () => narrowScreen.removeEventListener("change", syncSidebarWithViewport);
  }, []);

  useEffect(() => {
    setSelectedFilePath(null);
  }, [activeWorkspaceId]);

  useEffect(() => {
    setQueuedFollowUps([]);
    setSteeringFollowUpId(null);
  }, [activeThreadId]);

  useEffect(() => {
    window.localStorage.setItem("open-web-codex:file-panel-width:v1", String(filePanelWidth));
  }, [filePanelWidth]);

  const client = useMemo(() => new CodexMonitorWebClient({ baseUrl, token }), [baseUrl, token]);

  const refreshModelCatalog = useCallback(async () => {
    if (!activeWorkspaceId) return;
    setCatalogLoading(true);
    setCatalogError(null);
    try {
      const providerResponse = await client.listModelProviders(activeWorkspaceId);
      const catalog = parseModelProviderCatalog(providerResponse);
      const threadSelection = activeThreadModelSelectionRef.current;
      const selectedProviderId = threadSelection?.threadId === activeThreadIdRef.current
        ? threadSelection.providerId
        : catalog.currentProviderId;
      const selectedModelId = threadSelection?.threadId === activeThreadIdRef.current
        ? threadSelection.modelId
        : catalog.currentModelId;
      const selectedProvider = catalog.providers.find((provider) => provider.id === selectedProviderId);
      const nextModels = modelSummariesForProvider(selectedProvider);
      setModelProviders(catalog.providers.map((provider) => ({
        ...provider,
        isCurrent: provider.id === selectedProviderId,
      })));
      setCurrentProviderId(selectedProviderId);
      setProviderModels(nextModels);
      setSelectedProviderModelId(
        selectedModelId && nextModels.some((model) => model.id === selectedModelId)
          ? selectedModelId
          : nextModels[0]?.id ?? null,
      );
    } catch (error) {
      setCatalogError(error instanceof Error ? error.message : String(error));
    } finally {
      setCatalogLoading(false);
    }
  }, [activeWorkspaceId, client]);

  const persistThreadModelSelection = useCallback(async (
    workspaceId: string,
    threadId: string | null,
    providerId: string,
    modelId: string,
  ) => {
    if (!threadId || threadId.startsWith("pending-thread:")) return;
    await client.updateThreadModelSelection(workspaceId, threadId, providerId, modelId);
    setThreadsByWorkspace((previous) => ({
      ...previous,
      [workspaceId]: (previous[workspaceId] ?? []).map((thread) =>
        thread.id === threadId
          ? { ...thread, modelProvider: providerId, model: modelId }
          : thread),
    }));
    if (activeThreadIdRef.current === threadId) {
      activeThreadModelSelectionRef.current = { threadId, providerId, modelId };
    }
  }, [client]);

  const selectProviderAndDefaultModel = useCallback(async (providerId: string) => {
    const workspaceId = activeWorkspaceId;
    if (!workspaceId) return;
    const previousProviderId = currentProviderId;
    let switched = false;
    setCatalogLoading(true);
    setCatalogError(null);
    try {
      await client.writeModelProvider(workspaceId, { action: "select", id: providerId });
      switched = true;
      const modelResponse = await client.listModels(workspaceId);
      const nextModels = parseModelListResponse(modelResponse);
      const nextModel = nextModels.find((model) => model.isDefault) ?? nextModels[0];
      if (!nextModel) {
        throw new Error("This Provider has no selectable models");
      }
      await client.selectProviderModel(workspaceId, providerId, nextModel.model);
      await persistThreadModelSelection(
        workspaceId,
        activeThreadId,
        providerId,
        nextModel.model,
      );
      setCurrentProviderId(providerId);
      setProviderModels(nextModels);
      setSelectedProviderModelId(nextModel.id);
      await refreshModelCatalog();
    } catch (error) {
      if (switched && previousProviderId && previousProviderId !== providerId) {
        try {
          await client.writeModelProvider(workspaceId, {
            action: "select",
            id: previousProviderId,
          });
          await refreshModelCatalog();
        } catch {
          // Keep the original selection error; Refresh remains available.
        }
      }
      setCatalogError(error instanceof Error ? error.message : String(error));
    } finally {
      setCatalogLoading(false);
    }
  }, [activeThreadId, activeWorkspaceId, client, currentProviderId, persistThreadModelSelection, refreshModelCatalog]);

  const selectThreadModel = useCallback(async (modelId: string) => {
    const workspaceId = activeWorkspaceId;
    const providerId = currentProviderId;
    if (!workspaceId || !providerId) return;
    const selected = providerModels.find((model) => model.id === modelId);
    const model = selected?.model ?? modelId;
    setSelectedProviderModelId(modelId);
    setCatalogLoading(true);
    setCatalogError(null);
    try {
      await client.selectProviderModel(workspaceId, providerId, model);
      await persistThreadModelSelection(
        workspaceId,
        activeThreadId,
        providerId,
        model,
      );
    } catch (error) {
      setCatalogError(error instanceof Error ? error.message : String(error));
      await refreshModelCatalog();
    } finally {
      setCatalogLoading(false);
    }
  }, [activeThreadId, activeWorkspaceId, client, currentProviderId, persistThreadModelSelection, providerModels, refreshModelCatalog]);

  useEffect(() => {
    setModelProviders([]);
    setProviderModels([]);
    setSelectedProviderModelId(null);
    setCurrentProviderId(null);
    setCatalogError(null);
    activeThreadModelSelectionRef.current = null;
    if (activeWorkspaceId) void refreshModelCatalog();
  }, [activeWorkspaceId, refreshModelCatalog]);

  useEffect(() => {
    setMcpServers({});
    if (!activeWorkspaceId || !activeThreadId) return;

    let cancelled = false;
    void client.listMcpServerStatus(activeWorkspaceId, activeThreadId).then((value) => {
      if (cancelled) return;
      const snapshot = parseInitialMcpServers(value);
      setMcpServers((current) => ({ ...snapshot, ...current }));
    }).catch(() => {
      // Runtime startup notifications continue to update this projection.
    });

    return () => {
      cancelled = true;
    };
  }, [activeThreadId, activeWorkspaceId, client]);

  useEffect(() => {
    setRateLimits(null);
    if (!activeWorkspaceId) return;

    let cancelled = false;
    void client.getAccountRateLimits(activeWorkspaceId).then((value) => {
      if (cancelled) return;
      const snapshot = parseInitialRateLimits(value);
      setRateLimits((current) => mergeRateLimits(snapshot, current));
    }).catch(() => {
      // Rate limits are supplementary Profile state and never gate a Thread.
    });

    return () => {
      cancelled = true;
    };
  }, [activeWorkspaceId, client]);

  const activeWorkspace = workspaces.find((w) => w.id === activeWorkspaceId) ?? null;
  const listWorkspaceFiles = useCallback((workspaceId: string) => client.listWorkspaceFiles(workspaceId, activeThreadId), [activeThreadId, client]);
  const readWorkspaceFile = useCallback((workspaceId: string, path: string) => client.readWorkspaceFile(workspaceId, path, activeThreadId), [activeThreadId, client]);
  const loadWorkspaceGitStatus = useCallback((workspaceId: string) => client.getGitStatus(workspaceId, activeThreadId), [activeThreadId, client]);
  const openFile = useCallback((path: string) => {
    const workspacePath = activeWorkspace?.path?.replace(/\/$/, "");
    const normalized = workspacePath && path.startsWith(`${workspacePath}/`) ? path.slice(workspacePath.length + 1) : path.replace(/^\//, "");
    setSelectedFilePath(normalized);
    setFilePanelOpen(true);
  }, [activeWorkspace?.path]);

  // Streaming accumulators
  const streamingTexts = useRef<Map<string, string>>(new Map());
  const agentMessagePhases = useRef<Map<string, AgentMessagePhase>>(new Map());
  const reasoningSummaries = useRef<Map<string, string>>(new Map());
  const streamingLogIds = useRef<Map<string, string>>(new Map());
  const turnDiffLogIds = useRef<Map<string, string>>(new Map());
  const commandLogIds = useRef<Map<string, string>>(new Map());
  const toolLogIds = useRef<Map<string, string>>(new Map());
  const commandOutputs = useRef<Map<string, string>>(new Map());
  const commandStartedAt = useRef<Map<string, number>>(new Map());
  const interruptRequestTurnId = useRef<string | null>(null);
  const queueDispatching = useRef(false);
  const threadHydrationSequence = useRef(0);
  const recentAppServerEvents = useRef<Map<string, number>>(new Map());
  const pendingApprovalsByThread = useRef<Map<string, LogEntry[]>>(new Map());
  const threadTranscriptCache = useRef<Map<string, ThreadTranscriptCacheEntry>>(new Map());
  const refreshThreadsRef = useRef<((workspaceId?: string) => Promise<void>) | null>(null);
  const activeThreadIdRef = useRef(activeThreadId);
  activeThreadIdRef.current = activeThreadId;
  const activeWorkspaceIdRef = useRef(activeWorkspaceId);
  activeWorkspaceIdRef.current = activeWorkspaceId;

  useEffect(() => {
    // Workspace selection is intentionally workspace-first. A Thread becomes
    // active only after the user selects or creates one, so no transcript or
    // per-Thread runtime state may survive a workspace change.
    activeThreadIdRef.current = null;
    threadHydrationSequence.current += 1;
    threadTranscriptCache.current.clear();
    setActiveThreadId(null);
    setThreadLoading(false);
    setMessages([]);
    setQueuedFollowUps([]);
    setSteeringFollowUpId(null);
    setUserInputRequests([]);
    setTokenUsage(null);
    setGoal(null);
    setThinking(false);
    setTurnStartedAt(null);
    setThreadStatus("idle");
    setActiveTurnId(null);
    setStopping(false);
    interruptRequestTurnId.current = null;
  }, [activeWorkspaceId]);

  useEffect(() => {
    if (!activeWorkspaceId || !activeThreadId || threadLoading) return;
    const selected = (threadsByWorkspace[activeWorkspaceId] ?? [])
      .find((thread) => thread.id === activeThreadId);
    threadTranscriptCache.current.set(activeThreadId, {
      messages,
      updatedAt: selected?.updatedAt ?? 0,
      status: threadStatus,
      thinking,
      activeTurnId,
      turnStartedAt,
    });
  }, [
    activeThreadId,
    activeTurnId,
    activeWorkspaceId,
    messages,
    thinking,
    threadLoading,
    threadStatus,
    threadsByWorkspace,
    turnStartedAt,
  ]);

  const appendLog = useCallback(
    (
      level: LogEntry["level"],
      text: string,
      extra?: Partial<Omit<LogEntry, "id" | "level" | "text">>,
      id?: string,
    ) => {
      setMessages((prev) => appendWebLogEntry(prev, {
        level,
        text,
        ...extra,
        id: id || newLogId(),
      }));
    },
    [],
  );

  const handleAppEvent = useCallback(
    (event: AppServerEvent): WebEventLogEntry | null => {
      const message = event.message ?? {};
      const method = typeof message.method === "string" ? message.method : null;
      if (!method) return null;

      const params =
        message.params && typeof message.params === "object"
          ? (message.params as Record<string, unknown>)
          : {};
      const eventTurn = params.turn && typeof params.turn === "object"
        ? params.turn as Record<string, unknown>
        : null;
      const eventTurnId = typeof params.turnId === "string"
        ? params.turnId
        : typeof params.turn_id === "string"
          ? params.turn_id
          : typeof eventTurn?.id === "string"
            ? eventTurn.id
            : null;
      const belongsToInterruptedTurn = Boolean(
        eventTurnId
        && eventTurnId === interruptRequestTurnId.current,
      );

      const eventThreadId = getAppServerThreadId(event);
      if (eventThreadId && event.workspace_id) {
        if (method === "thread/name/updated") {
          const rawName = params.threadName ?? params.thread_name;
          const label = normalizeThreadName(rawName) ?? "Thread";
          setThreadsByWorkspace((previous) => ({
            ...previous,
            [event.workspace_id]: (previous[event.workspace_id] ?? []).map((thread) =>
              thread.id === eventThreadId ? { ...thread, label, optimistic: false } : thread),
          }));
        }
        const nextStatus = method === "turn/started"
          ? "running"
          : method === "turn/completed" || method === "thread/closed"
            ? "idle"
            : null;
        if (nextStatus) {
          setThreadsByWorkspace((previous) => ({
            ...previous,
            [event.workspace_id]: (previous[event.workspace_id] ?? []).map((thread) =>
              thread.id === eventThreadId ? { ...thread, status: nextStatus } : thread),
          }));
        }
      }
      const belongsToBackgroundThread = Boolean(
        eventThreadId
        && eventThreadId !== activeThreadIdRef.current,
      );
      if (belongsToBackgroundThread && eventThreadId) {
        threadTranscriptCache.current.delete(eventThreadId);
      }
      if (
        belongsToBackgroundThread
        && method !== "item/commandExecution/requestApproval"
        && method !== "serverRequest/resolved"
      ) {
        return null;
      }
      if (
        belongsToInterruptedTurn
        && method !== "turn/completed"
        && method !== "thread/status/changed"
        && method !== "serverRequest/resolved"
      ) {
        return null;
      }

      const itemId = typeof params.itemId === "string" ? params.itemId : null;
      const delta = typeof params.delta === "string" ? params.delta : "";

      if (isWebAppServerRecoveryEvent(method)) {
        setMessages((previous) => previous.some((entry) => entry.kind === "connection")
          ? previous.filter((entry) => entry.kind !== "connection")
          : previous);
        setThreadStatus((previous) => previous === "reconnecting" ? "running" : previous);
      }

      switch (method) {
        case "codex/stderr": {
          const parsed = parseCodexStderr(params);
          if (!parsed) return null;
          setThinking(true);
          setThreadStatus("reconnecting");
          setMessages((previous) => {
            const existing = previous.findIndex((entry) => entry.kind === "connection");
            if (existing < 0) {
              return [...previous.slice(-199), {
                id: newLogId(),
                level: "info" as const,
                text: parsed.text,
                kind: "connection" as const,
                streaming: true,
              }];
            }
            return previous.map((entry, index) => index === existing
              ? { ...entry, text: parsed.text, streaming: true }
              : entry);
          });
          return null;
        }

        case "turn/started": {
          const turn = params.turn && typeof params.turn === "object"
            ? params.turn as Record<string, unknown>
            : null;
          const startedTurnId = turn?.id ?? params.turnId ?? params.turn_id;
          if (
            typeof startedTurnId === "string"
            && startedTurnId === interruptRequestTurnId.current
          ) {
            return null;
          }
          interruptRequestTurnId.current = null;
          setThinking(true);
          setTurnStartedAt(() => {
            const raw = turn?.startedAt ?? params.startedAt ?? params.started_at;
            return typeof raw === "number" && Number.isFinite(raw)
              ? raw < 10_000_000_000 ? raw * 1000 : raw
              : Date.now();
          });
          setThreadStatus("running");
          setActiveTurnId(
            typeof startedTurnId === "string" && startedTurnId ? startedTurnId : null,
          );
          return null;
        }

        case "turn/completed":
          // Keep the richer live transcript. The durable thread projection can omit
          // reasoning and tool items, so replacing the transcript here made every
          // execution step disappear as soon as the final answer arrived.
          setThinking(false);
          setTurnStartedAt(null);
          setThreadStatus("idle");
          setActiveTurnId(null);
          setStopping(false);
          void refreshThreadsRef.current?.(event.workspace_id);
          setMessages((previous) => previous
            .filter((entry) => entry.kind !== "connection"
              && !(entry.level === "system" && entry.text === "Thinking...")
              && !(entry.kind === "reasoning"
                && /^(reasoning completed|reasoning in progress|reasoning)$/i.test(entry.text.trim())
                && !entry.reasoningSummary?.trim()))
            .map((entry) => entry.streaming
              ? {
                  ...entry,
                  streaming: false,
                  text: entry.text,
                }
              : entry));
          return null;

        case "error": {
          const parsed = parseWebAppServerError(params);
          if (!parsed.recoverable) {
            setThinking(false);
            setTurnStartedAt(null);
            setActiveTurnId(null);
            setStopping(false);
            return { level: "error" as const, text: parsed.text };
          }
          setThinking(true);
          setThreadStatus("reconnecting");
          setMessages((previous) => {
            const existing = previous.findIndex((entry) => entry.kind === "connection");
            if (existing < 0) {
              return [...previous.slice(-199), {
                id: newLogId(),
                level: "info" as const,
                text: parsed.text,
                kind: "connection" as const,
                streaming: true,
              }];
            }
            return previous.map((entry, index) => index === existing ? { ...entry, text: parsed.text, streaming: true } : entry);
          });
          return null;
        }

        case "thread/started": {
          const thread = params.thread as Record<string, unknown> | undefined;
          const threadName = typeof thread?.name === "string" && thread.name ? thread.name : null;
          const cliVersion = typeof thread?.cliVersion === "string" ? thread.cliVersion : null;
          return {
            level: "info",
            text: `Thread started${threadName ? `: ${threadName}` : ""}${cliVersion ? ` (${cliVersion})` : ""}`,
          };
        }

        case "mcpServer/startupStatus/updated": {
          const serverName = typeof params.name === "string" ? params.name : "";
          const status = typeof params.status === "string" ? params.status : "";
          if (!serverName) return null;
          setMcpServers(prev => ({
            ...prev,
            [serverName]: {
              name: serverName,
              status,
              error: params.error != null ? String(params.error) : null,
              failureReason: typeof params.failureReason === "string" ? params.failureReason : null,
            },
          }));
          if (status === "error") {
            const msg = typeof params.failureReason === "string" && params.failureReason
              ? `: ${params.failureReason}` : "";
            return { level: "error" as const, text: `MCP ${serverName} error${msg}` };
          }
          return null;
        }

        case "item/reasoning/summaryTextDelta":
        case "item/reasoning/textDelta": {
          if (!itemId || !delta) return null;
          const key = `reason_${itemId}`;
          const isSummary = method === "item/reasoning/summaryTextDelta";
          const target = isSummary ? reasoningSummaries.current : streamingTexts.current;
          const current = target.get(key) ?? "";
          const updated = current + delta;
          target.set(key, updated);

          const existingLogId = streamingLogIds.current.get(key);
          if (existingLogId) {
            setMessages((prev) =>
              prev.map((e) => (e.id === existingLogId
                ? {
                    ...e,
                    text: isSummary ? e.text : updated,
                    reasoningSummary: isSummary ? updated : e.reasoningSummary,
                    streaming: true,
                  }
                : e)),
            );
            return null;
          }
          const id = itemId;
          streamingLogIds.current.set(key, id);
          setMessages((prev) => [
            ...prev.slice(-199),
            {
              id,
              level: "system",
              text: isSummary ? "Reasoning in progress" : updated,
              reasoningSummary: isSummary ? updated : undefined,
              kind: "reasoning",
              streaming: true,
            },
          ]);
          return null;
        }

        case "item/agentMessage/delta": {
          if (!itemId) return null;
          if (!delta) return null;
          if (streamingTexts.current.has(`reason_${itemId}`)) return null;

          const current = streamingTexts.current.get(itemId) ?? "";
          const updated = current + stripLeadingProviderSentinel(delta);
          streamingTexts.current.set(itemId, updated);
          const streamingMessagePhase = agentMessagePhases.current.get(itemId);

          const existingLogId = streamingLogIds.current.get(itemId);
          if (existingLogId) {
            setMessages((prev) =>
              prev.map((e) => (e.id === existingLogId ? {
                ...e,
                text: updated,
                messagePhase: e.messagePhase ?? streamingMessagePhase,
                streaming: true,
              } : e)),
            );
            return null;
          }
          const id = itemId;
          streamingLogIds.current.set(itemId, id);
          setMessages((prev) => [
            ...prev.slice(-199),
            {
              id,
              level: "assistant",
              text: updated,
              messagePhase: streamingMessagePhase,
              streaming: true,
            },
          ]);
          return null;
        }

        case "item/completed": {
          const item = params.item as Record<string, unknown> | undefined;
          if (!item) return null;
          const role = typeof item.role === "string" ? item.role : null;
          const kind = typeof item.kind === "string" ? item.kind : null;
          const itemIdFromItem = typeof item?.id === "string" ? item.id : null;
          const itemType2 = typeof item.type === "string" ? item.type : null;
          const completedItemId = itemId ?? itemIdFromItem;
          const completedMessagePhase = agentMessagePhase(item.phase)
            ?? (completedItemId ? agentMessagePhases.current.get(completedItemId) : undefined);
          const completedInlineArtifacts = Array.isArray(item.inlineArtifacts)
            ? item.inlineArtifacts.flatMap((value) => {
                const artifact = parseInlineVisualizationArtifact(value);
                return artifact ? [artifact] : [];
              })
            : [];
          const completedReasoningSummary = (Array.isArray(item.summary) && item.summary.length > 0)
            ? item.summary.map((part) => String(part)).join("\n\n").trim()
            : typeof item.summary === "string" ? item.summary.trim() : "";
          const completedReasoningContent = Array.isArray(item.content)
            ? item.content.map((part) => String(part)).join("\n\n").trim()
            : typeof item.content === "string" ? item.content.trim() : "";

          if (role === "user" || isUserThreadItem(item)) return null;

          if (role === "assistant" || itemType2 === "agentMessage") {
            if (completedItemId && streamingTexts.current.has(completedItemId)) {
              const acc = streamingTexts.current.get(completedItemId)!;
              const eid = streamingLogIds.current.get(completedItemId);
              if (eid) {
                setMessages((prev) => acc.trim()
                  ? prev.map((e) => (e.id === eid ? {
                      ...e,
                      text: acc,
                      messagePhase: completedMessagePhase ?? e.messagePhase,
                      inlineArtifacts: completedInlineArtifacts.length
                        ? completedInlineArtifacts
                        : e.inlineArtifacts,
                      streaming: false,
                    } : e))
                  : prev.filter((e) => e.id !== eid));
              }
              streamingTexts.current.delete(completedItemId);
              streamingLogIds.current.delete(completedItemId);
              agentMessagePhases.current.delete(completedItemId);
              return null;
            }
            if (completedItemId) agentMessagePhases.current.delete(completedItemId);
            const completedText = stripLeadingProviderSentinel(
              typeof item.text === "string" ? item.text : "",
            );
            if (!completedText.trim()) return null;
            return {
              id: completedItemId || undefined,
              level: "assistant",
              text: completedText,
              messagePhase: completedMessagePhase,
              inlineArtifacts: completedInlineArtifacts.length
                ? completedInlineArtifacts
                : undefined,
            };
          }

          if (kind === "tool") {
            const toolType = typeof item.toolType === "string" ? item.toolType : "";
            const title = typeof item.title === "string" ? item.title : "";
            const status = typeof item.status === "string" ? item.status : "";
            const filePath = typeof item.filePath === "string" ? item.filePath : undefined;
            return {
              id: completedItemId || undefined,
              level: "info" as const,
              text: `${toolType}: ${title}`,
              kind: "tool" as const,
              toolType,
              toolTitle: title,
              toolStatus: status,
              filePath,
            };
          }

          if (kind === "reasoning") {
            const key = itemId ? `reason_${itemId}` : (itemIdFromItem ? `reason_${itemIdFromItem}` : null);
            if (key && streamingLogIds.current.has(key)) {
              const acc = streamingTexts.current.get(key) ?? "";
              const eid = streamingLogIds.current.get(key);
              if (eid) {
                const text = acc.trim() || completedReasoningContent || completedReasoningSummary;
                const summary = completedReasoningSummary || reasoningSummaries.current.get(key);
                setMessages((prev) => text || summary
                  ? prev.map((e) => (e.id === eid ? {
                      ...e,
                      text: text || summary || "",
                      reasoningSummary: summary,
                      streaming: false,
                    } : e))
                  : prev.filter((e) => e.id !== eid));
              }
              streamingTexts.current.delete(key);
              reasoningSummaries.current.delete(key);
              streamingLogIds.current.delete(key);
              if (eid) return null;
            }
            if (!completedReasoningContent && !completedReasoningSummary) return null;
            return {
              id: completedItemId || undefined,
              level: "system" as const,
              text: completedReasoningContent || completedReasoningSummary,
              reasoningSummary: completedReasoningSummary || undefined,
              kind: "reasoning" as const,
              streaming: false,
            };
          }

          if (kind === "diff") {
            const title = typeof item.title === "string" ? item.title : "";
            const diff = typeof item.diff === "string" ? item.diff : "";
            const lines = diff
              .split("\n")
              .filter(Boolean)
              .map((l: string) => {
                if (l.startsWith("+")) return { type: "add" as const, text: l.slice(1) };
                if (l.startsWith("-")) return { type: "del" as const, text: l.slice(1) };
                return { type: "ctx" as const, text: l };
              });
            return {
              id: completedItemId || undefined,
              level: "info" as const,
              text: title,
              kind: "diff" as const,
              diffTitle: title,
              diffLines: lines.slice(0, 100),
            };
          }


          if (itemType2 === "reasoning") {
            const key = itemId ? `reason_${itemId}` : (itemIdFromItem ? `reason_${itemIdFromItem}` : null);
            if (key && streamingLogIds.current.has(key)) {
              const acc = streamingTexts.current.get(key) ?? "";
              const eid = streamingLogIds.current.get(key);
              if (eid) {
                const text = acc.trim() || completedReasoningContent || completedReasoningSummary;
                const summary = completedReasoningSummary || reasoningSummaries.current.get(key);
                setMessages((prev) => text || summary
                  ? prev.map((e) => (e.id === eid ? {
                      ...e,
                      text: text || summary || "",
                      reasoningSummary: summary,
                      streaming: false,
                    } : e))
                  : prev.filter((e) => e.id !== eid));
              }
              streamingTexts.current.delete(key);
              reasoningSummaries.current.delete(key);
              streamingLogIds.current.delete(key);
              if (eid) return null;
            }
            if (!completedReasoningContent && !completedReasoningSummary) return null;
            return {
              id: completedItemId || undefined,
              level: "system" as const,
              text: completedReasoningContent || completedReasoningSummary,
              reasoningSummary: completedReasoningSummary || undefined,
              kind: "reasoning" as const,
              streaming: false,
            };
          }

          if (itemType2 === "commandExecution") {
            const cmd = commandText(item.command);
            const liveOutput = completedItemId ? commandOutputs.current.get(completedItemId) ?? "" : "";
            const output = typeof item.aggregatedOutput === "string" && item.aggregatedOutput
              ? item.aggregatedOutput
              : liveOutput;
            const exitCode = typeof item.exitCode === "number" ? item.exitCode : undefined;
            const startedAt = completedItemId ? commandStartedAt.current.get(completedItemId) : undefined;
            const durationMs = typeof item.durationMs === "number" && item.durationMs > 0
              ? item.durationMs
              : startedAt ? Date.now() - startedAt : undefined;
            const cwd = typeof item.cwd === "string" ? item.cwd : undefined;
            const cmdActions = Array.isArray(item.commandActions)
              ? (item.commandActions as Record<string,unknown>[]).map(a => ({type: String(a.type ?? ""), path: String(a.path ?? "")}))
              : [];
            const existingId = completedItemId ? commandLogIds.current.get(completedItemId) : undefined;
            if (existingId) {
              setMessages((previous) => previous.map((entry) => entry.id === existingId
                ? {
                    ...entry,
                    text: cmd || entry.text,
                    cmdOutput: output,
                    toolStatus: typeof item.status === "string" ? item.status : "completed",
                    cmdExitCode: exitCode,
                    cmdDurationMs: durationMs,
                    cmdCwd: cwd,
                    cmdActions,
                    streaming: false,
                  }
                : entry));
              if (completedItemId) {
                commandLogIds.current.delete(completedItemId);
                commandOutputs.current.delete(completedItemId);
                commandStartedAt.current.delete(completedItemId);
              }
              return null;
            }
            return {
              id: completedItemId || undefined,
              level: "info" as const,
              text: cmd,
              kind: "command_exec" as const,
              cmdOutput: output,
              toolStatus: typeof item.status === "string" ? item.status : undefined,
              cmdExitCode: exitCode,
              cmdDurationMs: durationMs,
              cmdCwd: cwd,
              cmdActions,
            };
          }
          const entry = webLogEntryFromThreadItem(item, newLogId);
          const existingId = completedItemId ? toolLogIds.current.get(completedItemId) : undefined;
          if (entry && existingId) {
            setMessages((previous) => previous.map((candidate) => candidate.id === existingId
              ? { ...entry, id: existingId, streaming: false }
              : candidate));
            if (completedItemId) toolLogIds.current.delete(completedItemId);
            return null;
          }
          return entry;
        }

        case "item/started": {
          const item = params.item as Record<string, unknown> | undefined;
          if (item && isUserThreadItem(item)) return null;
          const itemKind = typeof item?.kind === "string" ? item.kind : null;
          const itemType = typeof item?.type === "string" ? item.type : null;
          const startedItemId = typeof item?.id === "string" ? item.id : itemId;
          if (itemType === "commandExecution" && startedItemId) {
            const command = commandText(item?.command);
            const id = startedItemId;
            commandLogIds.current.set(startedItemId, id);
            commandOutputs.current.set(startedItemId, "");
            commandStartedAt.current.set(startedItemId, Date.now());
            setMessages((previous) => [
              ...previous.slice(-199),
              {
                id,
                level: "info",
                text: command || "Command",
                kind: "command_exec",
                approvalId: startedItemId,
                toolStatus: "inProgress",
                cmdCwd: typeof item?.cwd === "string" ? item.cwd : undefined,
                streaming: true,
              },
            ]);
            return null;
          }
          if (itemKind === "tool") {
            return {
              id: startedItemId || undefined,
              level: "info" as const,
              text: "",
              kind: "tool" as const,
              toolType: "",
              toolTitle: "",
              toolStatus: "running",
            };
          }
          if ((itemKind === "reasoning" || itemType === "reasoning") && startedItemId) {
            const key = `reason_${startedItemId}`;
            if (streamingLogIds.current.has(key)) return null;
            const id = startedItemId;
            streamingTexts.current.set(key, "");
            streamingLogIds.current.set(key, id);
            setMessages((previous) => [
              ...previous.slice(-199),
              {
                id,
                level: "system",
                text: "Reasoning in progress",
                kind: "reasoning",
                streaming: true,
              },
            ]);
            return null;
          }
          if (itemType === "agentMessage" && startedItemId) {
            const startedMessagePhase = agentMessagePhase(item?.phase);
            if (startedMessagePhase) {
              agentMessagePhases.current.set(startedItemId, startedMessagePhase);
            }
            const streamedText = streamingTexts.current.get(startedItemId) ?? "";
            const completedText = stripLeadingProviderSentinel(
              typeof item?.text === "string" ? item.text : "",
            );
            const text = streamedText || completedText;
            const existingId = streamingLogIds.current.get(startedItemId);
            if (existingId) {
              setMessages((previous) => previous.map((entry) =>
                entry.id === existingId
                  ? {
                    ...entry,
                    text,
                    messagePhase: startedMessagePhase ?? entry.messagePhase,
                    streaming: true,
                  }
                  : entry));
            } else if (text) {
              const id = startedItemId;
              streamingTexts.current.set(startedItemId, text);
              streamingLogIds.current.set(startedItemId, id);
              setMessages((previous) => [
                ...previous.slice(-199),
                {
                  id,
                  level: "assistant",
                  text,
                  messagePhase: startedMessagePhase,
                  streaming: true,
                },
              ]);
            }
            return null;
          }
          if (itemType) {
            const entry = webLogEntryFromThreadItem(item as Record<string, unknown>, newLogId);
            if (!entry) return null;
            const id = startedItemId || entry.id;
            if (startedItemId) toolLogIds.current.set(startedItemId, id);
            setMessages((previous) => [...previous.slice(-199), { ...entry, id, streaming: true }]);
            return null;
          }
          return null;
        }

        case "item/commandExecution/outputDelta": {
          if (!itemId || !delta) return null;
          const previousOutput = commandOutputs.current.get(itemId) ?? "";
          const output = (previousOutput + delta).slice(-200_000);
          commandOutputs.current.set(itemId, output);
          const logId = commandLogIds.current.get(itemId);
          if (logId) {
            setMessages((previous) => previous.map((entry) => entry.id === logId
              ? {
                  ...entry,
                  cmdOutput: output,
                  toolStatus: "inProgress",
                  streaming: true,
                }
              : entry));
          }
          return null;
        }

        case "item/commandExecution/terminalInteraction": {
          if (!itemId) return null;
          const stdin = typeof params.stdin === "string" ? params.stdin : "";
          // Empty stdin represents a terminal poll. It is protocol activity, not a
          // user-facing message, so keep the command running without adding output.
          if (!stdin) return null;
          const output = appendTerminalInteractionOutput(commandOutputs.current.get(itemId) ?? "", stdin);
          commandOutputs.current.set(itemId, output);
          const logId = commandLogIds.current.get(itemId);
          if (logId) {
            setMessages((previous) => previous.map((entry) => entry.id === logId
              ? {
                  ...entry,
                  cmdOutput: output,
                  toolStatus: "inProgress",
                  streaming: true,
                }
              : entry));
          }
          return null;
        }

        case "turn/error": {
          const msg = typeof params.message === "string" ? params.message : "Unknown error";
          setThinking(false);
          setTurnStartedAt(null);
          setActiveTurnId(null);
          setStopping(false);
          return { level: "error" as const, text: `Turn error: ${msg}` };
        }

        case "thread/status/changed": {
          const status = params.status as Record<string, unknown> | undefined;
          const type = typeof status?.type === "string" ? status.type : "unknown";
          const activeFlags = Array.isArray(status?.activeFlags) ? (status as Record<string, unknown>).activeFlags as string[] : [];
          const flagsStr = activeFlags.length > 0 ? ":" + activeFlags.join(",") : "";
          const statusTurnId = typeof params.turnId === "string"
            ? params.turnId
            : typeof params.turn_id === "string" ? params.turn_id : null;
          if (type === "active" && statusTurnId === interruptRequestTurnId.current) {
            return null;
          }
          setThreadStatus(type + flagsStr);
          if (type === "active") {
            setThinking(true);
            setTurnStartedAt((previous) => previous ?? Date.now());
          }
          if (type === "idle") {
            setThinking(false);
            setTurnStartedAt(null);
            setActiveTurnId(null);
            setStopping(false);
          }
          if (type === "error") {
            setThinking(false);
            setTurnStartedAt(null);
            setActiveTurnId(null);
            setStopping(false);
            return { level: "error" as const, text: "Thread error" };
          }
          if (type === "systemError") {
            setThinking(false);
            setTurnStartedAt(null);
            setActiveTurnId(null);
            setStopping(false);
            return {
              level: "error" as const,
              text: "System error. The runtime did not provide any additional error details.",
            };
          }
          return null;
        }

        case "turn/plan/updated": {
          const turnId = String(params.turnId ?? params.turn_id ?? "");
          const plan = normalizePlanUpdate(turnId, params.explanation, params.plan ?? params.steps);
          if (plan) setGoal((previous) => previous ? { ...previous, steps: plan.steps } : previous);
          return null;
        }

        case "item/plan/ready":
          return { level: "info" as const, text: "Plan ready" };

        case "turn/diff/updated":
        {
          const turnId = typeof params.turnId === "string"
            ? params.turnId
            : typeof params.turn_id === "string" ? params.turn_id : null;
          const diff = typeof params.diff === "string" ? params.diff : "";
          if (!turnId || !diff) return null;

          const parsed = parseWebTurnDiff(diff);
          setGoal((previous) => previous ? {
            ...previous,
            fileCount: parsed.fileCount,
            additions: parsed.additions,
            deletions: parsed.deletions,
          } : previous);
          const existingId = turnDiffLogIds.current.get(turnId);
          if (existingId) {
            setMessages((previous) => previous.map((entry) => entry.id === existingId
              ? {
                  ...entry,
                  text: parsed.title,
                  diffTitle: parsed.title,
                  diffLines: parsed.lines.slice(0, 400),
                  streaming: true,
                }
              : entry));
            return null;
          }

          const id = newLogId();
          turnDiffLogIds.current.set(turnId, id);
          setMessages((previous) => [
            ...previous.slice(-199),
            {
              id,
              level: "info",
              text: parsed.title,
              kind: "diff",
              diffTitle: parsed.title,
              diffLines: parsed.lines.slice(0, 400),
              streaming: true,
            },
          ]);
          return null;
        }

        case "thread/tokenUsage/updated": {
          const raw = params.tokenUsage as Record<string, unknown> | undefined;
          if (raw) setTokenUsage(normalizeTokenUsage(raw));
          return null;
        }

        case "thread/settings/updated": {
          const s = params as Record<string, unknown>;
          setThreadSettings(s);
          return null;
        }

        case "item/reasoning/summaryPartAdded": {
          const partItemId = typeof params.itemId === "string" ? params.itemId : null;
          if (partItemId) {
            const key = `reason_${partItemId}`;
            // Keep streaming entry alive — just add a paragraph break between parts
            const current = streamingTexts.current.get(key) ?? "";
            if (current && !current.endsWith("\n\n")) {
              streamingTexts.current.set(key, current + "\n\n");
            }
          }
          return null;
        }

        case "thread/goal/updated": {
          const g = params.goal as Record<string, unknown> | undefined;
          if (g) {
            setGoal((previous) => ({
              objective: typeof g.objective === "string" ? g.objective : "",
              status: typeof g.status === "string" ? g.status : "active",
              tokenBudget: typeof g.tokenBudget === "number" ? g.tokenBudget : null,
              tokensUsed: typeof g.tokensUsed === "number" ? g.tokensUsed : 0,
              timeUsedSeconds: typeof g.timeUsedSeconds === "number" ? g.timeUsedSeconds : 0,
              steps: previous?.steps ?? [],
              fileCount: previous?.fileCount,
              additions: previous?.additions,
              deletions: previous?.deletions,
            }));
          }
          return null;
        }

        case "thread/goal/cleared": {
          setGoal(null);
          return null;
        }

        case "account/rateLimits/updated": {
          const raw = params.rateLimits as Record<string, unknown> | undefined;
          if (raw) setRateLimits((current) => mergeRateLimits(current, raw));
          return null;
        }

        case "item/commandExecution/requestApproval": {
          const cmd = commandText(params.command);
          if (!cmd) return null;
          const requestId = message.id;
          const approvalId = typeof params.itemId === "string" ? params.itemId : undefined;
          const approvalMode = typeof params.mode === "string" ? params.mode : undefined;
          const approvalUrl = typeof params.url === "string" ? params.url : undefined;
          const approvalServerName = typeof params.serverName === "string"
            ? params.serverName
            : undefined;
          const approvalTool = typeof params.tool === "string"
            ? params.tool
            : typeof params.toolName === "string"
              ? params.toolName
              : cmd.match(/\btool\s+["“']([^"”']+)["”']/i)?.[1];
          const approvalThreadId = typeof params.threadId === "string"
            ? params.threadId
            : typeof params.thread_id === "string" ? params.thread_id : null;
          if (approvalThreadId && (typeof requestId === "number" || typeof requestId === "string")) {
            const cachedApproval: LogEntry = {
              id: `approval-${event.workspace_id}-${String(requestId)}`,
              level: "info",
              text: cmd,
              kind: "approval",
              approvalId,
              approvalRequestId: requestId,
              approvalStatus: "pending",
              approvalMode,
              approvalUrl,
              approvalServerName,
              approvalTool,
            };
            const current = pendingApprovalsByThread.current.get(approvalThreadId) ?? [];
            pendingApprovalsByThread.current.set(
              approvalThreadId,
              appendWebLogEntry(current, cachedApproval),
            );
          }
          if (belongsToBackgroundThread) return null;
          if (approvalId && (typeof requestId === "number" || typeof requestId === "string")) {
            setMessages((previous) => previous.map((entry) =>
              entry.kind === "command_exec" && entry.approvalId === approvalId
                ? { ...entry, approvalRequestId: requestId, approvalStatus: "pending" }
                : entry));
          }
          return {
            level: "info" as const,
            text: cmd,
            kind: "approval" as const,
            approvalId,
            approvalRequestId:
              typeof requestId === "number" || typeof requestId === "string"
                ? requestId
                : undefined,
            approvalStatus: "pending" as const,
            approvalMode,
            approvalUrl,
            approvalServerName,
            approvalTool,
          };
        }

        case "item/tool/requestUserInput": {
          const requestId = message.id;
          if (typeof requestId !== "number" && typeof requestId !== "string") return null;
          const request = parseWebUserInputRequest(event.workspace_id, requestId, params);
          if (!request) return null;
          setUserInputRequests((previous) => [
            ...previous.filter((candidate) => !(candidate.workspace_id === request.workspace_id && candidate.request_id === request.request_id)),
            request,
          ]);
          return null;
        }

        case "serverRequest/resolved": {
          const requestId = params.requestId ?? params.request_id;
          if (typeof requestId !== "number" && typeof requestId !== "string") return null;
          for (const [threadId, approvals] of pendingApprovalsByThread.current) {
            const remaining = approvals.filter((entry) => entry.approvalRequestId !== requestId);
            if (remaining.length > 0) pendingApprovalsByThread.current.set(threadId, remaining);
            else pendingApprovalsByThread.current.delete(threadId);
          }
          if (belongsToBackgroundThread) return null;
          setMessages((previous) => {
            const approval = previous.find((entry) =>
              entry.kind === "approval" && entry.approvalRequestId === requestId);
            const status: LogEntry["approvalStatus"] = isApprovalOutcome(approval?.approvalStatus)
              ? approval.approvalStatus
              : parseApprovalStatus(params.approvalStatus) ?? "resolved";
            return previous
              .map((entry) => entry.kind === "approval"
                && entry.approvalRequestId === requestId
                ? { ...entry, approvalStatus: status }
                : entry)
              .map((entry) => entry.kind === "command_exec"
                && (entry.approvalRequestId === requestId
                  || (approval?.approvalId && entry.approvalId === approval.approvalId))
                ? { ...entry, approvalRequestId: requestId, approvalStatus: status }
                : entry);
          });
          setUserInputRequests((previous) => previous.filter((request) => request.request_id !== requestId));
          return null;
        }


        default: {
          const text = summarizeWebAppServerEvent(event);
          return text && text !== "{}" ? { level: "event" as const, text } : null;
        }
      }
    },
    [],
  );

  /* ─── Connection ─── */

  const saveConnection = useCallback(() => {
    localStorage.setItem("codexMonitorWebBaseUrl", baseUrl);
    sessionStorage.setItem("codexMonitorWebToken", token);
  }, [baseUrl, token]);

  const checkGateway = useCallback(async () => {
    setGatewayState("checking");
    try {
      const health = await client.health();
      setGatewayState("online");
      setGatewayVersion(health.version);
      saveConnection();
      return true;
    } catch (error) {
      setGatewayState("offline");
      setGatewayVersion(null);
      appendLog("error", error instanceof Error ? error.message : String(error));
      return false;
    }
  }, [appendLog, client, saveConnection]);

  const refreshWorkspaces = useCallback(async () => {
    setBusy(true);
    try {
      const next = await client.listWorkspaces();
      setWorkspaces(next);
      setActiveWorkspaceId((cur) => cur ?? next[0]?.id ?? null);
    } catch (error) {
      appendLog("error", error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  }, [appendLog, client]);

  const refreshThreads = useCallback(async (forWorkspaceId?: string) => {
    const wid = forWorkspaceId ?? activeWorkspaceId;
    if (!wid) return;
    try {
      const raw = await client.listThreads(wid);
      const payload = unwrapWebRpcResult(raw);
      const payloadRecord = payload && typeof payload === "object"
        ? payload as Record<string, unknown>
        : null;
      const allData = payloadRecord?.data ?? payload ?? [];
      // Filter to this workspace by cwd
      const ws = workspaces.find(w => w.id === wid);
      const wsPath = ws?.path ?? '';
      const arr = Array.isArray(allData)
        ? allData.filter((t: Record<string, unknown>) =>
            wsPath && typeof t.cwd === "string" && t.cwd
              ? t.cwd === wsPath || t.cwd.startsWith(`${wsPath}/`) || t.cwd.startsWith(`${wsPath}\\`)
              : true)
        : [];
      if (arr.length > 0 || Array.isArray(arr)) {
        setThreadsByWorkspace(prev => {
          const received = arr.map((t: Record<string, unknown>) => ({
            id: String(t.id ?? ""),
            label: normalizeThreadName(t.name ?? t.label) ?? "Thread",
            updatedAt: parseThreadUpdatedAt(t.updatedAt ?? t.updated_at),
            modelProvider: typeof t.modelProvider === "string"
              ? t.modelProvider
              : typeof t.model_provider === "string" ? t.model_provider : null,
            model: typeof t.model === "string" ? t.model : null,
            turnCount: typeof t.turnCount === "number" ? t.turnCount : undefined,
            status: parseThreadStatus(t.status),
            optimistic: false,
          }));
          const existing = prev[wid] ?? [];
          const merged = received.map((thread) => {
            const optimistic = existing.find((candidate) => candidate.id === thread.id);
            const shouldKeepReturnedName = Boolean(
              optimistic?.optimistic
              && optimistic.label !== "Thread"
              && thread.label === "Thread",
            );
            return shouldKeepReturnedName
              ? { ...thread, label: optimistic?.label ?? thread.label, optimistic: true }
              : thread;
          });
          // Newly started threads are not listed until their first persisted turn.
          // Preserve the optimistic entry until Runtime returns the canonical row.
          // A newly returned Runtime name also wins over a stale placeholder in
          // the first task-list refresh.
          const pending = existing.filter(
            (thread) => thread.optimistic && !received.some((next) => next.id === thread.id),
          );
          return { ...prev, [wid]: [...pending, ...merged] };
        });
      }
    } catch (error) {
      appendLog("error", error instanceof Error ? error.message : String(error));
    }
  }, [activeWorkspaceId, appendLog, client, workspaces]);
  refreshThreadsRef.current = refreshThreads;

  const connectWorkspace = useCallback(async (id: string) => {
    if (!activeWorkspaceId || activeWorkspaceId !== id) {
      setActiveWorkspaceId(id);
    }
    try {
      await client.connectWorkspace(id);
      await refreshWorkspaces();
      // threads will refresh via the activeWorkspaceId effect
    } catch { /* not fatal */ }
  }, [client, refreshWorkspaces]);

 useEffect(() => {
   void checkGateway();
    void refreshWorkspaces();
   const unsub = client.subscribeAppServerEvents(
      (event) => {
        if (!rememberAppServerEvent(recentAppServerEvents.current, event)) return;
        // Accept events for any workspace; caller filters
        const wsId = activeWorkspaceIdRef.current;
        if (wsId && event.workspace_id !== wsId) return;
        const entry = handleAppEvent(event);
        if (entry) {
          const { id, level, text, ...extra } = entry;
          appendLog(level, text, extra, id);
        }
      },
      { onOpen: () => setGatewayState("online"), onError: () => setGatewayState("offline") },
    );
    return unsub;
  }, [appendLog, checkGateway, client]);

  // Auto-refresh threads when workspace changes
  useEffect(() => {
    void refreshThreads();
  }, [activeWorkspaceId, refreshThreads]);

  /* ─── Workspace actions ─── */


  const createWorkspace = useCallback(
    async (name: string) => {
      setBusy(true);
      try {
        await client.createWorkspace(name);
        await refreshWorkspaces();
      } catch (error) {
        appendLog("error", error instanceof Error ? error.message : String(error));
      } finally {
        setBusy(false);
      }
    },
    [appendLog, client, refreshWorkspaces],
  );

  const removeWorkspace = useCallback(async (workspaceId: string) => {
    const workspace = workspaces.find((candidate) => candidate.id === workspaceId);
    if (!workspace) return;
    const confirmed = window.confirm(
      `Remove workspace '${workspace.name}'? The repository folder is kept, but linked worktrees may be removed from disk.`,
    );
    if (!confirmed) return;

    setBusy(true);
    try {
      await client.removeWorkspace(workspaceId);
      setThreadsByWorkspace((previous) => {
        const next = { ...previous };
        delete next[workspaceId];
        return next;
      });
      if (activeWorkspaceId === workspaceId) {
        setActiveWorkspaceId(null);
        activeThreadIdRef.current = null;
        setActiveThreadId(null);
        setMessages([]);
      }
      await refreshWorkspaces();
    } catch (error) {
      appendLog("error", error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  }, [activeWorkspaceId, appendLog, client, refreshWorkspaces, workspaces]);


 const startThread = useCallback(async (
   workspaceId?: string,
   retryTemporaryId?: string,
 ): Promise<string | null> => {
   const wid = workspaceId ?? activeWorkspaceId;
   if (!wid) return null;
   const temporaryId = retryTemporaryId ?? `pending-thread:${newLogId()}`;
   const startedAt = Date.now();
   setActiveWorkspaceId(wid);
   activeThreadIdRef.current = temporaryId;
   setActiveThreadId(temporaryId);
   setThreadLoading(false);
   setMessages([]);
   setTokenUsage(null);
   setGoal(null);
   setThinking(false);
   setThreadStatus("idle");
   setActiveTurnId(null);
   setStopping(false);
   interruptRequestTurnId.current = null;
   setThreadsByWorkspace((previous) => {
     const existing = previous[wid] ?? [];
     const pending: ThreadInfo = {
       id: temporaryId,
       label: "Thread",
       updatedAt: startedAt,
       modelProvider: currentProviderId,
       model: providerModels.find((model) => model.id === selectedProviderModelId)?.model
         ?? selectedProviderModelId,
       status: "creating",
       optimistic: true,
       creationStatus: "creating",
     };
     return {
       ...previous,
       [wid]: retryTemporaryId
         ? existing.map((thread) => thread.id === temporaryId ? pending : thread)
         : [pending, ...existing],
     };
   });
   try {
     await client.connectWorkspace(wid);
     const result = await client.startThread(wid);
     // Handle Codex CLI JSON-RPC error embedded in result
     if (result && typeof result === "object" && "error" in result) {
       const err = (result as Record<string,unknown>).error as Record<string,unknown> | undefined;
       const msg = typeof err?.message === "string" ? err.message : JSON.stringify(err);
       throw new Error(msg);
     }
     const tid = extractThreadId(result);
     if (!tid) throw new Error("Server did not return a Thread ID");
     const resultRecord = result && typeof result === "object"
       ? result as Record<string, unknown>
       : null;
     const createdThread = resultRecord?.thread && typeof resultRecord.thread === "object"
       ? resultRecord.thread as Record<string, unknown>
       : null;
     const createdProvider = typeof createdThread?.modelProvider === "string"
       ? createdThread.modelProvider
       : typeof createdThread?.model_provider === "string"
         ? createdThread.model_provider
         : currentProviderId;
     const createdModel = typeof createdThread?.model === "string"
       ? createdThread.model
       : providerModels.find((model) => model.id === selectedProviderModelId)?.model
         ?? selectedProviderModelId;
     const createdName = extractThreadName(resultRecord) ?? "Thread";
     setThreadsByWorkspace((previous) => {
       const existing = previous[wid] ?? [];
       const replaced = existing.map((thread) => thread.id === temporaryId
         ? {
             ...thread,
             id: tid,
             label: createdName,
             status: "idle",
             optimistic: true,
             modelProvider: createdProvider,
             model: createdModel,
             creationStatus: undefined,
             creationError: undefined,
           }
         : thread);
       return {
         ...previous,
         [wid]: replaced.filter(
           (thread, index) => replaced.findIndex((candidate) => candidate.id === thread.id) === index,
         ),
       };
     });
     // Bind only the matching temporary window. Concurrent creation responses
     // may resolve in any order without stealing the currently selected Thread.
     if (activeThreadIdRef.current === temporaryId) {
       activeThreadIdRef.current = tid;
       setActiveThreadId(tid);
       setMessages([]);
       setTokenUsage(null);
       setGoal(null);
     setThinking(false);
     setThreadStatus("idle");
     setActiveTurnId(null);
     setStopping(false);
     interruptRequestTurnId.current = null;
     }
     await refreshThreads(wid);
     return tid;
   } catch (error) {
     const message = error instanceof Error ? error.message : String(error);
     setThreadsByWorkspace((previous) => ({
       ...previous,
       [wid]: (previous[wid] ?? []).map((thread) => thread.id === temporaryId
         ? {
             ...thread,
             status: "failed",
             creationStatus: "failed",
             creationError: message,
           }
         : thread),
     }));
     return null;
   }
  }, [activeWorkspaceId, client, currentProviderId, providerModels, refreshThreads, selectedProviderModelId]);

  const archiveThread = useCallback(async (workspaceId: string, threadId: string) => {
    const thread = (threadsByWorkspace[workspaceId] ?? []).find((candidate) => candidate.id === threadId);
    if (!thread) return;
    setBusy(true);
    try {
      await client.archiveThread(workspaceId, threadId);
      setThreadsByWorkspace((previous) => ({
        ...previous,
        [workspaceId]: (previous[workspaceId] ?? []).filter((candidate) => candidate.id !== threadId),
      }));
      if (activeThreadId === threadId) {
        activeThreadIdRef.current = null;
        setActiveThreadId(null);
        setMessages([]);
        setThreadStatus("idle");
        setThinking(false);
      }
    } catch (error) {
      appendLog("error", error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  }, [activeThreadId, appendLog, client, threadsByWorkspace]);

  const sendText = useCallback(async (
    text: string,
    targetWorkspaceId = activeWorkspaceId,
    targetThreadId = activeThreadId,
  ) => {
    if (!targetWorkspaceId || !targetThreadId || !text.trim()) return false;
    appendLog("user", text);
    setThinking(true);
    setTurnStartedAt(Date.now());
    setThreadStatus("running");
    setStopping(false);
    setBusy(true);
    try {
      const selectedModel = providerModels.find((model) => model.id === selectedProviderModelId);
      const response = await client.sendUserMessage(
        targetWorkspaceId,
        targetThreadId,
        text,
        selectedModel?.model ?? selectedProviderModelId,
        currentProviderId,
      );
      const payload = unwrapWebRpcResult(response);
      const record = payload && typeof payload === "object"
        ? payload as Record<string, unknown>
        : null;
      const turn = record?.turn && typeof record.turn === "object"
        ? record.turn as Record<string, unknown>
        : record;
      const turnId = turn?.id ?? record?.turnId ?? record?.turn_id;
      if (typeof turnId === "string" && turnId) setActiveTurnId(turnId);
      const returnedThreadName = normalizeThreadName(
        record?.threadName ?? record?.thread_name,
      );
      if (returnedThreadName && returnedThreadName !== "Thread") {
        setThreadsByWorkspace((previous) => ({
          ...previous,
          [targetWorkspaceId]: (previous[targetWorkspaceId] ?? []).map((thread) =>
            thread.id === targetThreadId && thread.label === "Thread"
              ? { ...thread, label: returnedThreadName, optimistic: false }
              : thread),
        }));
      }
      return true;
    } catch (error) {
      setThinking(false);
      setTurnStartedAt(null);
      setThreadStatus("idle");
      setStopping(false);
      appendLog("error", error instanceof Error ? error.message : String(error));
      return false;
    } finally {
      setBusy(false);
    }
  }, [activeThreadId, activeWorkspaceId, appendLog, client, currentProviderId, providerModels, selectedProviderModelId]);

  const sendMessage = useCallback(async () => {
    const text = draft.trim();
    if (!activeWorkspaceId || !text) return;
    if (!activeThreadId) {
      const threadId = await startThread(activeWorkspaceId);
      if (!threadId) return;
      setDraft("");
      await sendText(text, activeWorkspaceId, threadId);
      return;
    }
    setDraft("");
    const running = thinking
      || threadStatus === "running"
      || threadStatus === "reconnecting"
      || threadStatus.startsWith("active");
    if (running) {
      setQueuedFollowUps((previous) => [...previous, { id: newLogId(), text }]);
      return;
    }
    await sendText(text);
  }, [activeThreadId, activeWorkspaceId, draft, sendText, startThread, thinking, threadStatus]);

  const stopTurn = useCallback(() => {
    if (!activeWorkspaceId || !activeThreadId || stopping) return;
    interruptRequestTurnId.current = null;
    setStopping(true);
  }, [activeThreadId, activeWorkspaceId, stopping]);

  useEffect(() => {
    if (!stopping || !activeWorkspaceId || !activeThreadId || !activeTurnId) return;
    if (interruptRequestTurnId.current === activeTurnId) return;
    interruptRequestTurnId.current = activeTurnId;
    void client.interruptTurn(activeWorkspaceId, activeThreadId, activeTurnId)
      .then(() => {
        if (interruptRequestTurnId.current !== activeTurnId) return;
        setThinking(false);
        setTurnStartedAt(null);
        setThreadStatus("idle");
        setActiveTurnId(null);
        setStopping(false);
        setMessages(finalizeInterruptedTurnEntries);
        setThreadsByWorkspace((previous) => ({
          ...previous,
          [activeWorkspaceId]: (previous[activeWorkspaceId] ?? []).map((thread) =>
            thread.id === activeThreadId ? { ...thread, status: "idle" } : thread),
        }));
        void refreshThreadsRef.current?.(activeWorkspaceId);
      })
      .catch((error) => {
        if (interruptRequestTurnId.current === activeTurnId) interruptRequestTurnId.current = null;
        setStopping(false);
        appendLog("error", error instanceof Error ? error.message : String(error));
      });
  }, [activeThreadId, activeTurnId, activeWorkspaceId, appendLog, client, stopping]);

  useEffect(() => {
    const running = thinking
      || threadStatus === "running"
      || threadStatus === "reconnecting"
      || threadStatus.startsWith("active");
    const next = queuedFollowUps[0];
    if (running || busy || !next || queueDispatching.current) return;
    queueDispatching.current = true;
    setQueuedFollowUps((previous) => previous.filter((item) => item.id !== next.id));
    void sendText(next.text).finally(() => {
      queueDispatching.current = false;
    });
  }, [busy, queuedFollowUps, sendText, thinking, threadStatus]);

  const steerFollowUp = useCallback(async (id: string) => {
    if (!activeWorkspaceId || !activeThreadId || !activeTurnId || steeringFollowUpId) return;
    const item = queuedFollowUps.find((candidate) => candidate.id === id);
    if (!item) return;
    setSteeringFollowUpId(id);
    try {
      await client.steerTurn(activeWorkspaceId, activeThreadId, activeTurnId, item.text);
      setQueuedFollowUps((previous) => previous.filter((candidate) => candidate.id !== id));
    } catch (error) {
      appendLog("error", error instanceof Error ? error.message : String(error));
    } finally {
      setSteeringFollowUpId(null);
    }
  }, [activeThreadId, activeTurnId, activeWorkspaceId, appendLog, client, queuedFollowUps, steeringFollowUpId]);

  const deleteFollowUp = useCallback((id: string) => {
    setQueuedFollowUps((previous) => previous.filter((item) => item.id !== id));
  }, []);

  const submitUserInput = useCallback(async (request: RequestUserInputRequest, response: RequestUserInputResponse) => {
    if (submittingUserInputId !== null) return;
    setSubmittingUserInputId(request.request_id);
    try {
      await client.respondToServerRequest(request.workspace_id, request.request_id, { answers: response.answers });
      setUserInputRequests((previous) => previous.filter((candidate) => !(candidate.workspace_id === request.workspace_id && candidate.request_id === request.request_id)));
    } catch (error) {
      appendLog("error", error instanceof Error ? error.message : String(error));
    } finally {
      setSubmittingUserInputId(null);
    }
  }, [appendLog, client, submittingUserInputId]);

  const resolveApproval = useCallback(async (
    workspaceId: string,
    requestId: number | string,
    decision: "accept" | "decline",
  ) => {
    try {
      await client.respondToServerRequest(workspaceId, requestId, { decision });
      for (const [threadId, approvals] of pendingApprovalsByThread.current) {
        pendingApprovalsByThread.current.set(threadId, approvals.map((entry) =>
          entry.approvalRequestId === requestId
            ? { ...entry, approvalStatus: decision === "accept" ? "accepted" : "declined" }
            : entry));
      }
      setMessages((previous) => previous.map((entry) =>
        entry.approvalRequestId === requestId
          ? { ...entry, approvalStatus: decision === "accept" ? "accepted" : "declined" }
          : entry));
    } catch (error) {
      appendLog("error", error instanceof Error ? error.message : String(error));
    }
  }, [appendLog, client]);

  /* ─── Thread management ─── */

  const selectThread = useCallback(async (id: string) => {
    const hydrationSequence = threadHydrationSequence.current + 1;
    threadHydrationSequence.current = hydrationSequence;
    activeThreadIdRef.current = id;
    // Thread selection is a single visual state transition. Do not let the new
    // transcript inherit live-turn state from the previously selected Thread,
    // otherwise its last historical Turn renders expanded for one frame.
    setThinking(false);
    setThreadStatus("idle");
    setTurnStartedAt(null);
    setActiveThreadId(id);
    const selected = activeWorkspaceId
      ? (threadsByWorkspace[activeWorkspaceId] ?? []).find((thread) => thread.id === id)
      : undefined;
    const threadProvider = selected?.modelProvider ?? null;
    const threadModel = selected?.model ?? null;
    activeThreadModelSelectionRef.current = threadProvider && threadModel
      ? { threadId: id, providerId: threadProvider, modelId: threadModel }
      : null;
    if (selected?.creationStatus) {
      setThreadLoading(false);
      setMessages([]);
      setTokenUsage(null);
      setGoal(null);
      setActiveTurnId(null);
      setStopping(false);
      interruptRequestTurnId.current = null;
      return;
    }
    setThreadLoading(true);
    setMessages(pendingApprovalsByThread.current.get(id) ?? []);
    setTokenUsage(null);
    setGoal(null);
    setActiveTurnId(null);
    setStopping(false);
    interruptRequestTurnId.current = null;
    const wid = activeWorkspaceId;
    if (!wid) {
      setThreadLoading(false);
      return;
    }
    if (threadProvider && threadModel) {
      const cachedProvider = modelProviders.find((provider) => provider.id === threadProvider);
      const cachedModels = modelSummariesForProvider(cachedProvider);
      const immediateModels = cachedModels.length > 0
        ? cachedModels
        : [{ id: threadModel, model: threadModel, displayName: threadModel }];
      setModelProviders((providers) => providers.map((provider) => ({
        ...provider,
        isCurrent: provider.id === threadProvider,
      })));
      setCurrentProviderId(threadProvider);
      setProviderModels(immediateModels);
      setSelectedProviderModelId(
        immediateModels.find((model) => model.model === threadModel)?.id
          ?? immediateModels[0]?.id
          ?? null,
      );
      if (cachedModels.length === 0) {
        // A catalog miss must not delay transcript hydration. This read-only
        // lookup fills the picker after the Thread has already become usable.
        void client.listModels(wid, threadProvider, threadModel).then((response) => {
          if (threadHydrationSequence.current !== hydrationSequence) return;
          const nextModels = parseModelListResponse(response);
          if (nextModels.length === 0) return;
          setProviderModels(nextModels);
          setSelectedProviderModelId(
            nextModels.find((model) => model.model === threadModel)?.id
              ?? nextModels[0]?.id
              ?? null,
          );
        }).catch((error) => {
          if (threadHydrationSequence.current !== hydrationSequence) return;
          setCatalogError(error instanceof Error ? error.message : String(error));
        });
      }
    }
    const cachedTranscript = threadTranscriptCache.current.get(id);
    const selectedIsActive = ["active", "running", "inProgress", "reconnecting"]
      .includes(parseThreadStatus(selected?.status));
    if (
      cachedTranscript
      && !selectedIsActive
      && cachedTranscript.updatedAt >= (selected?.updatedAt ?? 0)
    ) {
      setMessages(cachedTranscript.messages);
      setThreadStatus(cachedTranscript.status);
      setThinking(cachedTranscript.thinking);
      setActiveTurnId(cachedTranscript.activeTurnId);
      setTurnStartedAt(cachedTranscript.turnStartedAt);
      setThreadLoading(false);
      return;
    }
    const revealHydratedThread = () => {
      const reveal = () => {
        if (threadHydrationSequence.current === hydrationSequence) {
          setThreadLoading(false);
        }
      };
      if (typeof window.requestAnimationFrame === "function") {
        window.requestAnimationFrame(reveal);
      } else {
        window.setTimeout(reveal, 0);
      }
    };
    try {
      // `thread/turns/list` is the authoritative full-fidelity history source.
      // The adapter resumes an unbound Runtime Thread internally, so an
      // additional thread/read or explicit resume only duplicates work.
      const turns = await client.listThreadTurns(wid, id);
      if (threadHydrationSequence.current !== hydrationSequence) return;
      const statusType = parseThreadStatus(selected?.status);
      setThreadStatus(statusType);
      setThinking(["active", "running", "inProgress"].includes(statusType));
      const activeTurn = [...turns].reverse().find((turn) => {
        const turnStatus = turn.status;
        return turnStatus === "inProgress"
          || turnStatus === "running"
          || (turnStatus && typeof turnStatus === "object"
            && ["inProgress", "running"].includes(String((turnStatus as Record<string, unknown>).type ?? "")));
      });
      setActiveTurnId(typeof activeTurn?.id === "string" ? activeTurn.id : null);
      if (activeTurn) {
        const rawStartedAt = activeTurn.startedAt;
        setTurnStartedAt(typeof rawStartedAt === "number" && Number.isFinite(rawStartedAt)
          ? rawStartedAt < 10_000_000_000 ? rawStartedAt * 1000 : rawStartedAt
          : Date.now());
      } else {
        setTurnStartedAt(null);
      }
      const historyThread = {
        id,
        turns,
        status: { type: statusType },
      };
      const loaded = buildWebThreadHistory(historyThread, newLogId);
      setMessages((current) => mergeWebThreadHistory(loaded, current));
      revealHydratedThread();
    } catch (error) {
      if (threadHydrationSequence.current !== hydrationSequence) return;
      setMessages((current) => appendWebLogEntry(current, {
        id: newLogId(),
        level: "error",
        text: error instanceof Error ? error.message : "Thread history could not be loaded",
      }));
      revealHydratedThread();
    }
  }, [activeWorkspaceId, client, modelProviders, threadsByWorkspace]);

  /* ─── Render ─── */

  const activeUserInputRequest = userInputRequests.find((request) =>
    request.workspace_id === activeWorkspaceId && request.params.thread_id === activeThreadId,
  ) ?? null;
  const activeThread = activeWorkspaceId && activeThreadId
    ? threadsByWorkspace[activeWorkspaceId]?.find((thread) => thread.id === activeThreadId) ?? null
    : null;
  const activeThreadTitle = activeThread?.label ?? (activeThreadId ? "Thread" : null);
  const retryActiveThreadCreation = () => {
    if (!activeWorkspaceId || activeThread?.creationStatus !== "failed") return;
    void startThread(activeWorkspaceId, activeThread.id);
  };

  return (
    <Layout
      theme={theme}
      sidebarCollapsed={sidebarCollapsed}
      onDismissSidebar={() => setSidebarCollapsed(true)}
      rightPanelOpen={filePanelOpen}
      rightPanelWidth={filePanelWidth}
      rightPanel={
        <FileManager
          workspaceId={activeWorkspaceId}
          selectedPath={selectedFilePath}
          onSelectedPathChange={setSelectedFilePath}
          onClose={() => setFilePanelOpen(false)}
          panelWidth={filePanelWidth}
          onPanelWidthChange={setFilePanelWidth}
          listFiles={listWorkspaceFiles}
          readFile={readWorkspaceFile}
          loadGitStatus={loadWorkspaceGitStatus}
        />
      }
      sidebar={
        <Sidebar
          gatewayState={gatewayState}
          gatewayVersion={gatewayVersion}
          workspaces={workspaces}
          activeWorkspaceId={activeWorkspaceId}
          onSelectWorkspace={setActiveWorkspaceId}
          threadsByWorkspace={threadsByWorkspace}
          activeThreadId={activeThreadId}
          onCreateWorkspace={createWorkspace}

          onSelectThread={selectThread}
          onNewThread={startThread}
          onArchiveThread={archiveThread}
          onRemoveWorkspace={removeWorkspace}
          baseUrl={baseUrl}
          token={token}
          onBaseUrlChange={setBaseUrl}
          onTokenChange={setToken}
          onCheckGateway={checkGateway}
          onLoadWorkspaces={refreshWorkspaces}
          busy={busy}
          mcpServers={mcpServers}
          rateLimits={rateLimits}
          currentProviderId={currentProviderId}
          theme={theme}
          onToggleTheme={() => setTheme((current) => current === "dark" ? "light" : "dark")}

          onConnectWorkspace={connectWorkspace}
        />
      }
    >
      <Conversation
        goal={goal}
        workspaceName={activeWorkspace?.name ?? null}
        threadTitle={activeThreadTitle}
        conversationId={activeThreadId}
        threadLoading={threadLoading}
        threadCreationStatus={activeThread?.creationStatus ?? null}
        threadCreationError={activeThread?.creationError ?? null}
        onRetryThreadCreation={retryActiveThreadCreation}
        sidebarCollapsed={sidebarCollapsed}
        onToggleSidebar={() => setSidebarCollapsed((collapsed) => !collapsed)}
        filePanelOpen={filePanelOpen}
        onToggleFilePanel={() => setFilePanelOpen((open) => !open)}
        onOpenFile={openFile}
          tokenUsage={tokenUsage}
          threadStatus={threadStatus}
          threadSettings={threadSettings}
          providers={modelProviders}
          currentProviderId={currentProviderId}
          models={providerModels}
          catalogLoading={catalogLoading}
          catalogError={catalogError}
          onRefreshCatalog={() => { void refreshModelCatalog(); }}
          onWriteProvider={async (input) => {
            if (!activeWorkspaceId) return;
            setCatalogLoading(true);
            setCatalogError(null);
            try {
              await client.writeModelProvider(activeWorkspaceId, input);
              if (input.action === "upsert" && input.select === true && typeof input.id === "string") {
                await selectProviderAndDefaultModel(input.id);
                return;
              }
              await refreshModelCatalog();
            } catch (error) {
              setCatalogError(error instanceof Error ? error.message : String(error));
              throw error;
            } finally {
              setCatalogLoading(false);
            }
          }}
          onSelectProvider={(providerId) => { void selectProviderAndDefaultModel(providerId); }}
          selectedModelId={selectedProviderModelId}
          onSelectModel={(modelId) => { void selectThreadModel(modelId); }}

        messages={messages}
        workspaceId={activeWorkspaceId ?? undefined}
        draft={draft}
        onDraftChange={setDraft}
        onSend={sendMessage}
        onStop={stopTurn}
        stopping={stopping}
        queuedFollowUps={queuedFollowUps}
        steeringFollowUpId={steeringFollowUpId}
        canSteer={Boolean(activeTurnId) && thinking && !stopping}
        onSteerFollowUp={(id) => { void steerFollowUp(id); }}
        onDeleteFollowUp={deleteFollowUp}
        userInputRequest={activeUserInputRequest}
        submittingUserInput={activeUserInputRequest?.request_id === submittingUserInputId}
        onSubmitUserInput={(request, response) => { void submitUserInput(request, response); }}
        busy={busy}
        sendDisabled={
          !activeWorkspaceId
          || threadLoading
          || activeThread?.creationStatus === "creating"
          || activeThread?.creationStatus === "failed"
        }
        thinking={thinking}
        turnStartedAt={turnStartedAt}
        onResolveApproval={resolveApproval}
      />
    </Layout>
  );
}
