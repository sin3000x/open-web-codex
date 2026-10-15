import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { AppServerEvent, ThreadTokenUsage, WorkspaceInfo } from "./types";
import {
  CodexMonitorWebClient,
  type CopilotTaskSelection,
  type SupervisorOverviewData,
} from "./services/webClient";
import type {
  CopilotProfileStatus,
  McpFormContent,
  McpFormResponseAction,
  PendingMcpFormSummary,
  PendingUserInputSummary,
} from "../browser/types";
import {
  isPlatformRequestError,
  type ProviderCatalogFailure,
} from "../browser/client";
import Layout from "./components/Layout";
import Sidebar from "./components/Sidebar";
import type { CopilotOption } from "./components/Sidebar/Workspaces";
import Conversation from "./components/Conversation";
import FileManager from "./components/FileManager";
import RightSidebar, { type RightSidebarTab } from "./components/RightSidebar";
import SupervisorOverview from "./components/Conversation/SupervisorOverview";
import type { TaskApprovalRequest } from "./components/Conversation/TaskApprovalQueue";
import type { GoalInfo } from "./components/Conversation/GoalBanner";
import type { QueuedFollowUp } from "./components/Conversation/FollowUpQueue";
import type { ModelProviderSummary, ModelSummary } from "./components/Conversation/Composer";
import { parseModelListResponse } from "./features/models/utils/modelListResponse";
import { agentMessagePhase, appendTerminalInteractionOutput, buildWebThreadHistory, commandText, isUserThreadItem, mergeWebThreadHistory, threadTurnDurationMs, unwrapWebRpcResult, webLogEntryFromThreadItem } from "./utils/webThreadHistory";
import { normalizeTokenUsage } from "./features/threads/utils/threadNormalize";
import { normalizePlanUpdate } from "./features/threads/utils/threadNormalize";
import { parseWebTurnDiff } from "./utils/webTurnDiff";
import { isWebAppServerRecoveryEvent, parseCodexStderr, parseWebAppServerError } from "./utils/webAppServerError";
import { summarizeWebAppServerEvent } from "./utils/webAppServerEventSummary";
import { stripLeadingProviderSentinel } from "./utils/providerText";
import { mergeRateLimits, parseInitialMcpServers, parseInitialRateLimits } from "./utils/webInitialStatus";
import { appendWebLogEntry } from "./utils/webApprovalLog";
import { rememberAppServerEvent } from "./utils/webAppServerEventDedup";
import { getAppServerThreadId } from "./utils/appServerEvents";
import { finalizeInterruptedTurnEntries } from "./utils/webInterruptedTurn";
import { createBrowserId } from "./utils/randomId";
import { latestAgentHistoryUpdates } from "./utils/agentWaitUpdates";
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
  approvalCredentialKind?: "maps";
  approvalTool?: string;
  kind?: "reasoning" | "tool" | "diff" | "approval" | "command_exec" | "connection";
  toolType?: string;
  toolTitle?: string;
  toolStatus?: string;
  toolDetail?: string;
  toolOutput?: string;
  agentAction?: "wait";
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
  turnDurationMs?: number;
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

const ACTIVE_WORKSPACE_SESSION_KEY = "open-web-codex:active-workspace:v1";
const activeThreadSessionKey = (workspaceId: string) =>
  `open-web-codex:active-thread:v1:${workspaceId}`;

function readSessionSelection(key: string): string | null {
  try {
    const value = window.sessionStorage.getItem(key)?.trim();
    return value || null;
  } catch {
    return null;
  }
}

function writeSessionSelection(key: string, value: string | null) {
  try {
    if (value) window.sessionStorage.setItem(key, value);
    else window.sessionStorage.removeItem(key);
  } catch {
    // Session selection is a disposable Browser projection. Runtime and
    // Platform state remain authoritative when storage is unavailable.
  }
}

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

const newLogId = () => createBrowserId("log");

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
    if (typeof provider.supportsFunctionTools !== "boolean") {
      throw new Error("Invalid Provider model catalog response");
    }
    const models = Array.isArray(provider.models) ? provider.models.flatMap((value) => {
      if (!value || typeof value !== "object") return [];
      const model = value as Record<string, unknown>;
      if (typeof model.modelId !== "string" || model.showInPicker === false) return [];
      return [{
        modelId: model.modelId,
        modelName: typeof model.modelName === "string" ? model.modelName : null,
        contextWindow: typeof model.contextWindow === "number" ? model.contextWindow : null,
        supportsSearchTool: model.supportsSearchTool === true,
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
      supportsFunctionTools: provider.supportsFunctionTools,
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

const PROVIDER_CATALOG_FAILURE_MESSAGES: Record<ProviderCatalogFailure, string> = {
  authentication: "Provider authentication failed. Check its credential configuration.",
  not_found: "Provider model catalog was not found. Check the Provider URL.",
  rate_limited: "Provider model catalog is rate limited. Try again later.",
  upstream: "The provider could not return its model catalog. Try again later.",
  timeout: "Provider model catalog timed out. Try again.",
  network: "Provider model catalog is unreachable. Check the Provider URL.",
  invalid_json: "Provider returned invalid model catalog data. Check the Provider URL.",
  incompatible_schema: "Provider model catalog is incompatible. Check the Provider URL and schema.",
  empty_catalog: "Provider returned no usable models; the existing catalog was kept.",
};

export function providerCatalogFailureMessage(error: unknown): string {
  if (isPlatformRequestError(error)) {
    const cause = error.detail.providerCatalogFailure;
    if (cause) return PROVIDER_CATALOG_FAILURE_MESSAGES[cause];
  }
  return "Unable to fetch Provider models. Try again.";
}

type ParsedModelProviderCatalog = ReturnType<typeof parseModelProviderCatalog>;

function parseFetchedProviderCatalog(
  value: unknown,
  targetProviderId: string,
  requiredSelection: { providerId: string; modelId: string } | null,
): ParsedModelProviderCatalog | null {
  const payload = unwrapWebRpcResult(value);
  if (!payload || typeof payload !== "object") return null;
  const record = payload as Record<string, unknown>;
  if (!Array.isArray(record.data)) return null;
  if (typeof record.currentProviderId !== "string" || !record.currentProviderId.trim()) return null;
  if (record.currentModelId !== undefined
    && record.currentModelId !== null
    && (typeof record.currentModelId !== "string" || !record.currentModelId.trim())) {
    return null;
  }
  const seenProviderIds = new Set<string>();
  for (const candidate of record.data) {
    if (!candidate || typeof candidate !== "object") return null;
    const provider = candidate as Record<string, unknown>;
    if (typeof provider.id !== "string" || !provider.id.trim()
      || typeof provider.name !== "string" || !provider.name.trim()
      || typeof provider.supportsFunctionTools !== "boolean"
      || seenProviderIds.has(provider.id)) {
      return null;
    }
    seenProviderIds.add(provider.id);
    if (!Array.isArray(provider.models)) return null;
    const seenModelIds = new Set<string>();
    for (const model of provider.models) {
      if (!model || typeof model !== "object") return null;
      const modelRecord = model as Record<string, unknown>;
      if (typeof modelRecord.modelId !== "string" || !modelRecord.modelId.trim()) return null;
      if (seenModelIds.has(modelRecord.modelId)) return null;
      seenModelIds.add(modelRecord.modelId);
    }
  }
  if (!seenProviderIds.has(record.currentProviderId)) return null;
  const target = record.data.find((candidate) => {
    if (!candidate || typeof candidate !== "object") return false;
    return (candidate as Record<string, unknown>).id === targetProviderId;
  });
  if (!target || typeof target !== "object") return null;
  const targetRecord = target as Record<string, unknown>;
  if (typeof targetRecord.name !== "string" || !targetRecord.name.trim()) return null;
  if (!Array.isArray(targetRecord.models) || targetRecord.models.length === 0) return null;

  const catalog = parseModelProviderCatalog(value);
  if (catalog.providers.length !== record.data.length
    || catalog.providers.some((provider) => !seenProviderIds.has(provider.id))) {
    return null;
  }
  const parsedTarget = catalog.providers.find((provider) => provider.id === targetProviderId);
  if (!parsedTarget || (parsedTarget.models?.length ?? 0) === 0) return null;
  const parsedCurrentProvider = catalog.providers.find(
    (provider) => provider.id === record.currentProviderId,
  );
  if (record.currentModelId
    && !parsedCurrentProvider?.models?.some((model) => model.modelId === record.currentModelId)) {
    return null;
  }
  if (requiredSelection) {
    if (!requiredSelection.providerId.trim() || !requiredSelection.modelId.trim()) return null;
    const requiredProvider = catalog.providers.find(
      (provider) => provider.id === requiredSelection.providerId,
    );
    if (!requiredProvider?.models?.some((model) => model.modelId === requiredSelection.modelId)) {
      return null;
    }
  }
  return catalog;
}

type ModelCatalogState = {
  providers: ModelProviderSummary[];
  currentProviderId: string | null;
  providerModels: ModelSummary[];
  selectedProviderModelId: string | null;
};

function projectModelCatalogState(
  catalog: ParsedModelProviderCatalog,
  activeThreadId: string | null,
  threadSelection: { threadId: string; providerId: string; modelId: string } | null,
): ModelCatalogState {
  const selectedProviderId = threadSelection?.threadId === activeThreadId
    ? threadSelection.providerId
    : catalog.currentProviderId;
  const selectedModelId = threadSelection?.threadId === activeThreadId
    ? threadSelection.modelId
    : catalog.currentModelId;
  const selectedProvider = catalog.providers.find((provider) => provider.id === selectedProviderId);
  const providerModels = modelSummariesForProvider(selectedProvider);
  return {
    providers: catalog.providers.map((provider) => ({
      ...provider,
      isCurrent: provider.id === selectedProviderId,
    })),
    currentProviderId: selectedProviderId,
    providerModels,
    selectedProviderModelId: selectedModelId && providerModels.some((model) => model.id === selectedModelId)
      ? selectedModelId
      : providerModels[0]?.id ?? null,
  };
}

/* ─────────── Component ─────────── */

export function resolveTurnStartedAt(
  currentStartedAt: number | null,
  runtimeStartedAt: unknown,
  now = Date.now(),
): number {
  // A locally submitted Turn already has a stable start time. Runtime events
  // can arrive later or be replayed with an older timestamp, so they must not
  // move the visible timer backwards. A Runtime timestamp is only needed when
  // hydrating an already-running Turn after refresh/reconnect.
  if (currentStartedAt !== null) return currentStartedAt;
  if (typeof runtimeStartedAt === "number" && Number.isFinite(runtimeStartedAt)) {
    return runtimeStartedAt < 10_000_000_000 ? runtimeStartedAt * 1000 : runtimeStartedAt;
  }
  return now;
}

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
  const [mapCardAttachment, setMapCardAttachment] = useState<{
    ref: string;
    title: string;
  } | null>(null);
  const [busy, setBusy] = useState(false);
  const [stopping, setStopping] = useState(false);
  const [queuedFollowUps, setQueuedFollowUps] = useState<QueuedFollowUp[]>([]);
  const [steeringFollowUpId, setSteeringFollowUpId] = useState<string | null>(null);
  const [pendingUserInputsById, setPendingUserInputsById] = useState<Map<string, PendingUserInputSummary>>(
    () => new Map(),
  );
  const [submittingPendingUserInputIds, setSubmittingPendingUserInputIds] = useState<Set<string>>(
    () => new Set(),
  );
  const [pendingMcpFormsById, setPendingMcpFormsById] = useState<Map<string, PendingMcpFormSummary>>(
    () => new Map(),
  );
  const [submittingMcpFormIds, setSubmittingMcpFormIds] = useState<Set<string>>(() => new Set());
  const [approvalProjectionRevision, setApprovalProjectionRevision] = useState(0);
  const [submittingApprovalIds, setSubmittingApprovalIds] = useState<Set<string>>(
    () => new Set(),
  );
  const [messages, setMessages] = useState<LogEntry[]>([]);
  const [gatewayState, setGatewayState] = useState<GatewayState>("checking");
  const [gatewayVersion, setGatewayVersion] = useState<string | null>(null);
  const [thinking, setThinking] = useState(false);
  const [turnStartedAt, setTurnStartedAt] = useState<number | null>(null);
  const [tokenUsage, setTokenUsage] = useState<ThreadTokenUsage | null>(null);
  const [threadStatus, setThreadStatus] = useState<string>("idle");
  const [activeTurnId, setActiveTurnId] = useState<string | null>(null);
  const [threadSettings, setThreadSettings] = useState<Record<string, unknown> | null>(null);
  const [supervisorOverview, setSupervisorOverview] =
    useState<SupervisorOverviewData | null>(null);
  const [supervisorOverviewLoading, setSupervisorOverviewLoading] = useState(false);
  const [supervisorOverviewError, setSupervisorOverviewError] = useState<string | null>(null);
  const [rateLimits, setRateLimits] = useState<Record<string, unknown> | null>(null);
  const [goal, setGoal] = useState<GoalInfo | null>(null);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(() =>
    typeof window !== "undefined" && window.matchMedia("(max-width: 760px)").matches,
  );
  const [rightPanelOpen, setRightPanelOpen] = useState(false);
  const [activeRightPanelTab, setActiveRightPanelTab] = useState<RightSidebarTab>("files");
  const [agentPanelUnread, setAgentPanelUnread] = useState(false);
  const [agentActivityFocusRequest, setAgentActivityFocusRequest] = useState(0);
  const [rightPanelWidth, setRightPanelWidth] = useState(() => {
    if (typeof window === "undefined") return 360;
    const stored = Number(window.localStorage.getItem("open-web-codex:right-panel-width:v1"));
    return Number.isFinite(stored) && stored >= 300 && stored <= 720 ? stored : 360;
  });
  const [selectedFilePath, setSelectedFilePath] = useState<string | null>(null);
  const [mcpServers, setMcpServers] = useState<Record<string, {name: string; status: string; error?: string | null; failureReason?: string | null}>>({});
  const [modelProviders, setModelProviders] = useState<ModelProviderSummary[]>([]);
  const [currentProviderId, setCurrentProviderId] = useState<string | null>(null);
  const [providerModels, setProviderModels] = useState<ModelSummary[]>([]);
  const [selectedProviderModelId, setSelectedProviderModelId] = useState<string | null>(null);
  const [providerCatalogOpenRequest, setProviderCatalogOpenRequest] = useState(0);
  const [catalogLoading, setCatalogLoading] = useState(false);
  const [catalogError, setCatalogError] = useState<string | null>(null);
  // A new composer is not a Runtime Thread until its first message is sent.
  const newConversationDraft = useRef<{
    workspaceId: string;
    copilot: CopilotTaskSelection | null;
    operationId: string;
  } | null>(null);
  const startingFirstMessage = useRef(false);
  const [copilots, setCopilots] = useState<CopilotOption[]>([]);
  const [copilotStatus, setCopilotStatus] = useState<CopilotProfileStatus | null>(null);
  const [copilotStatusLoading, setCopilotStatusLoading] = useState(false);
  const [copilotStatusError, setCopilotStatusError] = useState<string | null>(null);
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
    if (activeWorkspaceId) {
      writeSessionSelection(ACTIVE_WORKSPACE_SESSION_KEY, activeWorkspaceId);
    }
  }, [activeWorkspaceId]);

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
    setAgentPanelUnread(false);
  }, [activeThreadId]);

  useEffect(() => {
    window.localStorage.setItem("open-web-codex:right-panel-width:v1", String(rightPanelWidth));
  }, [rightPanelWidth]);

  const client = useMemo(() => new CodexMonitorWebClient({ baseUrl, token }), [baseUrl, token]);

  const refreshModelCatalog = useCallback(async () => {
    setCatalogLoading(true);
    setCatalogError(null);
    try {
      const providerResponse = await client.listModelProviders();
      const catalog = parseModelProviderCatalog(providerResponse);
      const next = projectModelCatalogState(
        catalog,
        activeThreadIdRef.current,
        activeThreadModelSelectionRef.current,
      );
      setModelProviders(next.providers);
      setCurrentProviderId(next.currentProviderId);
      setProviderModels(next.providerModels);
      setSelectedProviderModelId(next.selectedProviderModelId);
      if (!next.currentProviderId || !next.selectedProviderModelId) {
        setCatalogError("Select a Provider and model before starting.");
        setProviderCatalogOpenRequest((request) => request + 1);
      }
    } catch (error) {
      setCatalogError("Unable to load Provider settings. Try again.");
    } finally {
      setCatalogLoading(false);
    }
  }, [client]);

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
    if (startingFirstMessage.current) return;
    const workspaceId = activeWorkspaceId;
    if (!workspaceId) return;
    const previousProviderId = currentProviderId;
    let switched = false;
    setCatalogLoading(true);
    setCatalogError(null);
    try {
      if (activeThreadId && !activeThreadId.startsWith("pending-thread:")) {
        const { thread } = await client.readThread(workspaceId, activeThreadId);
        if (thread.modelProvider !== providerId) {
          throw new Error("Start a new conversation to use a different Provider. You can still change models within this Provider.");
        }
      }
      await client.writeModelProvider({ action: "select", id: providerId });
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
          await client.writeModelProvider({
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
    if (startingFirstMessage.current) return;
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
    void refreshModelCatalog();
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
  const uploadWorkspaceFiles = useCallback((workspaceId: string, files: File[], options?: { overwrite?: boolean; paths?: string[] }) => client.uploadWorkspaceFiles(workspaceId, files, activeThreadId, options), [activeThreadId, client]);
  const readWorkspaceFile = useCallback((workspaceId: string, path: string) => client.readWorkspaceFile(workspaceId, path, activeThreadId), [activeThreadId, client]);
  const downloadWorkspaceFile = useCallback((workspaceId: string, path: string) => client.downloadWorkspaceFile(workspaceId, path, activeThreadId), [activeThreadId, client]);
  const deleteWorkspaceFile = useCallback((workspaceId: string, path: string) => client.deleteWorkspaceFile(workspaceId, path, activeThreadId), [activeThreadId, client]);
  const loadWorkspaceGitStatus = useCallback((workspaceId: string) => client.getGitStatus(workspaceId, activeThreadId), [activeThreadId, client]);
  const openFile = useCallback((path: string) => {
    const workspacePath = activeWorkspace?.path?.replace(/\/$/, "");
    const normalized = workspacePath && path.startsWith(`${workspacePath}/`) ? path.slice(workspacePath.length + 1) : path.replace(/^\//, "");
    setSelectedFilePath(normalized);
    setActiveRightPanelTab("files");
    setRightPanelOpen(true);
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
  const terminalErrorThreads = useRef(new Set<string>());
  const queueDispatching = useRef(false);
  const threadHydrationSequence = useRef(0);
  const recentAppServerEvents = useRef<Map<string, number>>(new Map());
  const pendingApprovalsByThread = useRef<Map<string, LogEntry[]>>(new Map());
  const submittingApprovalIdsRef = useRef<Set<string>>(new Set());
  const threadTranscriptCache = useRef<Map<string, ThreadTranscriptCacheEntry>>(new Map());
  const refreshThreadsRef = useRef<((workspaceId?: string) => Promise<void>) | null>(null);
  const refreshSupervisorOverviewRef =
    useRef<((threadId?: string | null) => Promise<void>) | null>(null);
  const supervisorOverviewSequence = useRef(0);
  const supervisorOverviewRefreshTimer = useRef<number | null>(null);
  const agentActivitySequenceByThread = useRef<Map<string, number>>(new Map());
  const supervisorOverviewRef = useRef(supervisorOverview);
  supervisorOverviewRef.current = supervisorOverview;
  const rightPanelOpenRef = useRef(rightPanelOpen);
  rightPanelOpenRef.current = rightPanelOpen;
  const activeRightPanelTabRef = useRef(activeRightPanelTab);
  activeRightPanelTabRef.current = activeRightPanelTab;
  const activeThreadIdRef = useRef(activeThreadId);
  activeThreadIdRef.current = activeThreadId;
  const activeWorkspaceIdRef = useRef(activeWorkspaceId);
  activeWorkspaceIdRef.current = activeWorkspaceId;
  const pendingUserInputSequence = useRef(0);
  const pendingMcpFormSequence = useRef(0);

  const refreshPendingUserInputs = useCallback(async (
    threadId: string | null = activeThreadIdRef.current,
    runId?: string,
  ) => {
    const sequence = ++pendingUserInputSequence.current;
    if (!threadId && !runId) {
      setPendingUserInputsById(new Map());
      return;
    }
    try {
      const requests = runId
        ? await client.listRunUserInputRequests(runId)
        : await client.listThreadUserInputRequests(threadId as string);
      if (sequence !== pendingUserInputSequence.current) return;
      setPendingUserInputsById(new Map(requests.map((request) => [request.id, request])));
    } catch {
      if (sequence === pendingUserInputSequence.current) setPendingUserInputsById(new Map());
    }
  }, [client]);

  const refreshPendingMcpForms = useCallback(async (
    threadId: string | null = activeThreadIdRef.current,
    runId?: string,
  ) => {
    const sequence = ++pendingMcpFormSequence.current;
    if (!threadId && !runId) {
      setPendingMcpFormsById(new Map());
      return;
    }
    try {
      const requests = runId
        ? await client.listRunMcpFormRequests(runId)
        : await client.listThreadMcpFormRequests(threadId as string);
      if (sequence !== pendingMcpFormSequence.current) return;
      setPendingMcpFormsById(new Map(requests.map((request) => [request.id, request])));
    } catch {
      if (sequence === pendingMcpFormSequence.current) setPendingMcpFormsById(new Map());
    }
  }, [client]);

  const refreshSupervisorOverview = useCallback(async (
    threadId: string | null = activeThreadIdRef.current,
  ) => {
    const sequence = ++supervisorOverviewSequence.current;
    if (!threadId || threadId.startsWith("pending-thread:")) {
      setSupervisorOverview(null);
      setSupervisorOverviewLoading(false);
      setSupervisorOverviewError(null);
      return;
    }
    const visibleOverviewThreadId = supervisorOverviewRef.current?.agents
      .find((agent) => agent.is_root)?.thread_id;
    if (visibleOverviewThreadId !== threadId) {
      setSupervisorOverview(null);
    }
    setSupervisorOverviewLoading(true);
    setSupervisorOverviewError(null);
    try {
      const overview = await client.getSupervisorOverview(threadId);
      if (
        sequence !== supervisorOverviewSequence.current
        || activeThreadIdRef.current !== threadId
      ) {
        return;
      }
      const latestActivitySequence = overview?.activities.reduce(
        (latest, activity) => Math.max(latest, activity.sequence),
        0,
      ) ?? 0;
      const previousActivitySequence = agentActivitySequenceByThread.current.get(threadId);
      agentActivitySequenceByThread.current.set(threadId, latestActivitySequence);
      if (
        previousActivitySequence !== undefined
        && latestActivitySequence > previousActivitySequence
        && (!rightPanelOpenRef.current || activeRightPanelTabRef.current !== "agents")
      ) {
        setAgentPanelUnread(true);
      }
      setSupervisorOverview(overview);
    } catch {
      if (
        sequence !== supervisorOverviewSequence.current
        || activeThreadIdRef.current !== threadId
      ) {
        return;
      }
      if (supervisorOverviewRef.current) {
        setSupervisorOverviewError("The latest collaboration state could not be loaded.");
      } else {
        setSupervisorOverview(null);
      }
    } finally {
      if (
        sequence === supervisorOverviewSequence.current
        && activeThreadIdRef.current === threadId
      ) {
        setSupervisorOverviewLoading(false);
      }
    }
  }, [client]);
  refreshSupervisorOverviewRef.current = refreshSupervisorOverview;

  const scheduleSupervisorOverviewRefresh = useCallback(() => {
    if (supervisorOverviewRefreshTimer.current !== null) {
      window.clearTimeout(supervisorOverviewRefreshTimer.current);
    }
    supervisorOverviewRefreshTimer.current = window.setTimeout(() => {
      supervisorOverviewRefreshTimer.current = null;
      void refreshSupervisorOverviewRef.current?.(activeThreadIdRef.current);
    }, 120);
  }, []);

  useEffect(() => () => {
    if (supervisorOverviewRefreshTimer.current !== null) {
      window.clearTimeout(supervisorOverviewRefreshTimer.current);
    }
  }, []);

  useEffect(() => {
    void refreshSupervisorOverview(activeThreadId);
  }, [activeThreadId, refreshSupervisorOverview]);

  useEffect(() => {
    void refreshPendingUserInputs(activeThreadId);
  }, [activeThreadId, refreshPendingUserInputs]);

  useEffect(() => {
    void refreshPendingMcpForms(activeThreadId);
  }, [activeThreadId, refreshPendingMcpForms]);

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
    setPendingUserInputsById(new Map());
    setSubmittingPendingUserInputIds(new Set());
    setPendingMcpFormsById(new Map());
    setSubmittingMcpFormIds(new Set());
    setTokenUsage(null);
    setGoal(null);
    setThinking(false);
    setTurnStartedAt(null);
    setThreadStatus("idle");
    setActiveTurnId(null);
    setStopping(false);
    setSupervisorOverview(null);
    setSupervisorOverviewLoading(false);
    setSupervisorOverviewError(null);
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
      const runtimeItem = params.item && typeof params.item === "object"
        ? params.item as Record<string, unknown>
        : null;
      const runtimeItemType = typeof runtimeItem?.type === "string" ? runtimeItem.type : null;
      const agentRelevantItem = runtimeItemType !== null && [
        "mcpToolCall",
        "dynamicToolCall",
        "commandExecution",
        "collabAgentToolCall",
        "collabToolCall",
        "webSearch",
        "imageView",
        "imageGeneration",
        "agentMessage",
      ].includes(runtimeItemType);
      if (
        [
          "thread/started",
          "thread/status/changed",
          "thread/completed",
          "thread/failed",
          "turn/started",
          "turn/completed",
          "serverRequest/resolved",
          "platform/artifact/changed",
          "platform/userInputRequested",
          "platform/userInputResolved",
          "platform/mcpFormRequested",
          "platform/mcpFormResolved",
        ].includes(method)
        || (["item/started", "item/completed"].includes(method) && agentRelevantItem)
        || method.endsWith("/requestApproval")
      ) {
        scheduleSupervisorOverviewRefresh();
      }
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
        const nextStatus = method === "thread/status/changed"
          ? parseThreadStatus(params.status)
          : method === "turn/started"
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
        && method !== "platform/userInputRequested"
        && method !== "platform/userInputResolved"
        && method !== "platform/mcpFormRequested"
        && method !== "platform/mcpFormResolved"
      ) {
        return null;
      }
      if (
        belongsToInterruptedTurn
        && method !== "turn/completed"
        && method !== "thread/status/changed"
        && method !== "serverRequest/resolved"
        && method !== "platform/userInputRequested"
        && method !== "platform/userInputResolved"
        && method !== "platform/mcpFormRequested"
        && method !== "platform/mcpFormResolved"
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
          if (eventThreadId) terminalErrorThreads.current.delete(eventThreadId);
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
          setTurnStartedAt((previous) => resolveTurnStartedAt(
            previous,
            turn?.startedAt ?? params.startedAt ?? params.started_at,
          ));
          setThreadStatus("running");
          setActiveTurnId(
            typeof startedTurnId === "string" && startedTurnId ? startedTurnId : null,
          );
          return null;
        }

        case "turn/completed": {
          // Keep the richer live transcript. The durable thread projection can omit
          // reasoning and tool items, so replacing the transcript here made every
          // execution step disappear as soon as the final answer arrived.
          const completedTurnDurationMs = threadTurnDurationMs(eventTurn ?? params);
          setThinking(false);
          setTurnStartedAt(null);
          setThreadStatus("idle");
          setActiveTurnId(null);
          setStopping(false);
          void refreshThreadsRef.current?.(event.workspace_id);
          setMessages((previous) => {
            let turnStartIndex = -1;
            for (let index = previous.length - 1; index >= 0; index -= 1) {
              if (previous[index].level === "user") {
                turnStartIndex = index;
                break;
              }
            }
            const completedTurnEntryIds = turnStartIndex >= 0
              ? new Set(previous.slice(turnStartIndex).map((entry) => entry.id))
              : null;
            return previous
              .filter((entry) => entry.kind !== "connection"
              && !(entry.level === "system" && entry.text === "Thinking...")
              && !(entry.kind === "reasoning"
                && /^(reasoning completed|reasoning in progress|reasoning)$/i.test(entry.text.trim())
                && !entry.reasoningSummary?.trim()))
              .map((entry) => {
                const finalized = entry.streaming
                  ? {
                      ...entry,
                      streaming: false,
                      text: entry.text,
                    }
                  : entry;
                return completedTurnDurationMs !== undefined
                  && completedTurnEntryIds?.has(entry.id)
                  ? { ...finalized, turnDurationMs: completedTurnDurationMs }
                  : finalized;
              });
          });
          if (eventTurn?.error && typeof eventTurn.error === "object") {
            if (eventThreadId) terminalErrorThreads.current.add(eventThreadId);
            setMessages((previous) => previous.filter((entry) => entry.id !== `system-error:${eventThreadId}`));
            return {
              level: "error" as const,
              text: parseWebAppServerError({ error: eventTurn.error }).text,
            };
          }
          if (eventTurn?.status === "failed") {
            return { level: "error" as const, text: "The runtime turn failed." };
          }
          return null;
        }

        case "error": {
          const parsed = parseWebAppServerError(params);
          if (!parsed.recoverable) {
            if (eventThreadId) terminalErrorThreads.current.add(eventThreadId);
            setMessages((previous) => previous.filter((entry) => entry.id !== `system-error:${eventThreadId}`));
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
            if (eventThreadId && terminalErrorThreads.current.has(eventThreadId)) return null;
            return {
              id: `system-error:${eventThreadId}`,
              level: "error" as const,
              text: "The conversation encountered a system error.",
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
          // Public approval events intentionally omit raw command text. The
          // durable approval identity, not that optional display field, owns
          // whether the user can answer the request.
          const cmd = commandText(params.command) || "Approve the requested command execution";
          const requestId = message.id;
          const approvalId = typeof params.itemId === "string" ? params.itemId : undefined;
          const approvalMode = typeof params.mode === "string" ? params.mode : undefined;
          const approvalCredentialKind = params.credentialKind === "maps" ? "maps" : undefined;
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
              approvalCredentialKind,
              approvalTool,
            };
            const current = pendingApprovalsByThread.current.get(approvalThreadId) ?? [];
            pendingApprovalsByThread.current.set(
              approvalThreadId,
              appendWebLogEntry(current, cachedApproval),
            );
            setApprovalProjectionRevision((revision) => revision + 1);
            if (belongsToBackgroundThread) setAgentPanelUnread(true);
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
            approvalCredentialKind,
            approvalTool,
          };
        }

        case "platform/userInputRequested": {
          const runId = typeof params.runId === "string" ? params.runId : null;
          const activeThreadId = activeThreadIdRef.current;
          if (
            runId
            && activeThreadId
            && event.run_id === runId
            && event.root_thread_id === activeThreadId
          ) {
            void refreshPendingUserInputs(activeThreadId, runId);
          }
          return null;
        }

        case "platform/userInputResolved": {
          const approvalId = typeof params.approvalId === "string" ? params.approvalId : null;
          if (approvalId) {
            pendingUserInputSequence.current += 1;
            setPendingUserInputsById((previous) => {
              if (!previous.has(approvalId)) return previous;
              const next = new Map(previous);
              next.delete(approvalId);
              return next;
            });
          }
          return null;
        }

        case "platform/mcpFormRequested": {
          const runId = typeof params.runId === "string" ? params.runId : null;
          const activeThreadId = activeThreadIdRef.current;
          if (
            runId
            && activeThreadId
            && event.run_id === runId
            && event.root_thread_id === activeThreadId
          ) {
            void refreshPendingMcpForms(activeThreadId, runId);
          }
          return null;
        }

        case "platform/mcpFormResolved": {
          const approvalId = typeof params.approvalId === "string" ? params.approvalId : null;
          if (approvalId) {
            pendingMcpFormSequence.current += 1;
            setPendingMcpFormsById((previous) => {
              if (!previous.has(approvalId)) return previous;
              const next = new Map(previous);
              next.delete(approvalId);
              return next;
            });
          }
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
          setApprovalProjectionRevision((revision) => revision + 1);
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
          return null;
        }


        default: {
          const text = summarizeWebAppServerEvent(event);
          return text && text !== "{}" ? { level: "event" as const, text } : null;
        }
      }
    },
    [client, refreshPendingMcpForms, refreshPendingUserInputs, scheduleSupervisorOverviewRefresh],
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
      const stored = readSessionSelection(ACTIVE_WORKSPACE_SESSION_KEY);
      const storedIsAuthorized = Boolean(
        stored && next.some((workspace) => workspace.id === stored),
      );
      if (stored && !storedIsAuthorized) {
        writeSessionSelection(ACTIVE_WORKSPACE_SESSION_KEY, null);
        writeSessionSelection(activeThreadSessionKey(stored), null);
      }
      setWorkspaces(next);
      setActiveWorkspaceId((current) => {
        if (current && next.some((workspace) => workspace.id === current)) return current;
        if (stored && storedIsAuthorized) return stored;
        return next[0]?.id ?? null;
      });
    } catch (error) {
      appendLog("error", error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  }, [appendLog, client]);

  const applyCopilotStatus = useCallback((status: CopilotProfileStatus) => {
    setCopilotStatus(status);
    setCopilotStatusError(null);
    const activePackageIds = new Set(
      status.installations
        .filter((installation) => installation.active && !installation.restartRequired)
        .map((installation) => installation.packageId),
    );
    setCopilots(
      status.packages
        .filter((candidate) => candidate.available && activePackageIds.has(candidate.packageId))
        .map((candidate) => ({
          packageId: candidate.packageId,
          displayName: candidate.displayName ?? candidate.packageId,
        })),
    );
  }, []);

  const refreshCopilots = useCallback(async () => {
    setCopilotStatusLoading(true);
    try {
      const status = await client.copilotProfileStatus();
      applyCopilotStatus(status);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setCopilotStatusError(message);
      appendLog("error", message);
    } finally {
      setCopilotStatusLoading(false);
    }
  }, [appendLog, applyCopilotStatus, client]);

  const activateCopilot = useCallback(async (packageId: string) => {
    setCopilotStatusError(null);
    try {
      applyCopilotStatus(await client.activateCopilot(packageId));
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setCopilotStatusError(message);
      throw error;
    }
  }, [applyCopilotStatus, client]);

  const deactivateCopilot = useCallback(async (packageId: string) => {
    setCopilotStatusError(null);
    try {
      applyCopilotStatus(await client.deactivateCopilot(packageId));
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setCopilotStatusError(message);
      throw error;
    }
  }, [applyCopilotStatus, client]);

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
    void refreshCopilots();
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
  }, [appendLog, checkGateway, client, refreshCopilots, refreshWorkspaces]);

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
        writeSessionSelection(ACTIVE_WORKSPACE_SESSION_KEY, null);
        activeThreadIdRef.current = null;
        setActiveThreadId(null);
        setMessages([]);
      }
      writeSessionSelection(activeThreadSessionKey(workspaceId), null);
      await refreshWorkspaces();
    } catch (error) {
      appendLog("error", error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  }, [activeWorkspaceId, appendLog, client, refreshWorkspaces, workspaces]);


 const prepareNewConversation = useCallback((workspaceId: string, copilot?: CopilotTaskSelection | null) => {
   if (startingFirstMessage.current) return;
   newConversationDraft.current = {
     workspaceId, copilot: copilot ?? null, operationId: `pending-thread:${newLogId()}`,
   };
   threadHydrationSequence.current += 1;
   activeThreadIdRef.current = null;
   activeThreadModelSelectionRef.current = null;
   writeSessionSelection(activeThreadSessionKey(workspaceId), null);
   setActiveWorkspaceId(workspaceId);
   setActiveThreadId(null);
   setThreadLoading(false);
   setMessages([]);
   setDraft("");
   setMapCardAttachment(null);
   setQueuedFollowUps([]);
   setTokenUsage(null);
   setGoal(null);
   setThinking(false);
   setTurnStartedAt(null);
   setThreadStatus("idle");
   setActiveTurnId(null);
   setStopping(false);
   interruptRequestTurnId.current = null;
   setCatalogError(null);
 }, []);

 const startThread = useCallback(async (
   workspaceId?: string,
   retryTemporaryId?: string,
   copilot?: CopilotTaskSelection | null,
 ): Promise<string | null> => {
   const wid = workspaceId ?? activeWorkspaceId;
   if (!wid) return null;
   const selectedModel = providerModels.find(
     (model) => model.id === selectedProviderModelId,
   );
   if (!currentProviderId || !selectedModel) {
     setCatalogError("Select a Provider and model before starting.");
     setProviderCatalogOpenRequest((request) => request + 1);
     return null;
   }
   const providerId = currentProviderId;
   const modelId = selectedModel.model;
   const temporaryId = retryTemporaryId
     ?? `pending-thread:${newLogId()}`;
   const startedAt = Date.now();
   let runAccepted = false;
   const showAcceptedStart = () => {
     runAccepted = true;
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
         [wid]: existing.some((thread) => thread.id === temporaryId)
           ? existing.map((thread) => thread.id === temporaryId ? pending : thread)
           : [pending, ...existing],
       };
     });
   };
   try {
     await client.connectWorkspace(wid);
     const result = await client.startThread(wid, {
       operationId: temporaryId,
       providerId,
       modelId,
       copilot,
       onRunAccepted: showAcceptedStart,
     });
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
       writeSessionSelection(activeThreadSessionKey(wid), tid);
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
     if (!runAccepted) {
       appendLog("error", message);
       return null;
     }
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
  }, [activeWorkspaceId, appendLog, client, currentProviderId, providerModels, refreshThreads, selectedProviderModelId]);

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
        writeSessionSelection(activeThreadSessionKey(workspaceId), null);
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
    mapCardRef: string | null = null,
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
        mapCardRef,
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
    if (!activeWorkspaceId || !text || catalogLoading || startingFirstMessage.current) return;
    if (!activeThreadId) {
      const pending = newConversationDraft.current?.workspaceId === activeWorkspaceId
        ? newConversationDraft.current
        : { workspaceId: activeWorkspaceId, copilot: null, operationId: `pending-thread:${newLogId()}` };
      newConversationDraft.current = pending;
      startingFirstMessage.current = true;
      setBusy(true);
      try {
        const threadId = await startThread(activeWorkspaceId, pending.operationId, pending.copilot);
        if (!threadId) return;
        newConversationDraft.current = null;
        setDraft("");
        await sendText(text, activeWorkspaceId, threadId, null);
      } finally {
        startingFirstMessage.current = false;
        setBusy(false);
      }
      return;
    }
    const selectedMapCardRef = mapCardAttachment?.ref ?? null;
    const running = thinking
      || threadStatus === "running"
      || threadStatus === "reconnecting"
      || threadStatus.startsWith("active");
    if (running) {
      setDraft("");
      setMapCardAttachment(null);
      if (text) {
        setQueuedFollowUps((previous) => [
          ...previous,
          { id: newLogId(), text, mapCardRef: selectedMapCardRef },
        ]);
      }
      return;
    }
    setDraft("");
    setMapCardAttachment(null);
    await sendText(text, activeWorkspaceId, activeThreadId, selectedMapCardRef);
  }, [activeThreadId, activeWorkspaceId, catalogLoading, draft, mapCardAttachment, sendText, startThread, thinking, threadStatus]);

  const stopTurn = useCallback(() => {
    if (!activeWorkspaceId || !activeThreadId || stopping) return;
    interruptRequestTurnId.current = null;
    setStopping(true);
  }, [activeThreadId, activeWorkspaceId, stopping]);

  useEffect(() => {
    setMapCardAttachment(null);
  }, [activeThreadId]);

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
    void sendText(
      next.text,
      activeWorkspaceId,
      activeThreadId,
      next.mapCardRef ?? null,
    ).finally(() => {
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

  const submitPendingUserInput = useCallback(async (
    requestId: string,
    version: number,
    answers: Record<string, { answers: string[] }>,
  ) => {
    if (submittingPendingUserInputIds.has(requestId)) return;
    setSubmittingPendingUserInputIds((previous) => new Set(previous).add(requestId));
    try {
      await client.respondToUserInput(requestId, version, answers);
      pendingUserInputSequence.current += 1;
      setPendingUserInputsById((previous) => {
        if (!previous.has(requestId)) return previous;
        const next = new Map(previous);
        next.delete(requestId);
        return next;
      });
    } catch (error) {
      const status = error && typeof error === "object" && "status" in error
        ? (error as { status?: unknown }).status
        : null;
      if (status === 409) {
        await refreshPendingUserInputs(activeThreadIdRef.current);
      } else {
        appendLog("error", error instanceof Error ? error.message : String(error));
      }
    } finally {
      setSubmittingPendingUserInputIds((previous) => {
        const next = new Set(previous);
        next.delete(requestId);
        return next;
      });
    }
  }, [appendLog, client, refreshPendingUserInputs, submittingPendingUserInputIds]);

  const submitMcpForm = useCallback(async (
    requestId: string,
    version: number,
    action: McpFormResponseAction,
    content?: McpFormContent,
  ) => {
    if (submittingMcpFormIds.has(requestId)) return;
    setSubmittingMcpFormIds((previous) => new Set(previous).add(requestId));
    try {
      await client.respondToMcpForm(requestId, version, action, content);
      pendingMcpFormSequence.current += 1;
      setPendingMcpFormsById((previous) => {
        if (!previous.has(requestId)) return previous;
        const next = new Map(previous);
        next.delete(requestId);
        return next;
      });
    } catch (error) {
      const status = error && typeof error === "object" && "status" in error
        ? (error as { status?: unknown }).status
        : null;
      if (status === 409) {
        await refreshPendingMcpForms(activeThreadIdRef.current);
      } else {
        appendLog("error", error instanceof Error ? error.message : String(error));
      }
    } finally {
      setSubmittingMcpFormIds((previous) => {
        const next = new Set(previous);
        next.delete(requestId);
        return next;
      });
    }
  }, [appendLog, client, refreshPendingMcpForms, submittingMcpFormIds]);

  const resolveApproval = useCallback(async (
    workspaceId: string,
    requestId: number | string,
    decision: "accept" | "decline",
  ) => {
    const submissionId = `${workspaceId}:${String(requestId)}`;
    if (submittingApprovalIdsRef.current.has(submissionId)) return;
    submittingApprovalIdsRef.current.add(submissionId);
    setSubmittingApprovalIds(new Set(submittingApprovalIdsRef.current));
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
      setApprovalProjectionRevision((revision) => revision + 1);
    } catch (error) {
      appendLog("error", error instanceof Error ? error.message : String(error));
    } finally {
      submittingApprovalIdsRef.current.delete(submissionId);
      setSubmittingApprovalIds(new Set(submittingApprovalIdsRef.current));
    }
  }, [appendLog, client]);

  /* ─── Thread management ─── */

  const selectThread = useCallback(async (id: string) => {
    newConversationDraft.current = null;
    const hydrationSequence = threadHydrationSequence.current + 1;
    threadHydrationSequence.current = hydrationSequence;
    activeThreadIdRef.current = id;
    const selected = activeWorkspaceId
      ? (threadsByWorkspace[activeWorkspaceId] ?? []).find((thread) => thread.id === id)
      : undefined;
    if (activeWorkspaceId && selected) {
      writeSessionSelection(activeThreadSessionKey(activeWorkspaceId), id);
    }
    // Thread selection is a single visual state transition. Do not let the new
    // transcript inherit live-turn state from the previously selected Thread,
    // otherwise its last historical Turn renders expanded for one frame.
    setThinking(false);
    setThreadStatus("idle");
    setTurnStartedAt(null);
    setActiveThreadId(id);
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

  useEffect(() => {
    const workspaceId = activeWorkspaceId;
    if (!workspaceId || activeThreadId || activeThreadIdRef.current) return;
    const listedThreads = threadsByWorkspace[workspaceId];
    if (!listedThreads) return;
    const storedThreadId = readSessionSelection(activeThreadSessionKey(workspaceId));
    if (!storedThreadId) return;
    if (!listedThreads.some((thread) => thread.id === storedThreadId)) {
      writeSessionSelection(activeThreadSessionKey(workspaceId), null);
      return;
    }
    void selectThread(storedThreadId);
  }, [activeThreadId, activeWorkspaceId, selectThread, threadsByWorkspace]);

  /* ─── Render ─── */

  const pendingUserInputRequests = useMemo(
    () => Array.from(pendingUserInputsById.values()),
    [pendingUserInputsById],
  );
  const pendingMcpFormRequests = useMemo(
    () => Array.from(pendingMcpFormsById.values()),
    [pendingMcpFormsById],
  );
  const activeThread = activeWorkspaceId && activeThreadId
    ? threadsByWorkspace[activeWorkspaceId]?.find((thread) => thread.id === activeThreadId) ?? null
    : null;
  const activeThreadTitle = activeThread?.label ?? (activeThreadId ? "Thread" : null);
  const taskApprovals = useMemo(() => {
    if (!activeWorkspaceId || !supervisorOverview) return [];
    const agentByThread = new Map(
      supervisorOverview.agents.map((agent) => [agent.thread_id, agent]),
    );
    const requests: TaskApprovalRequest[] = [];
    for (const [threadId, approvals] of pendingApprovalsByThread.current) {
      const agent = agentByThread.get(threadId);
      if (!agent) continue;
      const actorLabel = agent.is_root
        ? "Root Supervisor"
        : agent.agent_nickname?.trim()
          || agent.agent_role?.trim()
          || "Runtime Agent";
      for (const approval of approvals) {
        if (approval.approvalRequestId === undefined) continue;
        const submissionId = `${activeWorkspaceId}:${String(approval.approvalRequestId)}`;
        requests.push({
          threadId,
          actorLabel,
          workspaceId: activeWorkspaceId,
          requestId: approval.approvalRequestId,
          command: approval.text,
          status: approval.approvalStatus,
          mode: approval.approvalMode,
          credentialKind: approval.approvalCredentialKind,
          submitting: submittingApprovalIds.has(submissionId),
        });
      }
    }
    return requests;
  }, [
    activeWorkspaceId,
    approvalProjectionRevision,
    submittingApprovalIds,
    supervisorOverview,
  ]);
  const delegatedTaskApprovals = taskApprovals.filter(
    (approval) => approval.threadId !== activeThreadId,
  );
  const agentWaitUpdates = useMemo(() => latestAgentHistoryUpdates(
    supervisorOverview?.agents ?? [],
    supervisorOverview?.activities ?? [],
  ), [supervisorOverview]);
  const agentPanelAvailable = true;
  const openAgentPanel = () => {
    const alreadyVisible = rightPanelOpen && activeRightPanelTab === "agents";
    setActiveRightPanelTab("agents");
    setRightPanelOpen(!alreadyVisible);
    setAgentPanelUnread(false);
  };
  const showAgentActivity = () => {
    setActiveRightPanelTab("agents");
    setRightPanelOpen(true);
    setAgentPanelUnread(false);
    setAgentActivityFocusRequest((request) => request + 1);
  };
  const openFilePanel = () => {
    const alreadyVisible = rightPanelOpen && activeRightPanelTab === "files";
    setActiveRightPanelTab("files");
    setRightPanelOpen(!alreadyVisible);
  };
  const loadAgentHistory = useCallback((threadId: string) => {
    if (!activeThreadId) {
      return Promise.reject(new Error("Select the Supervisor Thread first."));
    }
    return client.listAgentThreadTurns(activeThreadId, threadId);
  }, [activeThreadId, client]);
  const loadArtifactContent = useCallback(
    (artifactId: string) => client.readArtifactContent(artifactId),
    [client],
  );
  const downloadArtifact = useCallback(
    (artifactId: string) => client.downloadArtifact(artifactId),
    [client],
  );
  const retryActiveThreadCreation = () => {
    if (!activeWorkspaceId || activeThread?.creationStatus !== "failed") return;
    void startThread(
      activeWorkspaceId,
      activeThread.id,
      newConversationDraft.current?.workspaceId === activeWorkspaceId
        ? newConversationDraft.current.copilot : undefined,
    );
  };

  return (
    <Layout
      theme={theme}
      sidebarCollapsed={sidebarCollapsed}
      onDismissSidebar={() => setSidebarCollapsed(true)}
      rightPanelOpen={rightPanelOpen}
      onDismissRightPanel={() => setRightPanelOpen(false)}
      rightPanelWidth={rightPanelWidth}
      rightPanel={
        <RightSidebar
          activeTab={activeRightPanelTab}
          agentUnread={agentPanelUnread}
          width={rightPanelWidth}
          onWidthChange={setRightPanelWidth}
          onTabChange={(tab) => {
            setActiveRightPanelTab(tab);
            if (tab === "agents") setAgentPanelUnread(false);
          }}
          onClose={() => setRightPanelOpen(false)}
          agentPanel={
            <SupervisorOverview
              taskTitle={supervisorOverview?.taskTitle ?? activeThreadTitle ?? "Current task"}
              agents={supervisorOverview?.agents ?? []}
              activities={supervisorOverview?.activities ?? []}
              executions={supervisorOverview?.executions ?? []}
              artifacts={supervisorOverview?.artifacts ?? []}
              approvals={taskApprovals}
              onResolveApproval={resolveApproval}
              onLoadAgentHistory={loadAgentHistory}
              onLoadArtifactContent={loadArtifactContent}
              onDownloadArtifact={downloadArtifact}
              behaviorLogFocusRequest={agentActivityFocusRequest}
              loading={supervisorOverviewLoading}
              error={supervisorOverviewError}
            />
          }
          filePanel={
            <FileManager
              workspaceId={activeWorkspaceId}
              selectedPath={selectedFilePath}
              onSelectedPathChange={setSelectedFilePath}
              onClose={() => setRightPanelOpen(false)}
              panelWidth={rightPanelWidth}
              onPanelWidthChange={setRightPanelWidth}
              listFiles={listWorkspaceFiles}
              uploadFiles={uploadWorkspaceFiles}
              readFile={readWorkspaceFile}
              downloadFile={downloadWorkspaceFile}
              deleteFile={deleteWorkspaceFile}
              loadGitStatus={loadWorkspaceGitStatus}
              embedded
              enabled={rightPanelOpen && activeRightPanelTab === "files"}
            />
          }
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
          copilots={copilots}
          onNewThread={(workspaceId, copilot) => {
            prepareNewConversation(workspaceId, copilot);
          }}
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
          copilotStatus={copilotStatus}
          copilotStatusLoading={copilotStatusLoading}
          copilotStatusError={copilotStatusError}
          onRefreshCopilots={refreshCopilots}
          onActivateCopilot={activateCopilot}
          onDeactivateCopilot={deactivateCopilot}
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
        rightPanelOpen={rightPanelOpen}
        activeRightPanelTab={activeRightPanelTab}
        agentPanelAvailable={agentPanelAvailable}
        agentPanelUnread={agentPanelUnread}
        onOpenAgentPanel={openAgentPanel}
        onShowAgentActivity={showAgentActivity}
        onOpenFilePanel={openFilePanel}
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
            setCatalogLoading(true);
            setCatalogError(null);
            const action = typeof input.action === "string" ? input.action : "upsert";
            try {
              const response = await client.writeModelProvider(input);
              if (action === "fetch") {
                const providerId = typeof input.id === "string" ? input.id.trim() : "";
                const threadSelection = activeThreadModelSelectionRef.current;
                const requiredSelection = threadSelection?.threadId === activeThreadIdRef.current
                  ? {
                    providerId: threadSelection.providerId,
                    modelId: threadSelection.modelId,
                  }
                  : null;
                const catalog = providerId
                  ? parseFetchedProviderCatalog(response, providerId, requiredSelection)
                  : null;
                if (!catalog) throw new Error("Invalid Provider model catalog response");
                const next = projectModelCatalogState(
                  catalog,
                  activeThreadIdRef.current,
                  threadSelection,
                );
                setModelProviders(next.providers);
                setCurrentProviderId(next.currentProviderId);
                setProviderModels(next.providerModels);
                setSelectedProviderModelId(next.selectedProviderModelId);
                return;
              }
              if (action === "upsert" && input.select === true && typeof input.id === "string") {
                if (activeWorkspaceId) {
                  await selectProviderAndDefaultModel(input.id);
                } else {
                  const catalog = parseModelProviderCatalog(response);
                  const next = projectModelCatalogState(catalog, null, null);
                  setModelProviders(next.providers);
                  setCurrentProviderId(next.currentProviderId);
                  setProviderModels(next.providerModels);
                  setSelectedProviderModelId(next.selectedProviderModelId);
                  if (!next.currentProviderId || !next.selectedProviderModelId) {
                    setCatalogError("Select a Provider and model before starting.");
                    setProviderCatalogOpenRequest((request) => request + 1);
                  }
                }
                return;
              }
              await refreshModelCatalog();
            } catch (error) {
              const message = action === "fetch"
                ? providerCatalogFailureMessage(error)
                : "Unable to save Provider settings. Try again.";
              setCatalogError(message);
              if (action !== "fetch") appendLog("error", message);
              throw error;
            } finally {
              setCatalogLoading(false);
            }
          }}
          onSelectProvider={(providerId) => { void selectProviderAndDefaultModel(providerId); }}
          selectedModelId={selectedProviderModelId}
          onSelectModel={(modelId) => { void selectThreadModel(modelId); }}
          providerCatalogOpenRequest={providerCatalogOpenRequest}
        messages={messages}
        finalArtifacts={supervisorOverview?.artifacts ?? []}
        agentWaitUpdates={agentWaitUpdates}
        taskApprovals={delegatedTaskApprovals}
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
        pendingUserInputRequests={pendingUserInputRequests}
        submittingPendingUserInputIds={submittingPendingUserInputIds}
        onSubmitPendingUserInput={(requestId, version, answers) => {
          void submitPendingUserInput(requestId, version, answers);
        }}
        pendingMcpFormRequests={pendingMcpFormRequests}
        submittingMcpFormIds={submittingMcpFormIds}
        onSubmitMcpForm={(requestId, version, action, content) => {
          void submitMcpForm(requestId, version, action, content);
        }}
        busy={busy}
        sendDisabled={
          !activeWorkspaceId
          || catalogLoading
          || !currentProviderId
          || !providerModels.some((model) => model.id === selectedProviderModelId)
          || threadLoading
          || activeThread?.creationStatus === "creating"
          || activeThread?.creationStatus === "failed"
        }
        thinking={thinking}
        turnStartedAt={turnStartedAt}
        onResolveApproval={resolveApproval}
        mapCardAttachment={mapCardAttachment}
        onSelectMapCardForRevision={(ref, title) => {
          setMapCardAttachment({ ref, title });
        }}
        onClearMapCardAttachment={() => setMapCardAttachment(null)}
      />
    </Layout>
  );
}
