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
import { appendTerminalInteractionOutput, buildWebThreadHistory, commandText, isUserThreadItem, mergeWebThreadHistory, unwrapWebRpcResult, webLogEntryFromThreadItem } from "./utils/webThreadHistory";
import { normalizeTokenUsage } from "./features/threads/utils/threadNormalize";
import { normalizePlanUpdate } from "./features/threads/utils/threadNormalize";
import { parseWebTurnDiff } from "./utils/webTurnDiff";
import { isWebAppServerRecoveryEvent, parseCodexStderr, parseWebAppServerError } from "./utils/webAppServerError";
import { parseWebUserInputRequest } from "./utils/webUserInput";
import { summarizeWebAppServerEvent } from "./utils/webAppServerEventSummary";
import { mergeRateLimits, parseInitialMcpServers, parseInitialRateLimits } from "./utils/webInitialStatus";
import { appendWebLogEntry } from "./utils/webApprovalLog";
import { loadWebApprovalHistory, resolveStoredWebApproval, saveWebApproval } from "./utils/webApprovalHistory";
import { rememberAppServerEvent } from "./utils/webAppServerEventDedup";
import { getAppServerThreadId } from "./utils/appServerEvents";
import "./styles/web.css";
import "./styles/web-refactor.css";

/* ─────────── Types ─────────── */

export type LogEntry = {
  id: string;
  level: "event" | "error" | "info" | "user" | "assistant" | "system";
  text: string;
  approvalId?: string;
  approvalRequestId?: number | string;
  approvalStatus?: "pending" | "accepted" | "declined" | "resolved";
  kind?: "reasoning" | "tool" | "diff" | "approval" | "command_exec" | "connection";
  toolType?: string;
  toolTitle?: string;
  toolStatus?: string;
  toolDetail?: string;
  toolOutput?: string;
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

type GatewayState = "checking" | "online" | "offline";

type ThreadInfo = {
  id: string;
  label: string;
  updatedAt: number;
  turnCount?: number;
  status?: string;
  optimistic?: boolean;
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

function extractThreadIdFromEvent(event: AppServerEvent) {
  const message = event.message ?? {};
  if (message.method !== "thread/started") return null;
  const params =
    message.params && typeof message.params === "object"
      ? (message.params as Record<string, unknown>)
      : null;
  return extractThreadId(params);
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
      const [providerResponse, modelResponse] = await Promise.all([
        client.listModelProviders(activeWorkspaceId),
        client.listModels(activeWorkspaceId),
      ]);
      const providerPayload = unwrapWebRpcResult(providerResponse);
      const providerRecord = providerPayload && typeof providerPayload === "object"
        ? providerPayload as Record<string, unknown>
        : {};
      const rawProviders = Array.isArray(providerRecord.data) ? providerRecord.data : [];
      const nextModels = parseModelListResponse(modelResponse);
      setModelProviders(rawProviders.flatMap((value): ModelProviderSummary[] => {
        if (!value || typeof value !== "object") return [];
        const provider = value as Record<string, unknown>;
        if (typeof provider.id !== "string" || typeof provider.name !== "string") return [];
        return [{
          id: provider.id,
          name: provider.name,
          kind: provider.kind === "local" || provider.kind === "custom" ? provider.kind : "builtIn",
          isCurrent: provider.isCurrent === true,
          modelCount: provider.isCurrent === true
            ? nextModels.length
            : typeof provider.modelCount === "number" ? provider.modelCount : 0,
          baseUrl: typeof provider.baseUrl === "string" ? provider.baseUrl : null,
          envKey: typeof provider.envKey === "string" ? provider.envKey : null,
          wireApi: typeof provider.wireApi === "string" ? provider.wireApi : "responses",
          canEdit: provider.canEdit === true,
          canDelete: provider.canDelete === true,
          canFetchModels: provider.canFetchModels === true,
          models: Array.isArray(provider.models) ? provider.models.flatMap((value) => {
            if (!value || typeof value !== "object") return [];
            const model = value as Record<string, unknown>;
            if (typeof model.modelId !== "string") return [];
            return [{ modelId: model.modelId, modelName: typeof model.modelName === "string" ? model.modelName : null, contextWindow: typeof model.contextWindow === "number" ? model.contextWindow : null }];
          }) : [],
        }];
      }));
      setCurrentProviderId(typeof providerRecord.currentProviderId === "string" ? providerRecord.currentProviderId : null);
      setProviderModels(nextModels);
      setSelectedProviderModelId((selected) =>
        selected && nextModels.some((model) => model.id === selected)
          ? selected
          : nextModels[0]?.id ?? null,
      );
    } catch (error) {
      setCatalogError(error instanceof Error ? error.message : String(error));
    } finally {
      setCatalogLoading(false);
    }
  }, [activeWorkspaceId, client]);

  useEffect(() => {
    setModelProviders([]);
    setProviderModels([]);
    setSelectedProviderModelId(null);
    setCurrentProviderId(null);
    setCatalogError(null);
    if (activeWorkspaceId) void refreshModelCatalog();
  }, [activeWorkspaceId, refreshModelCatalog]);

  useEffect(() => {
    setMcpServers({});
    setRateLimits(null);
    if (!activeWorkspaceId) return;

    let cancelled = false;
    const initializeWorkspaceStatus = async () => {
      try {
        await client.connectWorkspace(activeWorkspaceId);
      } catch {
        return;
      }
      if (cancelled) return;

      const [mcpResult, rateLimitResult] = await Promise.allSettled([
        client.listMcpServerStatus(activeWorkspaceId),
        client.getAccountRateLimits(activeWorkspaceId),
      ]);
      if (cancelled) return;
      if (mcpResult.status === "fulfilled") {
        const snapshot = parseInitialMcpServers(mcpResult.value);
        setMcpServers((current) => ({ ...snapshot, ...current }));
      }
      if (rateLimitResult.status === "fulfilled") {
        const snapshot = parseInitialRateLimits(rateLimitResult.value);
        setRateLimits((current) => mergeRateLimits(snapshot, current));
      }
    };

    void initializeWorkspaceStatus();
    return () => {
      cancelled = true;
    };
  }, [activeWorkspaceId, client]);

  const activeWorkspace = workspaces.find((w) => w.id === activeWorkspaceId) ?? null;
  const listWorkspaceFiles = useCallback((workspaceId: string) => client.listWorkspaceFiles(workspaceId), [client]);
  const readWorkspaceFile = useCallback((workspaceId: string, path: string) => client.readWorkspaceFile(workspaceId, path), [client]);
  const loadWorkspaceGitStatus = useCallback((workspaceId: string) => client.getGitStatus(workspaceId), [client]);
  const openFile = useCallback((path: string) => {
    const workspacePath = activeWorkspace?.path?.replace(/\/$/, "");
    const normalized = workspacePath && path.startsWith(`${workspacePath}/`) ? path.slice(workspacePath.length + 1) : path.replace(/^\//, "");
    setSelectedFilePath(normalized);
    setFilePanelOpen(true);
  }, [activeWorkspace?.path]);

  // Streaming accumulators
  const streamingTexts = useRef<Map<string, string>>(new Map());
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
  const refreshThreadsRef = useRef<((workspaceId?: string) => Promise<void>) | null>(null);
  const activeThreadIdRef = useRef(activeThreadId);
  activeThreadIdRef.current = activeThreadId;
  const appendLog = useCallback(
    (level: LogEntry["level"], text: string, extra?: Partial<Omit<LogEntry, "id" | "level" | "text">>) => {
      setMessages((prev) => appendWebLogEntry(prev, {
        id: newLogId(),
        level,
        text,
        ...extra,
      }));
    },
    [],
  );

  const handleAppEvent = useCallback(
    (event: AppServerEvent): (Partial<Omit<LogEntry, "id" | "level" | "text">> & { level: LogEntry["level"]; text: string }) | null => {
      const message = event.message ?? {};
      const method = typeof message.method === "string" ? message.method : null;
      if (!method) return null;

      const params =
        message.params && typeof message.params === "object"
          ? (message.params as Record<string, unknown>)
          : {};

      const eventThreadId = getAppServerThreadId(event);
      if (eventThreadId && event.workspace_id) {
        if (method === "thread/name/updated") {
          const rawName = params.threadName ?? params.thread_name;
          const label = typeof rawName === "string" && rawName.trim()
            ? rawName.trim()
            : "Thread";
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
        && activeThreadIdRef.current
        && eventThreadId !== activeThreadIdRef.current,
      );
      if (
        belongsToBackgroundThread
        && method !== "item/commandExecution/requestApproval"
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

        case "turn/started":
          setThinking(true);
          setTurnStartedAt(() => {
            const turn = params.turn && typeof params.turn === "object"
              ? params.turn as Record<string, unknown>
              : null;
            const raw = turn?.startedAt ?? params.startedAt ?? params.started_at;
            return typeof raw === "number" && Number.isFinite(raw)
              ? raw < 10_000_000_000 ? raw * 1000 : raw
              : Date.now();
          });
          setThreadStatus("running");
          setActiveTurnId(() => {
            const turn = params.turn && typeof params.turn === "object"
              ? params.turn as Record<string, unknown>
              : null;
            const id = turn?.id ?? params.turnId ?? params.turn_id;
            return typeof id === "string" && id ? id : null;
          });
          return null;

        case "turn/completed":
          // Keep the richer live transcript. The durable thread projection can omit
          // reasoning and tool items, so replacing the transcript here made every
          // execution step disappear as soon as the final answer arrived.
          setThinking(false);
          setTurnStartedAt(null);
          setThreadStatus("idle");
          setActiveTurnId(null);
          setStopping(false);
          interruptRequestTurnId.current = null;
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
            interruptRequestTurnId.current = null;
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
          const id = newLogId();
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
          const updated = current + delta;
          streamingTexts.current.set(itemId, updated);

          const existingLogId = streamingLogIds.current.get(itemId);
          if (existingLogId) {
            setMessages((prev) =>
              prev.map((e) => (e.id === existingLogId ? { ...e, text: updated, streaming: true } : e)),
            );
            return null;
          }
          const id = newLogId();
          streamingLogIds.current.set(itemId, id);
          setMessages((prev) => [
            ...prev.slice(-199),
            { id, level: "assistant", text: updated, streaming: true },
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
              if (eid)
                setMessages((prev) =>
                  prev.map((e) => (e.id === eid ? { ...e, text: acc, streaming: false } : e)),
                );
              streamingTexts.current.delete(completedItemId);
              streamingLogIds.current.delete(completedItemId);
              return null;
            }
            return {
              level: "assistant",
              text: (typeof item.text === "string" ? item.text : "") || "(no response)",
            };
          }

          if (kind === "tool") {
            const toolType = typeof item.toolType === "string" ? item.toolType : "";
            const title = typeof item.title === "string" ? item.title : "";
            const status = typeof item.status === "string" ? item.status : "";
            const filePath = typeof item.filePath === "string" ? item.filePath : undefined;
            return {
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
            const id = newLogId();
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
            const id = newLogId();
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
          interruptRequestTurnId.current = null;
          return { level: "error" as const, text: `Turn error: ${msg}` };
        }

        case "thread/status/changed": {
          const status = params.status as Record<string, unknown> | undefined;
          const type = typeof status?.type === "string" ? status.type : "unknown";
          const activeFlags = Array.isArray(status?.activeFlags) ? (status as Record<string, unknown>).activeFlags as string[] : [];
          const flagsStr = activeFlags.length > 0 ? ":" + activeFlags.join(",") : "";
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
            interruptRequestTurnId.current = null;
          }
          if (type === "error") {
            setThinking(false);
            setTurnStartedAt(null);
            setActiveTurnId(null);
            setStopping(false);
            interruptRequestTurnId.current = null;
            return { level: "error" as const, text: "Thread error" };
          }
          if (type === "systemError") {
            setThinking(false);
            setTurnStartedAt(null);
            setActiveTurnId(null);
            setStopping(false);
            interruptRequestTurnId.current = null;
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
            };
            const current = pendingApprovalsByThread.current.get(approvalThreadId) ?? [];
            pendingApprovalsByThread.current.set(
              approvalThreadId,
              appendWebLogEntry(current, cachedApproval),
            );
            saveWebApproval(event.workspace_id, approvalThreadId, cachedApproval);
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
          resolveStoredWebApproval(event.workspace_id, requestId, "resolved");
          for (const [threadId, approvals] of pendingApprovalsByThread.current) {
            const remaining = approvals.filter((entry) => entry.approvalRequestId !== requestId);
            if (remaining.length > 0) pendingApprovalsByThread.current.set(threadId, remaining);
            else pendingApprovalsByThread.current.delete(threadId);
          }
          if (belongsToBackgroundThread) return null;
          setMessages((previous) => {
            const approval = previous.find((entry) =>
              entry.kind === "approval" && entry.approvalRequestId === requestId);
            const status = approval?.approvalStatus === "accepted" || approval?.approvalStatus === "declined"
              ? approval.approvalStatus
              : "resolved";
            return previous
              .filter((entry) => !(entry.kind === "approval" && entry.approvalRequestId === requestId))
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
            label: String(t.name ?? t.label ?? "Thread"),
            updatedAt: parseThreadUpdatedAt(t.updatedAt ?? t.updated_at),
            turnCount: typeof t.turnCount === "number" ? t.turnCount : undefined,
            status: parseThreadStatus(t.status),
            optimistic: false,
          }));
          // Newly started threads are not listed until their first persisted turn.
          // Preserve the optimistic entry until Runtime returns the canonical row.
          const pending = (prev[wid] ?? []).filter(
            (thread) => thread.optimistic && !received.some((next) => next.id === thread.id),
          );
          return { ...prev, [wid]: [...pending, ...received] };
        });
      }
    } catch { /* not fatal */ }
  }, [activeWorkspaceId, client, workspaces]);
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
        const wsId = activeWorkspaceId;
        if (wsId && event.workspace_id !== wsId) return;
        const tid = extractThreadIdFromEvent(event);
        if (tid && !activeThreadIdRef.current) setActiveThreadId(tid);
        const entry = handleAppEvent(event);
        if (entry) {
          const { level, text, ...extra } = entry;
          appendLog(level, text, extra);
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


 const startThread = useCallback(async (workspaceId?: string): Promise<string | null> => {
   const wid = workspaceId ?? activeWorkspaceId;
   if (!wid) return null;
   setBusy(true);
   try {
     await connectWorkspace(wid);
     setActiveWorkspaceId(wid);
     const result = await client.startThread(wid);
     // Handle Codex CLI JSON-RPC error embedded in result
     if (result && typeof result === "object" && "error" in result) {
       const err = (result as Record<string,unknown>).error as Record<string,unknown> | undefined;
       const msg = typeof err?.message === "string" ? err.message : JSON.stringify(err);
       throw new Error(msg);
     }
     const tid = extractThreadId(result);
     if (tid) {
       // A new thread must begin with a clean transcript, even before its first
       // user message makes it into `thread/list`.
       setActiveThreadId(tid);
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
         if (existing.some((thread) => thread.id === tid)) {
           return previous;
         }
         return {
           ...previous,
           [wid]: [{ id: tid, label: "Thread", updatedAt: Date.now(), optimistic: true }, ...existing],
         };
       });
     }
     await refreshThreads(wid);
     appendLog("info", `Started thread${tid ? ` ${tid}` : ""}.`);
     return tid;
   } catch (error) {
     appendLog("error", error instanceof Error ? error.message : String(error));
     return null;
   } finally {
     setBusy(false);
   }
  }, [activeWorkspaceId, appendLog, client, connectWorkspace, refreshThreads]);

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
      const response = await client.sendUserMessage(targetWorkspaceId, targetThreadId, text, selectedModel?.model ?? selectedProviderModelId);
      const payload = unwrapWebRpcResult(response);
      const record = payload && typeof payload === "object"
        ? payload as Record<string, unknown>
        : null;
      const turn = record?.turn && typeof record.turn === "object"
        ? record.turn as Record<string, unknown>
        : record;
      const turnId = turn?.id ?? record?.turnId ?? record?.turn_id;
      if (typeof turnId === "string" && turnId) setActiveTurnId(turnId);
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
  }, [activeThreadId, activeWorkspaceId, appendLog, client, providerModels, selectedProviderModelId]);

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
    void client.interruptTurn(activeWorkspaceId, activeThreadId, activeTurnId).catch((error) => {
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
      resolveStoredWebApproval(
        workspaceId,
        requestId,
        decision === "accept" ? "accepted" : "declined",
      );
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
    setActiveThreadId(id);
    const restoredApprovals = activeWorkspaceId
      ? loadWebApprovalHistory(activeWorkspaceId, id)
      : [];
    setMessages(mergeWebThreadHistory(
      restoredApprovals,
      pendingApprovalsByThread.current.get(id) ?? [],
    ));
    setTokenUsage(null);
    setGoal(null);
    setActiveTurnId(null);
    setStopping(false);
    interruptRequestTurnId.current = null;
    const wid = activeWorkspaceId;
    if (!wid) return;
    try {
      let resumed: Record<string, unknown> | null = null;
      // Resuming an already-active thread may be rejected; history remains readable.
      try {
        resumed = await client.resumeThread(wid, id);
      } catch {
        // Best effort: `thread/read` below is the source for the browser projection.
      }
      // A resumed thread is the source for live status, but its embedded turns
      // may be a summary projection. Request persisted turns explicitly with
      // `itemsView: full` so history restores commands, reasoning, diffs, MCP
      // calls, and other non-message items. Older runtimes fall back to read.
      let persistedTurns: Record<string, unknown>[] | null = null;
      try {
        persistedTurns = await client.listThreadTurns(wid, id);
      } catch {
        // The paginated history API is experimental and may be absent on an
        // older app-server; retain compatibility with thread/read.
      }
      const raw = resumed ?? await client.readThread(wid, id);
      const payload = unwrapWebRpcResult(raw);
      const obj = payload && typeof payload === "object"
        ? payload as Record<string, unknown>
        : {};
      const thread = obj.thread && typeof obj.thread === "object"
        ? obj.thread as Record<string, unknown>
        : undefined;
      const embeddedTurns = Array.isArray(thread?.turns)
        ? thread.turns as Record<string, unknown>[]
        : Array.isArray(obj.turns)
          ? obj.turns as Record<string, unknown>[]
          : [];
      const turns = persistedTurns && persistedTurns.length > 0
        ? persistedTurns
        : embeddedTurns;
      const status = thread?.status;
      if (status && typeof status === "object") {
        const statusType = (status as Record<string, unknown>).type;
        if (typeof statusType === "string") {
          setThreadStatus(statusType);
          setThinking(statusType === "active");
        }
      }
      const activeTurn = [...turns].reverse().find((turn) => {
        const turnStatus = turn.status;
        return turnStatus === "inProgress"
          || turnStatus === "running"
          || (turnStatus && typeof turnStatus === "object"
            && ["inProgress", "running"].includes(String((turnStatus as Record<string, unknown>).type ?? "")));
      });
      setActiveTurnId(typeof activeTurn?.id === "string" ? activeTurn.id : null);
      if (activeTurn) {
        const rawStartedAt = activeTurn.startedAt ?? activeTurn.started_at;
        setTurnStartedAt(typeof rawStartedAt === "number" && Number.isFinite(rawStartedAt)
          ? rawStartedAt < 10_000_000_000 ? rawStartedAt * 1000 : rawStartedAt
          : Date.now());
      } else {
        setTurnStartedAt(null);
      }
      const historyThread = thread
        ? { ...thread, turns }
        : { turns, status: obj.status };
      const loaded = buildWebThreadHistory(historyThread, newLogId);
      if (threadHydrationSequence.current !== hydrationSequence) return;
      setMessages((current) => mergeWebThreadHistory(loaded, current));
    } catch { /* history load is best-effort */ }
  }, [activeWorkspaceId, client]);

  /* ─── Render ─── */

  const activeUserInputRequest = userInputRequests.find((request) =>
    request.workspace_id === activeWorkspaceId && request.params.thread_id === activeThreadId,
  ) ?? null;
  const activeThreadTitle = activeWorkspaceId && activeThreadId
    ? threadsByWorkspace[activeWorkspaceId]?.find((thread) => thread.id === activeThreadId)?.label ?? "Thread"
    : null;

  return (
    <Layout
      theme={theme}
      sidebarCollapsed={sidebarCollapsed}
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
              await refreshModelCatalog();
            } catch (error) {
              setCatalogError(error instanceof Error ? error.message : String(error));
              throw error;
            } finally {
              setCatalogLoading(false);
            }
          }}
          selectedModelId={selectedProviderModelId}
          onSelectModel={setSelectedProviderModelId}

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
        sendDisabled={!activeWorkspaceId}
        thinking={thinking}
        turnStartedAt={turnStartedAt}
        onResolveApproval={resolveApproval}
      />
    </Layout>
  );
}
