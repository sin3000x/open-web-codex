#!/usr/bin/env node

// Opt-in real Provider gate for warehouse-network-copilot.
//
// This script deliberately does not emulate a model, Agent, Skill, MCP server,
// Workspace, algorithm, or map.  It only adds a local forwarding proxy so the
// real DeepSeek request/response can be observed without logging credentials or
// full prompts/tool schemas.  The temporary Provider is removed in finally.

import assert from "node:assert/strict";
import { createHash, randomUUID } from "node:crypto";
import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";
import {
  multiAgentTaskPrompt,
  singleAgentTaskPrompt,
  toolCapabilityTaskPrompt,
} from "./scenarios.mjs";
import {
  facilityChangeMatches,
  hasStandaloneMapEmbedInFinalRootMessage,
  validateBusinessEvidence,
} from "./multi-agent.mjs";
import {
  hasNoChildCollaboration,
  isMinimumFeasibleSolution,
  validateGeneratedFiles,
  validateQuoteMeanEvidence,
} from "./single-agent.mjs";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const fixtureDir = path.join(scriptDir, "..", "fixtures", "warehouse-network", "mock_data");
const fixtureManifestPath = path.join(
  scriptDir,
  "..",
  "fixtures",
  "warehouse-network",
  "mock_data",
  "manifest.json",
);
const baseUrl = (process.env.E2E_BASE_URL ?? "http://127.0.0.1:4810").replace(/\/$/, "");
const apiBase = baseUrl + "/api";
const entry = process.env.E2E_REAL_DEEPSEEK_ENTRY ?? "multi-business";
const isSingleAgentScenario = entry === "single-business";
const isToolCapabilityGate = entry === "tool-capability";
const scenario = isSingleAgentScenario ? "single-agent" : "multi-agent";
const copilotPackageId = isSingleAgentScenario
  ? "warehouse-network-single-agent"
  : "warehouse-network-copilot";
const providerSourceId = process.env.E2E_REAL_DEEPSEEK_SOURCE_PROVIDER_ID ?? "deepseek-e2e";
const model = process.env.E2E_REAL_DEEPSEEK_MODEL ?? "deepseek-v4-flash";
const toolChoiceMode =
  process.env.E2E_REAL_DEEPSEEK_TOOL_CHOICE_MODE ?? "observe";
const useExistingProvider =
  (process.env.E2E_REAL_DEEPSEEK_USE_EXISTING ?? "0") === "1";
const targetBaseUrl = (
  process.env.E2E_REAL_DEEPSEEK_TARGET_BASE_URL ?? "https://api.deepseek.com"
).replace(/\/$/, "");
const username = process.env.E2E_ADMIN_USERNAME ?? "real-deepseek-e2e";
const email =
  process.env.E2E_ADMIN_EMAIL ?? "real-deepseek-e2e@open-web-codex.local";
const password =
  process.env.E2E_ADMIN_PASSWORD ?? "open-web-codex-real-deepseek-e2e";
const secrets = [password].filter(Boolean);
const stamp = new Date().toISOString().replace(/[-:.TZ]/g, "").slice(0, 14);
const serviceTargetHours = 12;
const balikpapanCandidateWarehouseId = "WH-CANDIDATE-BALIKPAPAN";
const results = [];
const state = {
  token: undefined,
  manifest: undefined,
  proxy: undefined,
  cleanupErrors: [],
  userInputInteractions: [],
};

class ApiError extends Error {
  constructor(method, pathname, status, body) {
    super(
      method +
        " " +
        pathname +
        " failed (" +
        status +
        "): " +
        (boundedAssistantSummary(sanitize(body)) ?? "unavailable"),
    );
    this.method = method;
    this.status = status;
    this.pathname = pathname;
    this.body = undefined;
  }
}

class NativeRuntimeBlocker extends Error {
  constructor(code, details) {
    super(code + ": " + JSON.stringify(details));
    this.name = "NativeRuntimeBlocker";
    this.code = code;
    this.details = details;
  }
}

function sanitize(value) {
  let text =
    typeof value === "string"
      ? value
      : JSON.stringify(value) ?? String(value);
  for (const secret of [...secrets, state.token].filter(Boolean)) {
    text = text.split(secret).join("[redacted]");
  }
  return text;
}

function canonicalJson(value) {
  if (Array.isArray(value)) return value.map(canonicalJson);
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value)
        .sort(([left], [right]) => left.localeCompare(right))
        .map(([key, entry]) => [key, canonicalJson(entry)]),
    );
  }
  return value;
}

function log(value) {
  process.stdout.write(sanitize(value) + "\n");
}

async function api(pathname, options = {}) {
  const headers = new Headers(options.headers);
  if (state.token) headers.set("authorization", "Bearer " + state.token);
  if (options.body !== undefined) headers.set("content-type", "application/json");
  const response = await fetch(apiBase + pathname, {
    ...options,
    headers,
    body: options.body === undefined ? undefined : JSON.stringify(options.body),
  });
  const text = await response.text();
  let body;
  if (text) {
    try {
      body = JSON.parse(text);
    } catch {
      body = text;
    }
  }
  if (!response.ok) throw new ApiError(options.method ?? "GET", pathname, response.status, body);
  return body;
}

async function eventually(probe, description, timeoutMs = 60_000, intervalMs = 250) {
  const deadline = Date.now() + timeoutMs;
  let lastError;
  while (Date.now() < deadline) {
    try {
      const value = await probe();
      if (value) return value;
    } catch (error) {
      lastError = error;
    }
    await new Promise((resolve) => setTimeout(resolve, intervalMs));
  }
  throw new Error(
    description + " timed out" + (lastError ? ": " + sanitize(lastError.message) : ""),
  );
}

async function readFixtureManifest() {
  const manifest = JSON.parse(await readFile(fixtureManifestPath, "utf8"));
  assert.equal(manifest.schema_version, 1);
  assert.deepEqual(manifest.excluded_files, [".DS_Store"]);
  assert.equal(manifest.files.length, 6);
  for (const entry of manifest.files) {
    const bytes = await readFile(path.join(fixtureDir, entry.path));
    assert.equal(bytes.length, entry.bytes, entry.path + " byte count drifted");
    assert.equal(
      createHash("sha256").update(bytes).digest("hex"),
      entry.sha256,
      entry.path + " SHA-256 drifted",
    );
  }
  return manifest;
}

async function uploadWorkspaceFile(workspaceId, fixturePath, relativePath) {
  const form = new FormData();
  form.append("file", new Blob([await readFile(fixturePath)]), relativePath);
  const headers = new Headers();
  if (state.token) headers.set("authorization", "Bearer " + state.token);
  const response = await fetch(
    apiBase + "/workspaces/" + encodeURIComponent(workspaceId) + "/files",
    { method: "POST", headers, body: form },
  );
  const text = await response.text();
  if (!response.ok) {
    throw new ApiError("POST", "/workspaces/" + workspaceId + "/files", response.status, text);
  }
  return text ? JSON.parse(text) : undefined;
}

function providerId(catalog, id) {
  return (catalog.data ?? []).find((entry) => entry.id === id);
}

function summarizeToolChoice(value) {
  if (value === undefined) return "omitted";
  if (value === null) return "null";
  if (typeof value === "string") return value;
  if (typeof value === "object" && typeof value.type === "string") return "object:" + value.type;
  return "object";
}

function visibleToolNames(body) {
  if (!Array.isArray(body?.tools)) return [];
  return body.tools
    .map((tool) => tool?.function?.name ?? tool?.name ?? tool?.namespace ?? tool?.type)
    .filter((name) => typeof name === "string")
    .slice(0, 100);
}

function currentChatTurnState(body) {
  const messages = Array.isArray(body?.messages) ? body.messages : [];
  let userIndex = -1;
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    if (messages[index]?.role === "user") {
      userIndex = index;
      break;
    }
  }
  if (userIndex < 0) {
    return {
      has_user_message: false,
      has_structured_tool_activity: false,
      boundary_key: undefined,
    };
  }
  const turnMessages = messages.slice(userIndex);
  const hasStructuredToolActivity = turnMessages.some((message) => {
    if (!message || typeof message !== "object") return false;
    if (message.role === "tool") return true;
    if (
      message.role === "assistant" &&
      ((Array.isArray(message.tool_calls) && message.tool_calls.length > 0) ||
        (message.function_call && typeof message.function_call === "object"))
    ) {
      return true;
    }
    return (
      Array.isArray(message.content) &&
      message.content.some((item) =>
        ["tool_call", "tool_result", "tool_use", "tool_output"].includes(item?.type),
      )
    );
  });
  return {
    has_user_message: true,
    has_structured_tool_activity: hasStructuredToolActivity,
    // The boundary key is an in-process probe key only. It is never logged.
    boundary_key: createHash("sha256")
      .update(JSON.stringify(messages[userIndex]))
      .digest("hex"),
  };
}

function responseToolCalls(text) {
  const names = [];
  let structured = false;
  let terminal = "eof_without_done";
  for (const line of text.split(/\r?\n/)) {
    const payload = line.startsWith("data: ") ? line.slice(6) : line;
    if (!payload) continue;
    if (payload === "[DONE]") {
      terminal = "done";
      continue;
    }
    try {
      const value = JSON.parse(payload);
      const calls = value?.choices?.flatMap((choice) => choice?.delta?.tool_calls ?? []) ?? [];
      for (const call of calls) {
        structured = true;
        const name = call?.function?.name;
        if (typeof name === "string" && !names.includes(name)) names.push(name);
      }
      const outputItems = value?.response?.output ?? [];
      for (const item of outputItems) {
        if (item?.type === "function_call" || item?.type === "tool_search_call") {
          structured = true;
          if (typeof item.name === "string" && !names.includes(item.name)) names.push(item.name);
        }
      }
    } catch {
      // Non-JSON SSE lines are intentionally not retained.
    }
  }
  return { structured, names, terminal };
}

function safeResponseHeaders(headers) {
  const result = {};
  for (const [key, value] of headers) {
    if (["content-encoding", "content-length", "transfer-encoding", "connection"].includes(key)) {
      continue;
    }
    result[key] = value;
  }
  return result;
}

class DeepSeekProbeProxy {
  constructor(mode) {
    this.server = createServer((request, response) => {
      this.forward(request, response).catch((error) => {
        response.writeHead(502, { "content-type": "application/json" });
        response.end(JSON.stringify({ error: "real_provider_proxy_failed" }));
        this.errors.push(String(error?.message ?? error));
      });
    });
    this.address = undefined;
    this.rounds = [];
    this.errors = [];
    this.toolChoicePolicy = undefined;
    this.toolChoiceMode = mode;
    this.structuredTurns = new Set();
  }

  bindExactProviderModel(providerIdValue, modelId) {
    this.toolChoicePolicy = {
      provider_id: providerIdValue,
      model: modelId,
    };
  }

  async start() {
    await new Promise((resolve, reject) => {
      this.server.once("error", reject);
      this.server.listen(0, "127.0.0.1", resolve);
    });
    const address = this.server.address();
    this.address = "http://127.0.0.1:" + address.port;
    return this;
  }

  async close() {
    await new Promise((resolve) => this.server.close(() => resolve()));
  }

  async forward(request, response) {
    const chunks = [];
    for await (const chunk of request) chunks.push(chunk);
    const bodyText = Buffer.concat(chunks).toString("utf8");
    let body;
    try {
      body = bodyText ? JSON.parse(bodyText) : {};
    } catch {
      body = {};
    }
    const isChat = request.url?.includes("/chat/completions") ?? false;
    const requestTools = visibleToolNames(body);
    const originalToolChoice = summarizeToolChoice(body.tool_choice);
    const chatTurn = isChat ? currentChatTurnState(body) : undefined;
    const turnKey =
      chatTurn?.boundary_key && body.model === this.toolChoicePolicy?.model
        ? body.model + ":" + chatTurn.boundary_key
        : undefined;
    const policyMatches =
      isChat &&
      body.model === this.toolChoicePolicy?.model &&
      Array.isArray(body.tools) &&
      body.tools.length > 0;
    const originalChoiceIsAuto =
      body.tool_choice === undefined || body.tool_choice === "auto";
    const turnAlreadyProducedTool = turnKey ? this.structuredTurns.has(turnKey) : false;
    const shouldRequireToolChoice =
      this.toolChoiceMode === "force_first_tool" &&
      policyMatches &&
      originalChoiceIsAuto &&
      chatTurn?.has_user_message === true &&
      chatTurn.has_structured_tool_activity === false &&
      !turnAlreadyProducedTool;
    const effectiveBody = shouldRequireToolChoice
      ? { ...body, tool_choice: "required" }
      : body;
    const metadata = {
      round: this.rounds.length + 1,
      path: request.url ?? "unknown",
      wire_api: isChat ? "chat" : "responses",
      tools_present: Array.isArray(body.tools) && body.tools.length > 0,
      tool_count: Array.isArray(body.tools) ? body.tools.length : 0,
      tool_choice: summarizeToolChoice(effectiveBody.tool_choice),
      original_tool_choice: originalToolChoice,
      effective_tool_choice: summarizeToolChoice(effectiveBody.tool_choice),
      tool_choice_overridden: shouldRequireToolChoice,
      current_turn_has_user_message: chatTurn?.has_user_message ?? false,
      current_turn_has_structured_tool_activity:
        chatTurn?.has_structured_tool_activity ?? false,
      current_turn_tool_call_already_seen: turnAlreadyProducedTool,
      visible_tool_names: requestTools,
    };
    const headers = new Headers();
    for (const [key, value] of Object.entries(request.headers)) {
      if (value === undefined || ["host", "content-length", "connection"].includes(key)) continue;
      headers.set(key, Array.isArray(value) ? value.join(",") : value);
    }
    const forwardedBodyText =
      effectiveBody === body ? bodyText : JSON.stringify(effectiveBody);
    const upstream = await fetch(targetBaseUrl + (request.url ?? "/"), {
      method: request.method,
      headers,
      body: forwardedBodyText || undefined,
    });
    const responseText = await upstream.text();
    const calls = responseToolCalls(responseText);
    const invalidWireToolNames = isChat
      ? calls.names.filter((name) => !requestTools.includes(name))
      : [];
    this.rounds.push({
      ...metadata,
      http_status: upstream.status,
      response_terminal: calls.terminal,
      status: upstream.status,
      structured_tool_calls: calls.structured,
      tool_call_count: calls.names.length,
      wire_tool_names: calls.names,
      invalid_wire_tool_names: invalidWireToolNames,
    });
    if (turnKey && calls.structured) this.structuredTurns.add(turnKey);
    response.writeHead(upstream.status, safeResponseHeaders(upstream.headers));
    response.end(responseText);
  }
}

function itemType(event) {
  return event.payload?.itemType ?? event.payload?.data?.type;
}

function eventData(event) {
  return event?.payload?.data ?? {};
}

function eventTool(event) {
  const data = eventData(event);
  return data.tool ?? data.name ?? data.data?.tool ?? data.result?.tool ?? "";
}

const SAFE_RESULT_KEYS = [
  "code",
  "message",
  "error",
  "status",
  "state",
  "success",
  "type",
  "kind",
  "schema",
  "schema_version",
  "resource_schema",
  "resource_uri",
  "uri",
  "data_ref",
  "map_spec_ref",
  "delivery",
];

function safeStructuredSummary(value) {
  const source = value?.structuredContent ?? value;
  if (!source || typeof source !== "object" || Array.isArray(source)) {
    return typeof source === "string" ? boundedAssistantSummary(source) : undefined;
  }
  const summary = {};
  for (const key of SAFE_RESULT_KEYS) {
    const entry = source[key];
    if (entry === undefined || entry === null) continue;
    if (typeof entry === "string") {
      summary[key] = boundedAssistantSummary(entry);
    } else if (typeof entry === "number" || typeof entry === "boolean") {
      summary[key] = entry;
    } else if (key === "data_ref" && typeof entry === "object") {
      summary[key] = safeStructuredSummary(entry);
    } else if (key === "delivery" && typeof entry === "object") {
      summary[key] = safeStructuredSummary(entry);
    }
  }
  return Object.keys(summary).length > 0 ? summary : undefined;
}

function facilityChangeEvidence(event) {
  const structured = eventData(event)?.result?.structuredContent;
  if (!structured || typeof structured !== "object" || Array.isArray(structured)) {
    return undefined;
  }
  const addedWarehouseIds = Array.isArray(structured.added_warehouse_ids)
    ? structured.added_warehouse_ids.filter((id) => typeof id === "string").slice(0, 8)
    : [];
  const removedWarehouseIds = Array.isArray(structured.removed_warehouse_ids)
    ? structured.removed_warehouse_ids.filter((id) => typeof id === "string").slice(0, 8)
    : [];
  const coverageAtTarget = Array.isArray(structured.coverage)
    ? structured.coverage.find((metric) => metric?.target_hours === serviceTargetHours)
    : undefined;
  const delta = coverageAtTarget?.delta;
  return {
    added_warehouse_ids: addedWarehouseIds,
    removed_warehouse_ids: removedWarehouseIds,
    target_hours: coverageAtTarget?.target_hours,
    city_coverage_rate_delta:
      typeof delta?.city_coverage_rate === "number" ? delta.city_coverage_rate : undefined,
    demand_weighted_coverage_rate_delta:
      typeof delta?.demand_weighted_coverage_rate === "number"
        ? delta.demand_weighted_coverage_rate
        : undefined,
  };
}

function safeEventItem(event) {
  const data = eventData(event);
  const error = data.error ?? data.result?.error;
  const errorCode =
    typeof error === "object"
      ? error.code ?? error.type ?? error.codexErrorInfo
      : typeof error === "string"
        ? error
        : undefined;
  const errorMessage =
    typeof error === "object" ? error.message ?? error.detail : undefined;
  const facilityChange = eventTool(event) === "assess_facility_change"
    ? facilityChangeEvidence(event)
    : undefined;
  return {
    thread_id: event.thread_id,
    turn_id: event.turn_id,
    event_type: event.event_type,
    item_type: itemType(event),
    tool: eventTool(event) || undefined,
    status: typeof data.status === "string" ? data.status : undefined,
    error_code: typeof errorCode === "string" ? errorCode : undefined,
    error_message: boundedAssistantSummary(errorMessage),
    result_summary: safeStructuredSummary(data.result),
    facility_change: facilityChange,
  };
}

function nativeToolNames(events) {
  const names = [];
  for (const event of events) {
    const data = eventData(event);
    const namespace = data.namespace ?? data.toolNamespace;
    const name = data.name ?? data.toolName;
    if (typeof namespace === "string" && typeof name === "string") {
      const value = namespace + "." + name;
      if (!names.includes(value)) names.push(value);
    } else if (typeof data.tool === "string" && !names.includes(data.tool)) {
      names.push(data.tool);
    }
  }
  return names;
}

function eventSummary(events) {
  return events.map((event) => ({
    event_type: event.event_type,
    turn_id: event.turn_id,
    item_type: itemType(event),
    tool: eventTool(event) || undefined,
  }));
}

function safeRuntimeErrors(events) {
  const errors = [];
  for (const event of events) {
    const data = eventData(event);
    const error = data.error ?? data.turn?.error;
    const errorObject = error && typeof error === "object" ? error : undefined;
    const codexErrorInfo =
      errorObject?.codexErrorInfo ?? data.codexErrorInfo ?? data.failureReason;
    const code =
      errorObject?.code ??
      errorObject?.type ??
      (typeof codexErrorInfo === "string" ? codexErrorInfo : undefined);
    const message =
      errorObject?.message ??
      data.message ??
      (typeof data.failureReason === "string" ? data.failureReason : undefined);
    const sourceType = data.sourceType ?? event.event_type;
    if (!error && !code && !message && event.event_type !== "codex.unknown") continue;
    errors.push({
      event_type: event.event_type,
      source_type: typeof sourceType === "string" ? sourceType : undefined,
      thread_id: event.thread_id,
      turn_id: event.turn_id,
      item_id: event.item_id,
      code: typeof code === "string" ? code : undefined,
      message: boundedAssistantSummary(message),
    });
  }
  return errors.slice(-40);
}

function boundedAssistantSummary(value) {
  if (typeof value !== "string") return undefined;
  const normalized = sanitize(value).replace(/\s+/g, " ").trim();
  if (!normalized) return undefined;
  return normalized.length > 240 ? normalized.slice(0, 237) + "..." : normalized;
}

function eventAgentRole(data) {
  for (const key of ["agentRole", "agent_role", "subagentRole", "subagent_role"]) {
    if (typeof data?.[key] === "string" && data[key].trim()) return data[key].trim();
  }
  return undefined;
}

function safeTimeline(events, rounds, agentProjections = []) {
  const threads = new Map();
  const collaboration = [];
  const mcp = [];
  const mapProducers = [];
  const mailbox = [];
  let lastAssistantText;
  for (const event of events) {
    const data = eventData(event);
    const threadId = event.thread_id ?? data.threadId ?? data.thread_id;
    const turnId = event.turn_id ?? data.turnId ?? data.turn_id;
    const role = eventAgentRole(data);
    if (typeof threadId === "string") {
      const current = threads.get(threadId) ?? {
        thread_id: threadId,
        role,
        turns: [],
        terminal: [],
      };
      if (!current.role && role) current.role = role;
      if (typeof turnId === "string" && !current.turns.some((turn) => turn.turn_id === turnId)) {
        current.turns.push({ turn_id: turnId, statuses: [] });
      }
      const turn = current.turns.find((candidate) => candidate.turn_id === turnId);
      if (turn && !turn.statuses.includes(event.event_type)) turn.statuses.push(event.event_type);
      if (/thread\.(completed|failed|cancelled|interrupted)/i.test(event.event_type)) {
        if (!current.terminal.includes(event.event_type)) current.terminal.push(event.event_type);
      }
      threads.set(threadId, current);
    }
    const item = safeEventItem(event);
    const itemName = String(item.tool ?? "");
    if (item.item_type === "collabAgentToolCall" || item.item_type === "collabToolCall") {
      collaboration.push(item);
    }
    if (/mailbox|mail|message|wait_agent|followup|resume_agent/i.test(event.event_type + " " + itemName)) {
      mailbox.push(item);
    }
    if (item.item_type === "mcpToolCall" || /^mcp__/.test(itemName)) {
      mcp.push(item);
    }
    if (/map|create_network_map_card/i.test(itemName) || /map/i.test(String(item.item_type))) {
      mapProducers.push(item);
    }
    if (item.item_type === "agentMessage") {
      const summary = boundedAssistantSummary(data.text ?? data.message ?? data.content);
      if (summary) lastAssistantText = summary;
    }
  }
  for (const agent of agentProjections) {
    const threadId = agent.thread_id ?? agent.threadId;
    if (typeof threadId !== "string") continue;
    const current = threads.get(threadId) ?? {
      thread_id: threadId,
      turns: [],
      terminal: [],
    };
    const role = agent.agent_role ?? agent.agentRole;
    if (typeof role === "string" && role.trim()) current.role = role.trim();
    const parentThreadId = agent.parent_thread_id ?? agent.parentThreadId;
    if (typeof parentThreadId === "string") current.parent_thread_id = parentThreadId;
    const status = agent.status_type ?? agent.statusType;
    if (typeof status === "string" && status.trim()) current.status = status.trim();
    threads.set(threadId, current);
  }
  const lastCollaborationTerminal = collaboration
    .filter((item) =>
      /completed|failed|cancelled|interrupted|error/i.test(
        String(item.event_type) + " " + String(item.status),
      ),
    )
    .at(-1);
  return {
    rounds: rounds.slice(-40).map((round) => ({
      round: round.round,
      wire_api: round.wire_api,
      tool_count: round.tool_count,
      visible_tool_names: round.visible_tool_names.slice(0, 40),
      structured_tool_calls: round.structured_tool_calls,
      wire_tool_names: round.wire_tool_names,
      original_tool_choice: round.original_tool_choice,
      effective_tool_choice: round.effective_tool_choice,
      tool_choice_overridden: round.tool_choice_overridden,
      current_turn_has_structured_tool_activity:
        round.current_turn_has_structured_tool_activity,
      current_turn_tool_call_already_seen: round.current_turn_tool_call_already_seen,
      http_status: round.http_status,
      response_terminal: round.response_terminal,
      invalid_wire_tool_names: round.invalid_wire_tool_names,
    })),
    threads: [...threads.values()].slice(-20),
    collaboration: collaboration.slice(-40),
    mailbox: mailbox.slice(-40),
    mcp: mcp.slice(-60),
    map_producers: mapProducers.slice(-20),
    last_collaboration_terminal: lastCollaborationTerminal,
    errors: safeRuntimeErrors(events),
    last_assistant_text: lastAssistantText,
  };
}

function logTimeline(label, timeline) {
  const rounds = (timeline?.rounds ?? []).map((round) => ({
    round: round.round,
    wire_api: round.wire_api,
    http_status: round.http_status,
    response_terminal: round.response_terminal,
    tool_count: round.tool_count,
    structured_tool_calls: round.structured_tool_calls,
    wire_tool_names: round.wire_tool_names,
    invalid_wire_tool_names: round.invalid_wire_tool_names,
    original_tool_choice: round.original_tool_choice,
    effective_tool_choice: round.effective_tool_choice,
    tool_choice_overridden: round.tool_choice_overridden,
  }));
  log(`[${label} ROUNDS] ` + JSON.stringify(rounds));
  log(`[${label} THREADS] ` + JSON.stringify(timeline?.threads ?? []));
  log(`[${label} COLLABORATION] ` + JSON.stringify(timeline?.collaboration ?? []));
  log(`[${label} MAILBOX] ` + JSON.stringify(timeline?.mailbox ?? []));
  log(`[${label} MCP] ` + JSON.stringify(timeline?.mcp ?? []));
  log(`[${label} MAP_PRODUCERS] ` + JSON.stringify(timeline?.map_producers ?? []));
  log(`[${label} LAST_COLLAB_TERMINAL] ` + JSON.stringify(timeline?.last_collaboration_terminal));
  log(`[${label} ERRORS] ` + JSON.stringify(timeline?.errors ?? []));
  log(`[${label} LAST_ASSISTANT] ` + JSON.stringify(timeline?.last_assistant_text));
}

async function runSelfTests() {
  const model = {
    modelId: "deepseek-v4-flash",
    supportsSearchTool: true,
  };
  assert.equal(model.modelId, "deepseek-v4-flash");
  assert.equal(model.supportsSearchTool, true);
  const initialTurn = currentChatTurnState({
    messages: [{ role: "user", content: "start" }],
  });
  assert.equal(initialTurn.has_user_message, true);
  assert.equal(initialTurn.has_structured_tool_activity, false);
  const completedToolTurn = currentChatTurnState({
    messages: [
      { role: "user", content: "start" },
      {
        role: "assistant",
        tool_calls: [{ id: "call-1", type: "function" }],
      },
      { role: "tool", tool_call_id: "call-1", content: "{}" },
    ],
  });
  assert.equal(completedToolTurn.has_user_message, true);
  assert.equal(completedToolTurn.has_structured_tool_activity, true);
  assert.equal(
    invalidWireToolRound([
      { wire_api: "chat", invalid_wire_tool_names: ["stale_tool"] },
    ])?.invalid_wire_tool_names[0],
    "stale_tool",
  );
  assert.equal(
    turnFailureEvent(
      [{ turn_id: "turn-ok", event_type: "codex.turn.completed", payload: { data: {} } }],
      "turn-ok",
    ),
    undefined,
  );
  assert.equal(
    turnFailureEvent(
      [
        {
          turn_id: "turn-failed",
          event_type: "codex.turn.completed",
          payload: { data: { error: { code: "provider_protocol_violation" } } },
        },
      ],
      "turn-failed",
    )?.event_type,
    "codex.turn.completed",
  );
  assert.match(multiAgentTaskPrompt, /Balikpapan/);
  assert.match(multiAgentTaskPrompt, /地图/);
  assert.doesNotMatch(multiAgentTaskPrompt, /agent_type|spawn_agent|mcp|Resource|outputs\//i);
  assert.match(singleAgentTaskPrompt, /用脚本/);
  assert.match(singleAgentTaskPrompt, /haversine/);
  assert.doesNotMatch(
    singleAgentTaskPrompt,
    /opening_policy|warehouse_quote_mean_calculation|plan_cost_matrix|outputs\//i,
  );
  assert.match(toolCapabilityTaskPrompt, /tool_search.*spawn_agent/s);
  assert.doesNotMatch(toolCapabilityTaskPrompt, /outputs\//i);
  assert.equal(
    isMinimumFeasibleSolution(
      {
        status: "optimal",
        opening_policy: { kind: "minimum_feasible" },
        selected_number_to_open: 1,
        minimum_number_to_open_proven: true,
        solver_stages: [
          { kind: "minimum_openings", status: "optimal", optimality: "proven" },
          { kind: "minimum_cost", status: "optimal", optimality: "proven" },
        ],
        coverage: [{
          target_hours: serviceTargetHours,
          demand_weighted_coverage_rate: 0.9,
        }],
      },
      serviceTargetHours,
    ),
    true,
  );
  assert.equal(hasNoChildCollaboration({ collaboration: [], nativeToolNames: [] }), true);
  const facilityChangeSelfTest = facilityChangeMatches({
    event: {
      payload: {
        data: {
          tool: "assess_facility_change",
          arguments: {
            before_ref: { resource_schema: "network_baseline.v2" },
            scenario: {
              add_warehouse_ids: [balikpapanCandidateWarehouseId],
              objective: "min_time",
              service_targets: [serviceTargetHours],
            },
          },
          result: {
            structuredContent: {
              added_warehouse_ids: [balikpapanCandidateWarehouseId],
              removed_warehouse_ids: [],
              coverage: [{
                target_hours: serviceTargetHours,
                delta: {
                  city_coverage_rate: 0.02,
                  demand_weighted_coverage_rate: 0.03,
                },
              }],
            },
          },
        },
      },
    },
    eventData,
    facilityChangeEvidence,
    candidateWarehouseId: balikpapanCandidateWarehouseId,
    serviceTargetHours,
  });
  assert.equal(facilityChangeSelfTest.valid, true);
  const currentBaseline = {
    resource_ref: { resource_schema: "network_baseline.v2" },
  };
  const currentCoverage = {
    target_hours: serviceTargetHours,
    city_coverage_rate: 0.66,
    demand_weighted_coverage_rate: 0.815,
  };
  const validMultiEvidence = validateBusinessEvidence({
    dataHandoff: {
      outcome: "ready",
      operation: "created",
      prepared_input_relative_path: "outputs/warehouse-network/prepared/input.json",
      input_identity: { content_sha256: "a".repeat(64) },
      role_counts: { demand: 1, existing_warehouse: 1 },
      warnings: [],
      warning_count: 0,
      warnings_truncated: false,
      candidate_warehouse_count: 12,
    },
    baseline: currentBaseline,
    baselineCoverage: currentCoverage,
    roles: new Set(["data_agent", "network_agent"]),
    facilityChangeValid: true,
    mapProducerCompleted: true,
    rootFinalMapEmbed: true,
    generatedWorkspaceFiles: ["outputs/warehouse-network/prepared/input.json"],
  });
  assert.equal(validMultiEvidence.valid, true);
  const legacyBaselineValidation = validateBusinessEvidence({
    dataHandoff: {
      outcome: "ready",
      operation: "created",
      prepared_input_relative_path: "outputs/warehouse-network/prepared/input.json",
      input_identity: { content_sha256: "a".repeat(64) },
      role_counts: { demand: 1, existing_warehouse: 1 },
      warnings: [],
      warning_count: 0,
      warnings_truncated: false,
      candidate_warehouse_count: 12,
    },
    baseline: {
      schema_version: "network_baseline.v2",
      coverage: [currentCoverage],
    },
    baselineCoverage: currentCoverage,
    roles: new Set(["data_agent", "network_agent"]),
    facilityChangeValid: true,
    mapProducerCompleted: true,
    rootFinalMapEmbed: true,
    generatedWorkspaceFiles: ["outputs/warehouse-network/prepared/input.json"],
  });
  assert.equal(legacyBaselineValidation.code, "multi_agent_typed_baseline_invalid");
  const missingMapEmbedValidation = validateBusinessEvidence({
    dataHandoff: validMultiEvidence.valid
      ? {
          outcome: "ready",
          operation: "created",
          prepared_input_relative_path: "outputs/warehouse-network/prepared/input.json",
          input_identity: { content_sha256: "a".repeat(64) },
          role_counts: { demand: 1, existing_warehouse: 1 },
          warnings: [],
          warning_count: 0,
          warnings_truncated: false,
          candidate_warehouse_count: 12,
        }
      : undefined,
    baseline: currentBaseline,
    baselineCoverage: currentCoverage,
    roles: new Set(["data_agent", "network_agent"]),
    facilityChangeValid: true,
    mapProducerCompleted: true,
    rootFinalMapEmbed: false,
    generatedWorkspaceFiles: ["outputs/warehouse-network/prepared/input.json"],
  });
  assert.equal(missingMapEmbedValidation.code, "map_embed_not_forwarded");
  assert.equal(
    hasStandaloneMapEmbedInFinalRootMessage(
      [
        {
          thread_id: "root",
          payload: { itemType: "agentMessage", data: { text: "map child" } },
        },
        {
          thread_id: "root",
          payload: {
            itemType: "agentMessage",
            data: { text: "done\n\n::codex-inline-vis{artifact=\"map-1\"}" },
          },
        },
      ],
      "root",
    ),
    true,
  );
  assert.equal(
    hasStandaloneMapEmbedInFinalRootMessage(
      [
        {
          thread_id: "root",
          payload: {
            itemType: "agentMessage",
            data: { text: "地图已生成 ::codex-inline-vis{artifact=\"map-1\"}" },
          },
        },
      ],
      "root",
    ),
    false,
  );
  let paginationCalls = [];
  const pagedProjection = await readEventProjection({
    taskId: "self-pagination",
    fetchPage: async (afterSequence) => {
      paginationCalls.push(afterSequence);
      if (afterSequence === 0) {
        return Array.from({ length: EVENT_PROJECTION_PAGE_SIZE }, (_, index) => ({
          sequence: index + 1,
          event_type: index === 0 ? "codex.turn.completed" : "codex.item.delta",
          turn_id: "turn-self",
        }));
      }
      if (afterSequence === EVENT_PROJECTION_PAGE_SIZE) {
        return [{
          sequence: EVENT_PROJECTION_PAGE_SIZE + 1,
          event_type: "codex.item.completed",
          turn_id: "turn-self",
          payload: {
            data: {
              tool: "solve_p_median",
              status: "completed",
              result: { structuredContent: { status: "optimal" } },
            },
          },
        }];
      }
      return [];
    },
  });
  assert.deepEqual(paginationCalls, [0, EVENT_PROJECTION_PAGE_SIZE]);
  assert.equal(pagedProjection.rawEventCount, EVENT_PROJECTION_PAGE_SIZE + 1);
  assert.equal(pagedProjection.lastSequence, EVENT_PROJECTION_PAGE_SIZE + 1);
  assert.deepEqual(pagedProjection.events.map((event) => event.sequence), [1, 201]);

  let lateProjectionFetchCount = 0;
  let lateProjectionClock = 0;
  const lateProjection = await awaitProjectionConvergence("self-task", {
    label: "self-test late cost/solve projection",
    turnId: "turn-self",
    fetchEvents: async () => {
      const includeLateTools = lateProjectionFetchCount++ > 0;
      return readEventProjection({
        taskId: "self-task",
        fetchPage: async (afterSequence) => {
          if (afterSequence < 8_400) {
            return Array.from({ length: EVENT_PROJECTION_PAGE_SIZE }, (_, index) => ({
              sequence: afterSequence + index + 1,
              event_type:
                afterSequence === 0 && index === 0
                  ? "codex.turn.completed"
                  : "codex.item.delta",
              turn_id: "turn-self",
            }));
          }
          if (!includeLateTools) return [];
          return [
            {
              sequence: 8_401,
              event_type: "codex.item.completed",
              turn_id: "turn-self",
              payload: {
                data: {
                  tool: "plan_cost_matrix",
                  status: "completed",
                  result: {
                    structuredContent: {
                      calculation_rule_source: "observed_quote_mean",
                    },
                  },
                },
              },
            },
            {
              sequence: 8_402,
              event_type: "codex.item.completed",
              turn_id: "turn-self",
              payload: {
                data: {
                  tool: "solve_p_median",
                  status: "completed",
                  result: { structuredContent: { status: "optimal" } },
                },
              },
            },
          ];
        },
      });
    },
    predicate: (events) =>
      hasCompletedStructuredTool(events, "plan_cost_matrix") &&
      hasCompletedStructuredTool(events, "solve_p_median"),
    timeoutMs: 5_000,
    stableMs: 2_000,
    pollMs: 1,
    now: () => lateProjectionClock,
    sleep: async (milliseconds) => {
      lateProjectionClock += Math.max(1, milliseconds);
    },
  });
  assert.equal(lateProjection.predicate_satisfied, true);
  assert.equal(lateProjection.events.length, 3);
  assert.equal(lateProjection.rawEventCount, 8_402);
  assert.equal(lateProjection.filtered_event_count, 3);

  let fullPageCalls = 0;
  await assert.rejects(
    () =>
      readEventProjection({
        taskId: "self-page-limit",
        fetchPage: async (afterSequence) => {
          fullPageCalls += 1;
          return Array.from({ length: EVENT_PROJECTION_PAGE_SIZE }, (_, index) => ({
            sequence: afterSequence + index + 1,
            event_type: "codex.item.delta",
          }));
        },
      }),
    (error) =>
      error instanceof NativeRuntimeBlocker &&
      error.code === "event_projection_page_limit" &&
      fullPageCalls === EVENT_PROJECTION_PAGE_LIMIT,
  );
  let stableMissingClock = 0;
  const stableMissing = await awaitProjectionConvergence("self-task", {
    label: "self-test stable missing projection",
    turnId: "turn-self",
    fetchEvents: async () => ({
      events: [{ turn_id: "turn-self", event_type: "codex.turn.completed", sequence: 1 }],
      lastSequence: 1,
      rawEventCount: 1,
      pages: 1,
      truncated: false,
    }),
    predicate: (events) => hasCompletedStructuredTool(events, "solve_p_median"),
    timeoutMs: 5_000,
    stableMs: 2_000,
    pollMs: 1,
    now: () => stableMissingClock,
    sleep: async (milliseconds) => {
      stableMissingClock += Math.max(1, milliseconds);
    },
  });
  assert.equal(stableMissing.predicate_satisfied, false);
  assert.equal(stableMissing.terminal_seen, true);
  log("[PASS] real DeepSeek per-model capability self-test");
  log("[PASS] real DeepSeek current-turn tool-choice self-test");
  log("[PASS] real DeepSeek Balikpapan facility-change self-test");
  log("[PASS] real DeepSeek baseline DTO-shape self-test");
  log("[PASS] real DeepSeek projection convergence self-test");
}

function invalidWireToolRound(rounds) {
  return rounds.find(
    (round) =>
      round.wire_api === "chat" &&
      Array.isArray(round.invalid_wire_tool_names) &&
      round.invalid_wire_tool_names.length > 0,
  );
}

function assertProviderTransportSafety(rounds, providerIdValue) {
  const invalidRound = invalidWireToolRound(rounds);
  if (invalidRound) {
    throw new NativeRuntimeBlocker("provider_tool_call_not_visible", {
      provider_id: providerIdValue,
      stage: "current_turn",
      round: {
        round: invalidRound.round,
        visible_tool_count: invalidRound.tool_count,
        visible_tool_names: invalidRound.visible_tool_names,
        wire_tool_names: invalidRound.wire_tool_names,
        invalid_wire_tool_names: invalidRound.invalid_wire_tool_names,
        original_tool_choice: invalidRound.original_tool_choice,
        effective_tool_choice: invalidRound.effective_tool_choice,
      },
    });
  }
  const serverFailure = rounds.find(
    (round) => typeof round.http_status === "number" && round.http_status >= 500,
  );
  if (serverFailure) {
    throw new NativeRuntimeBlocker("provider_server_failure", {
      provider_id: providerIdValue,
      round: {
        round: serverFailure.round,
        http_status: serverFailure.http_status,
        response_terminal: serverFailure.response_terminal,
      },
    });
  }
  const disconnectedAfterTool = rounds.find(
    (round) =>
      round.wire_api === "chat" &&
      round.structured_tool_calls === true &&
      round.response_terminal !== "done",
  );
  if (disconnectedAfterTool) {
    throw new NativeRuntimeBlocker("provider_stream_disconnected_after_tool", {
      provider_id: providerIdValue,
      round: {
        round: disconnectedAfterTool.round,
        response_terminal: disconnectedAfterTool.response_terminal,
        wire_tool_names: disconnectedAfterTool.wire_tool_names,
      },
    });
  }
}

async function ensureAuthenticated() {
  const health = await api("/health");
  assert.equal(health.ok, true);
  let auth;
  try {
    auth = await api("/bootstrap", {
      method: "POST",
      body: { name: "Real DeepSeek E2E Owner", username, email, password },
    });
  } catch (error) {
    if (!(error instanceof ApiError) || error.status !== 409) throw error;
    auth = await api("/sessions/local", { method: "POST" });
  }
  state.token = auth.session_token;
  return health.version;
}

function installationId(status, packageId) {
  return (status.installations ?? []).find(
    (entry) => (entry.package_id ?? entry.packageId) === packageId,
  );
}

async function ensureCopilotActive() {
  let status = await api("/profile/copilots");
  const current = installationId(status, copilotPackageId);
  if (!current?.active) {
    const activated = await api("/profile/copilots/activate", {
      method: "POST",
      body: { packageId: copilotPackageId },
    });
    throw new NativeRuntimeBlocker("copilot_restart_required", {
      package_id: copilotPackageId,
      restart_required:
        activated.restartRequired ?? activated.restart_required ?? true,
    });
  }
  status = await api("/profile/copilots");
  const target = installationId(status, copilotPackageId);
  assert(target?.active === true);
  if (target.restartRequired ?? target.restart_required) {
    const activated = await api("/profile/copilots/activate", {
      method: "POST",
      body: { packageId: copilotPackageId },
    });
    throw new NativeRuntimeBlocker("copilot_restart_required", {
      package_id: copilotPackageId,
      state: target.state,
      restart_required:
        activated.restartRequired ?? activated.restart_required ?? true,
      reason: "package_revision_refreshed",
    });
  }
  const stateValue = String(target.state).toLowerCase();
  if (stateValue !== "configured") {
    throw new NativeRuntimeBlocker("copilot_runtime_unavailable", {
      package_id: copilotPackageId,
      state: target.state,
      restart_required: false,
    });
  }
  return target;
}

async function enableMultiAgent() {
  const settings = await api("/profile/agents/settings", {
    method: "PUT",
    body: { multiAgentEnabled: true, maxThreads: 4, maxDepth: 3 },
  });
  assert.equal(settings.multiAgentEnabled ?? settings.multi_agent_enabled, true);
}

async function configureModelToolSearch(providerIdValue) {
  const refreshed = await api(
    "/providers/" + encodeURIComponent(providerIdValue) + "/models/refresh",
    { method: "POST" },
  );
  const refreshedProvider = providerId(refreshed, providerIdValue);
  const refreshedModel = refreshedProvider?.models?.find(
    (entry) => entry.modelId === model,
  );
  if (!refreshedModel) {
    throw new NativeRuntimeBlocker("model_not_returned_by_provider_catalog", {
      provider_id: providerIdValue,
      model,
      available_model_count: refreshedProvider?.models?.length ?? 0,
    });
  }
  const configured = await api(
    "/providers/" +
      encodeURIComponent(providerIdValue) +
      "/models/" +
      encodeURIComponent(model),
    {
      method: "PATCH",
      body: {
        contextWindow: refreshedModel.contextWindow ?? 128_000,
        supportsSearchTool: true,
      },
    },
  );
  const configuredProvider = providerId(configured, providerIdValue);
  const configuredModel = configuredProvider?.models?.find(
    (entry) => entry.modelId === model,
  );
  if (!configuredModel || configuredModel.supportsSearchTool !== true) {
    throw new NativeRuntimeBlocker("model_tool_search_capability_not_persisted", {
      provider_id: providerIdValue,
      model,
      response_provider_ids: Array.isArray(configured?.data)
        ? configured.data.map((entry) => entry?.id).filter((id) => typeof id === "string")
        : [],
      response_keys: configured && typeof configured === "object"
        ? Object.keys(configured)
        : [],
      configured: configuredModel
        ? { supports_search_tool: configuredModel.supportsSearchTool === true }
        : null,
    });
  }
  const profileConfig = await api("/profile/files/config");
  const runtimeConfigCapability =
    typeof profileConfig.content === "string" &&
    profileConfig.content.includes(`model_id = "${model}"`) &&
    profileConfig.content.includes("supports_search_tool = true");
  log("[CONFIG CAPABILITY] " + JSON.stringify({ runtime_config_capability: runtimeConfigCapability }));
  return {
    originalModel: refreshedModel,
    configuredModel,
  };
}

async function configureTemporaryProvider() {
  const catalog = await api("/providers");
  const source = providerId(catalog, providerSourceId);
  if (!source) {
    throw new NativeRuntimeBlocker("provider_not_configured", {
      provider_id: providerSourceId,
      available_provider_ids: (catalog.data ?? []).map((entry) => entry.id),
    });
  }
  if (source.kind === "builtIn" || source.kind === "BuiltIn") {
    throw new NativeRuntimeBlocker("provider_not_editable", { provider_id: providerSourceId });
  }
  if (!source.envKey && !source.env_key) {
    throw new NativeRuntimeBlocker("provider_credentials_unavailable", {
      provider_id: providerSourceId,
      credential_configured: false,
    });
  }
  if (useExistingProvider) {
    const originalSupports = process.env.E2E_REAL_DEEPSEEK_ORIGINAL_SUPPORTS_FUNCTION_TOOLS;
    if (!["0", "1"].includes(originalSupports ?? "")) {
      throw new NativeRuntimeBlocker("provider_capability_restore_contract_missing", {
        provider_id: providerSourceId,
        required_environment: "E2E_REAL_DEEPSEEK_ORIGINAL_SUPPORTS_FUNCTION_TOOLS=0|1",
      });
    }
    await api("/providers/" + encodeURIComponent(source.id), {
      method: "PUT",
      body: {
        name: source.name,
        baseUrl: state.proxy.address + "/v1",
        wireApi: "chat",
        supportsFunctionTools: true,
        credentials: { mode: "preserve" },
        select: false,
      },
    });
    const record = {
      id: source.id,
      source,
      restoreExisting: true,
      originalSupportsFunctionTools: originalSupports === "1",
      originalBaseUrl: source.baseUrl,
      originalWireApi: source.wireApi,
      originalName: source.name,
      originalModel: source.models?.find((entry) => entry.modelId === model),
    };
    try {
      const modelCapability = await configureModelToolSearch(source.id);
      return { ...record, configuredModel: modelCapability.configuredModel };
    } catch (error) {
      await removeTemporaryProvider(record);
      throw error;
    }
  }
  const temporaryId = "deepseek-real-e2e-" + stamp.toLowerCase();
  const envKey = source.envKey ?? source.env_key;
  const result = await api("/providers/" + encodeURIComponent(temporaryId), {
    method: "PUT",
    body: {
      name: "DeepSeek Real E2E (temporary)",
      baseUrl: state.proxy.address + "/v1",
      wireApi: "chat",
      supportsFunctionTools: true,
      credentials: { mode: "environment", envKey },
      select: false,
    },
  });
  const configured = providerId(result, temporaryId);
  assert(configured);
  // The current ProviderService selects a newly-created Provider as part of
  // its atomic upsert.  Keep that selection for this Run: Runtime resolves
  // the task's Provider from the current config, and the original selection
  // is restored immediately before deleting the temporary Provider.
  const restoreProviderId = catalog.currentProviderId ?? catalog.current_provider_id;
  const record = {
    id: temporaryId,
    source,
    modelProviderCatalog: result,
    restoreProviderId,
    restoreModelId: catalog.currentModelId ?? catalog.current_model_id,
  };
  try {
    const modelCapability = await configureModelToolSearch(temporaryId);
    return { ...record, configuredModel: modelCapability.configuredModel };
  } catch (error) {
    await removeTemporaryProvider(record);
    throw error;
  }
}

async function removeTemporaryProvider(provider) {
  if (!provider?.id) return;
  const cleanupCall = async (label, operation) => {
    try {
      return await operation();
    } catch (error) {
      state.cleanupErrors.push(label + ": " + sanitize(error?.message ?? error));
      return undefined;
    }
  };
  if (provider.restoreExisting) {
    await cleanupCall("restore existing Provider", () => api(
      "/providers/" + encodeURIComponent(provider.id),
      {
        method: "PUT",
        body: {
          name: provider.originalName,
          baseUrl: provider.originalBaseUrl,
          wireApi: provider.originalWireApi,
          supportsFunctionTools: provider.originalSupportsFunctionTools,
          credentials: { mode: "preserve" },
          select: false,
        },
      },
    ));
    if (provider.originalModel) {
      await cleanupCall("restore existing model capability", () => api(
        "/providers/" +
            encodeURIComponent(provider.id) +
            "/models/" +
            encodeURIComponent(model),
        {
          method: "PATCH",
          body: {
            contextWindow: provider.originalModel.contextWindow ?? 128_000,
            supportsSearchTool: provider.originalModel.supportsSearchTool === true,
          },
        },
      ));
    }
    return;
  }
  if (provider.restoreProviderId) {
    await cleanupCall("restore Provider selection", () => api(
      "/providers/" + encodeURIComponent(provider.restoreProviderId) + "/select",
      { method: "POST" },
    ));
    if (provider.restoreModelId) {
      await cleanupCall("restore model selection", () => api(
        "/providers/" +
          encodeURIComponent(provider.restoreProviderId) +
          "/models/" +
          encodeURIComponent(provider.restoreModelId) +
          "/select",
        { method: "POST" },
      ));
    }
  }
  await cleanupCall("delete temporary Provider", () => api(
    "/providers/" + encodeURIComponent(provider.id),
    { method: "DELETE" },
  ));
  const catalog = await cleanupCall("verify temporary Provider deletion", () => api("/providers"));
  if (catalog && providerId(catalog, provider.id)) {
    state.cleanupErrors.push("temporary Provider remains after deletion");
  }
}

async function createTaskAndRun(providerIdValue, title) {
  const project = await api("/projects/managed", {
    method: "POST",
    body: { name: title + " project" },
  });
  const workspace = await api("/workspaces", {
    method: "POST",
    body: {
      project_id: project.id,
      idempotency_key: "real-deepseek-workspace-" + randomUUID(),
      kind: "main",
      name: title + " workspace",
      source_ref: null,
      parent_workspace_id: null,
      copy_agents_md: false,
    },
  });
  const record = { project, workspace, task: undefined, run: undefined };
  try {
    for (const entry of state.manifest.files) {
      await uploadWorkspaceFile(
        workspace.id,
        path.join(fixtureDir, entry.path),
        "mock_data/" + entry.path,
      );
    }
    const task = await api("/tasks", {
      method: "POST",
      body: {
        project_id: project.id,
        workspace_id: workspace.id,
        title,
        model_provider: providerIdValue,
        model,
        copilot_package_id: copilotPackageId,
      },
    });
    record.task = task;
    const selectedPackageId = task.copilot_package_id ?? task.copilotPackageId;
    if (selectedPackageId !== copilotPackageId) {
      throw new NativeRuntimeBlocker("copilot_package_selection_mismatch", {
        expected_package_id: copilotPackageId,
        observed_package_id: selectedPackageId,
      });
    }
    const started = await api("/tasks/" + task.id + "/runs", {
      method: "POST",
      body: { idempotency_key: "real-deepseek-run-" + randomUUID() },
    });
    record.run = await eventually(
      async () => {
        const current = await api("/runs/" + (started.run ?? started).id);
        return current.codex_thread_id ? current : undefined;
      },
      "Real DeepSeek Run readiness",
      90_000,
      500,
    );
    return record;
  } catch (error) {
    await cleanupCase(record);
    throw error;
  }
}

async function send(taskId, minimal = false) {
  const text = minimal
    ? toolCapabilityTaskPrompt
    : isSingleAgentScenario
      ? singleAgentTaskPrompt
      : multiAgentTaskPrompt;
  return api("/tasks/" + taskId + "/messages", {
    method: "POST",
    body: {
      text,
      model,
      model_provider: taskProviderId,
      effort: "none",
      service_tier: null,
      access_mode: "workspace-write",
      images: [],
      collaboration_mode: null,
    },
  });
}

const EVENT_PROJECTION_PAGE_SIZE = 200;
const EVENT_PROJECTION_PAGE_LIMIT = 200;

function retainProjectionEvent(event) {
  const eventType = String(event?.event_type ?? "");
  const item = String(itemType(event) ?? "");
  if (
    /delta$|token_usage\.updated|tokenUsageUpdated/i.test(eventType)
  ) {
    return false;
  }
  if (
    eventType === "codex.item.completed" ||
    eventType === "codex.item.failed" ||
    eventType === "codex.item.cancelled" ||
    eventType === "codex.item.interrupted" ||
    eventType === "codex.unknown" ||
    /approval|error|failed|cancelled|interrupted/i.test(eventType) ||
    /thread\.(started|completed|failed|cancelled|interrupted)|turn\.(started|completed|failed|cancelled|interrupted)/i.test(
      eventType,
    ) ||
    /mcpToolCall|collabAgentToolCall|collabToolCall/i.test(item)
  ) {
    return true;
  }
  return item === "agentMessage" && /completed|done/i.test(eventType);
}

async function readEventProjection({ taskId, fetchPage }) {
  const events = [];
  let afterSequence = 0;
  let rawEventCount = 0;
  let pages = 0;
  let lastBatchLength = 0;
  let lastSequence = 0;
  while (pages < EVENT_PROJECTION_PAGE_LIMIT) {
    const batch = await fetchPage(afterSequence);
    if (!Array.isArray(batch)) {
      throw new NativeRuntimeBlocker("event_projection_invalid_page", {
        task_id: taskId,
        page: pages + 1,
      });
    }
    pages += 1;
    lastBatchLength = batch.length;
    if (batch.length === 0) break;
    let pageSequence = afterSequence;
    for (const event of batch) {
      if (!Number.isInteger(event?.sequence) || event.sequence <= pageSequence) {
        throw new NativeRuntimeBlocker("event_projection_sequence_invalid", {
          task_id: taskId,
          page: pages,
          after_sequence: afterSequence,
        });
      }
      pageSequence = event.sequence;
      if (retainProjectionEvent(event)) events.push(event);
    }
    rawEventCount += batch.length;
    lastSequence = pageSequence;
    afterSequence = pageSequence;
    if (batch.length < EVENT_PROJECTION_PAGE_SIZE) break;
  }
  if (pages >= EVENT_PROJECTION_PAGE_LIMIT && lastBatchLength === EVENT_PROJECTION_PAGE_SIZE) {
    throw new NativeRuntimeBlocker("event_projection_page_limit", {
      task_id: taskId,
      page_limit: EVENT_PROJECTION_PAGE_LIMIT,
      raw_event_count: rawEventCount,
      last_sequence: lastSequence,
    });
  }
  return {
    events,
    lastSequence,
    rawEventCount,
    pages,
    truncated: false,
  };
}

async function taskEventsSnapshot(taskId) {
  return readEventProjection({
    taskId,
    fetchPage: (afterSequence) => {
      const query =
        "?after_sequence=" +
        encodeURIComponent(afterSequence) +
        "&limit=" +
        EVENT_PROJECTION_PAGE_SIZE;
      return api("/tasks/" + taskId + "/events" + query);
    },
  });
}

function hasCompletedStructuredTool(events, toolName) {
  return events.some(
    (event) =>
      event.event_type === "codex.item.completed" &&
      eventData(event)?.status === "completed" &&
      String(eventTool(event)).includes(toolName) &&
      eventData(event)?.result?.structuredContent !== undefined,
  );
}

function hasCompletedSpawnAgent(events) {
  return events.some(
    (event) =>
      event.event_type === "codex.item.completed" &&
      eventData(event)?.status === "completed" &&
      /spawnAgent|spawn_agent|multi_agent_v1/.test(String(eventTool(event))),
  );
}

async function awaitProjectionConvergence(
  taskId,
  {
    label,
    turnId,
    predicate,
    timeoutMs = 30_000,
    stableMs = 2_000,
    pollMs = 250,
    fetchEvents = taskEventsSnapshot,
    now = () => Date.now(),
    sleep = (milliseconds) =>
      new Promise((resolve) => setTimeout(resolve, milliseconds)),
  },
) {
  const startedAt = now();
  let latestEvents = [];
  let lastSequence;
  let lastRawEventCount;
  let stableSince;
  while (now() - startedAt <= timeoutMs) {
    try {
      const snapshot = await fetchEvents(taskId);
      if (!snapshot || snapshot.truncated === true || !Array.isArray(snapshot.events)) {
        throw new NativeRuntimeBlocker(
          snapshot?.truncated === true
            ? "event_projection_page_limit"
            : "event_projection_snapshot_invalid",
          {
            label,
            turn_id: turnId,
          },
        );
      }
      latestEvents = snapshot.events;
      const rawSequence = snapshot.lastSequence;
      const rawEventCount = snapshot.rawEventCount;
      if (!Number.isInteger(rawSequence) || !Number.isInteger(rawEventCount)) {
        throw new NativeRuntimeBlocker("event_projection_snapshot_invalid", {
          label,
          turn_id: turnId,
        });
      }
      if (
        rawSequence !== lastSequence ||
        rawEventCount !== lastRawEventCount
      ) {
        stableSince = now();
        lastSequence = rawSequence;
        lastRawEventCount = rawEventCount;
      }
      if (predicate(latestEvents)) {
        return {
          ...snapshot,
          predicate_satisfied: true,
          terminal_seen: latestEvents.some(
            (event) =>
              event.turn_id === turnId && event.event_type === "codex.turn.completed",
          ),
          filtered_event_count: latestEvents.length,
        };
      }
      const terminalSeen = latestEvents.some(
        (event) =>
          event.turn_id === turnId && event.event_type === "codex.turn.completed",
      );
      if (terminalSeen && stableSince !== undefined && now() - stableSince >= stableMs) {
        return {
          ...snapshot,
          predicate_satisfied: false,
          terminal_seen: true,
          filtered_event_count: latestEvents.length,
        };
      }
      const remainingMs = timeoutMs - (now() - startedAt);
      if (remainingMs <= 0) break;
      await sleep(Math.min(pollMs, remainingMs));
    } catch (error) {
      if (error instanceof NativeRuntimeBlocker) throw error;
      throw new NativeRuntimeBlocker("projection_convergence_failed", {
        label,
        turn_id: turnId,
        error_code: error instanceof ApiError ? error.status : "transport_failure",
      });
    }
  }
  throw new NativeRuntimeBlocker("projection_convergence_timeout", {
    label,
    turn_id: turnId,
    terminal_seen: latestEvents.some(
      (event) =>
        event.turn_id === turnId && event.event_type === "codex.turn.completed",
    ),
    last_raw_event_count: lastRawEventCount,
    last_sequence: lastSequence,
  });
}

async function diagnosticTimeline(record) {
  const events = record?.task?.id
    ? await taskEventsSnapshot(record.task.id)
        .then((snapshot) => snapshot.events)
        .catch(() => [])
    : [];
  const agents = record?.run?.id
    ? await api("/runs/" + record.run.id + "/agents").catch(() => [])
    : [];
  return safeTimeline(
    Array.isArray(events) ? events : [],
    state.proxy?.rounds ?? [],
    Array.isArray(agents) ? agents : [],
  );
}

async function attachTimeline(error, record) {
  if (!(error instanceof NativeRuntimeBlocker)) return error;
  const timeline = await diagnosticTimeline(record);
  error.details = {
    ...(error.details ?? {}),
    run_id: record?.run?.id,
    timeline,
  };
  return error;
}

function turnFailureEvent(events, turnId) {
  return events.find((event) => {
    if (event.turn_id !== turnId) return false;
    const data = eventData(event);
    return (
      event.event_type === "codex.thread.failed" ||
      data.failureReason !== undefined ||
      data.artifactDelivery?.state === "failed" ||
      (event.event_type === "codex.turn.completed" && data.error !== undefined)
    );
  });
}

async function answerPendingUserInputs(runId, answeredIds) {
  const requests = await api("/runs/" + runId + "/user-input-requests");
  for (const request of requests) {
    if (answeredIds.has(request.id)) continue;
    assert.equal(request.source?.kind, "root", "business input must be requested by Root");
    assert(
      Array.isArray(request.questions) &&
        request.questions.length >= 1 &&
        request.questions.length <= 3,
      "user input must contain one to three questions",
    );
    const answers = {};
    for (const question of request.questions) {
      assert(
        Array.isArray(question.options) &&
          question.options.length >= 2 &&
          question.options.length <= 3,
        "every user input question must have two or three options",
      );
      const selected = question.options[0]?.label;
      assert.match(selected ?? "", /\(Recommended\)$/);
      answers[question.id] = { answers: [selected] };
    }
    await api("/approvals/" + request.id + "/user-input", {
      method: "POST",
      body: { answers, version: request.version },
    });
    answeredIds.add(request.id);
    state.userInputInteractions.push({
      run_id: runId,
      request_id: request.id,
      source: request.source,
      question_ids: request.questions.map((question) => question.id),
      selected_labels: Object.values(answers).flatMap((answer) => answer.answers),
    });
  }
}

async function waitForTurn(taskId, turnId, options = 300_000) {
  const settings =
    typeof options === "number" ? { timeoutMs: options } : options;
  const timeoutMs = settings.timeoutMs ?? 300_000;
  const answeredUserInputIds = new Set();
  try {
    return await eventually(
      async () => {
        const snapshot = await taskEventsSnapshot(taskId);
        const events = snapshot.events;
        if (settings.autoAnswerUserInput && settings.runId) {
          await answerPendingUserInputs(settings.runId, answeredUserInputIds);
        }
        const failure = turnFailureEvent(events, turnId);
        if (failure) {
          const failureData = eventData(failure);
          throw new NativeRuntimeBlocker("provider_or_copilot_turn_failed", {
            turn_id: turnId,
            event_type: failure.event_type,
            canonical_item_type: failure.payload?.itemType,
            error_code:
              failureData.error?.code ??
              failureData.error?.codexErrorInfo ??
              failureData.code ??
              "unknown",
          });
        }
        const approval = events.find(
          (event) => {
            if (event.turn_id !== turnId || event.event_type !== "platform.approval.requested") {
              return false;
            }
            const approvalId = String(
              eventData(event).approvalId ?? event.item_id ?? "",
            );
            return !answeredUserInputIds.has(approvalId);
          },
        );
        if (approval) {
          throw new NativeRuntimeBlocker("approval_required", {
            canonical_item_id: approval.item_id,
            canonical_item_type: approval.payload?.itemType,
          });
        }
        return events.some(
          (event) => event.turn_id === turnId && event.event_type === "codex.turn.completed",
        )
          ? events
          : undefined;
      },
      "Real DeepSeek Turn completion",
      timeoutMs,
      500,
    );
  } catch (error) {
    if (error instanceof NativeRuntimeBlocker) throw error;
    const snapshot = await taskEventsSnapshot(taskId).catch(() => ({ events: [] }));
    const events = snapshot.events;
    throw new NativeRuntimeBlocker("provider_or_copilot_turn_incomplete", {
      turn_id: turnId,
      reason: "turn_completion_timeout",
      canonical_items: eventSummary(events).filter((event) => event.turn_id === turnId),
    });
  }
}

async function cleanupRun(runId) {
  if (!runId) return;
  const rememberCleanupError = (label, error) => {
    if (error instanceof ApiError && error.status === 404) return;
    state.cleanupErrors.push(label + ": " + boundedAssistantSummary(error?.message ?? error));
  };
  try {
    const run = await api("/runs/" + runId);
    if (["running", "queued", "recovery_pending"].includes(run.status)) {
      await api("/runs/" + runId + "/cancel", { method: "POST" });
    }
  } catch (error) {
    rememberCleanupError("inspect/cancel run", error);
  }
  await eventually(
    async () => {
      try {
        const run = await api("/runs/" + runId);
        return !["running", "queued", "recovery_pending"].includes(run.status);
      } catch {
        return true;
      }
    },
    "Run cleanup",
    30_000,
    500,
  ).catch((error) => rememberCleanupError("wait for run cleanup", error));
  await eventually(
    async () => {
      try {
        const agents = await api("/runs/" + runId + "/agents");
        return agents.every((agent) =>
          !/active|running|working|starting|queued/i.test(
            String(agent.status_type ?? agent.statusType ?? agent.status ?? ""),
          ),
        );
      } catch {
        return true;
      }
    },
    "Agent cleanup",
    30_000,
    500,
  ).catch((error) => rememberCleanupError("wait for agent cleanup", error));
}

async function cleanupCase(record) {
  await cleanupRun(record?.run?.id);
  if (record?.workspace?.id) {
    await api("/workspaces/" + record.workspace.id, { method: "DELETE" }).catch(
      (error) => {
        if (!(error instanceof ApiError && error.status === 404)) {
          state.cleanupErrors.push(
            "delete workspace: " +
              (boundedAssistantSummary(error?.message ?? error) ?? "unavailable"),
          );
        }
      },
    );
  }
  if (record?.project?.id) {
    await api("/projects/" + record.project.id, { method: "DELETE" }).catch(
      (error) => {
        if (!(error instanceof ApiError && error.status === 404)) {
          state.cleanupErrors.push(
            "delete project: " +
              (boundedAssistantSummary(error?.message ?? error) ?? "unavailable"),
          );
        }
      },
    );
  }
}

let taskProviderId;

async function runToolSearchGate(provider) {
  const record = await createTaskAndRun(provider.id, "Real DeepSeek D2 tool-search " + stamp);
  try {
    const roundStart = state.proxy.rounds.length;
    const response = await send(record.task.id, true);
    assert(response.turn_id);
    const firstRound = await eventually(
      async () => state.proxy.rounds.find(
        (round) => round.round > roundStart && round.wire_api === "chat",
      ),
      "D2 first real Chat request",
      300_000,
      250,
    );
    const firstEvidence = {
      round: firstRound.round,
      tools_present: firstRound.tools_present,
      tool_count: firstRound.tool_count,
      tool_choice: firstRound.tool_choice,
      original_tool_choice: firstRound.original_tool_choice,
      effective_tool_choice: firstRound.effective_tool_choice,
      tool_choice_overridden: firstRound.tool_choice_overridden,
      current_turn_has_structured_tool_activity:
        firstRound.current_turn_has_structured_tool_activity,
      visible_tool_names: firstRound.visible_tool_names,
      structured_tool_calls: firstRound.structured_tool_calls,
      wire_tool_names: firstRound.wire_tool_names,
      model_supports_native_tool_search:
        firstRound.visible_tool_names.includes("tool_search"),
    };
    const firstSnapshot = await taskEventsSnapshot(record.task.id).catch(() => ({ events: [] }));
    const firstEvents = firstSnapshot.events;
    log("[D2 ROUND 1] " + JSON.stringify(firstEvidence));
    if (!firstEvidence.tools_present || !firstEvidence.model_supports_native_tool_search) {
      throw new NativeRuntimeBlocker("model_tool_search_capability_not_applied", {
        provider_id: provider.id,
        model,
        evidence: firstEvidence,
        canonical_items: eventSummary(firstEvents).filter(
          (event) => event.turn_id === response.turn_id,
        ),
      });
    }
    if (!firstEvidence.structured_tool_calls) {
      throw new NativeRuntimeBlocker("provider_tool_call_not_produced", {
        provider_id: provider.id,
        model,
        stage: "tool_search",
        evidence: firstEvidence,
        canonical_items: eventSummary(firstEvents).filter(
          (event) => event.turn_id === response.turn_id,
        ),
      });
    }
    if (!firstEvidence.wire_tool_names.includes("tool_search")) {
      throw new NativeRuntimeBlocker("provider_tool_search_not_called", {
        provider_id: provider.id,
        model,
        evidence: firstEvidence,
        canonical_items: eventSummary(firstEvents).filter(
          (event) => event.turn_id === response.turn_id,
        ),
      });
    }

    const secondRound = await eventually(
      async () =>
        state.proxy.rounds.find(
          (round) => round.wire_api === "chat" && round.round > firstRound.round,
        ),
      "D2 spawn_agent follow-up request",
      120_000,
      250,
    ).catch(() => undefined);
    if (!secondRound) {
      const snapshot = await taskEventsSnapshot(record.task.id).catch(() => ({ events: [] }));
      const events = snapshot.events;
      throw new NativeRuntimeBlocker("provider_spawn_agent_call_not_produced", {
        provider_id: provider.id,
        model,
        evidence: firstEvidence,
        canonical_items: eventSummary(events).filter(
          (event) => event.turn_id === response.turn_id,
        ),
      });
    }
    const secondEvidence = {
      round: secondRound.round,
      tools_present: secondRound.tools_present,
      tool_count: secondRound.tool_count,
      tool_choice: secondRound.tool_choice,
      original_tool_choice: secondRound.original_tool_choice,
      effective_tool_choice: secondRound.effective_tool_choice,
      tool_choice_overridden: secondRound.tool_choice_overridden,
      current_turn_has_structured_tool_activity:
        secondRound.current_turn_has_structured_tool_activity,
      visible_tool_names: secondRound.visible_tool_names,
      structured_tool_calls: secondRound.structured_tool_calls,
      wire_tool_names: secondRound.wire_tool_names,
    };
    log("[D2 ROUND 2] " + JSON.stringify(secondEvidence));
    await waitForTurn(record.task.id, response.turn_id);
    const projection = await awaitProjectionConvergence(record.task.id, {
      label: "D2 canonical spawnAgent projection",
      turnId: response.turn_id,
      predicate: (current) => hasCompletedSpawnAgent(current),
    });
    const events = projection.events;
    const nativeNames = nativeToolNames(events);
    log(
      "[D2 CANONICAL] " +
        JSON.stringify({
          native_tool_names: nativeNames,
          items: eventSummary(events).filter((event) => event.turn_id === response.turn_id),
        }),
    );
    logTimeline("D2 ACTOR TIMELINE", await diagnosticTimeline(record));
    if (!secondEvidence.structured_tool_calls) {
      throw new NativeRuntimeBlocker("provider_tool_call_not_produced", {
        provider_id: provider.id,
        model,
        stage: "spawn_agent",
        evidence: secondEvidence,
        native_tool_names: nativeNames,
      });
    }
    if (!hasCompletedSpawnAgent(events)) {
      throw new NativeRuntimeBlocker("provider_spawn_agent_call_not_produced", {
        provider_id: provider.id,
        model,
        evidence: secondEvidence,
        native_tool_names: nativeNames,
      });
    }
    return {
      status: "passed",
      provider_id: provider.id,
      model,
      run_id: record.run?.id,
      round_count: state.proxy.rounds.length,
      native_tool_names: nativeNames,
    };
  } catch (error) {
    throw await attachTimeline(error, record);
  } finally {
    await cleanupCase(record);
  }
}

async function runGate(provider) {
  const record = await createTaskAndRun(provider.id, "Real DeepSeek 12h network " + stamp);
  const roundStart = state.proxy.rounds.length;
  try {
    const response = await send(record.task.id);
    assert(response.turn_id);
    const firstRound = await eventually(
      async () => state.proxy.rounds.find(
        (round) => round.round > roundStart && round.wire_api === "chat",
      ),
      "first real Chat request",
      180_000,
      250,
    );
    const minimalEvidence = {
      round: firstRound.round,
      tools_present: firstRound.tools_present,
      tool_count: firstRound.tool_count,
      tool_choice: firstRound.tool_choice,
      original_tool_choice: firstRound.original_tool_choice,
      effective_tool_choice: firstRound.effective_tool_choice,
      tool_choice_overridden: firstRound.tool_choice_overridden,
      current_turn_has_structured_tool_activity:
        firstRound.current_turn_has_structured_tool_activity,
      visible_tool_names: firstRound.visible_tool_names,
      structured_tool_calls: firstRound.structured_tool_calls,
      wire_tool_names: firstRound.wire_tool_names,
      model_supports_native_tool_search:
        firstRound.visible_tool_names.includes("tool_search"),
    };
    log("[ROUND 1] " + JSON.stringify(minimalEvidence));

    await waitForTurn(record.task.id, response.turn_id, {
      timeoutMs: 600_000,
      runId: record.run.id,
      autoAnswerUserInput: true,
    });
    const projection = await awaitProjectionConvergence(record.task.id, {
      label: "multi-agent typed business projection",
      turnId: response.turn_id,
      predicate: (current) =>
        [
          "prepare_network_input",
          "evaluate_network_baseline",
          "assess_facility_change",
          "create_network_map_card",
        ].every((toolName) => hasCompletedStructuredTool(current, toolName)),
    });
    const events = projection.events;
    const allEvents = projection.events;
    const nativeNames = nativeToolNames(allEvents);
    const userInputInteractions = state.userInputInteractions.filter(
      (interaction) => interaction.run_id === record.run.id,
    );
    if (
      userInputInteractions.length < 1 ||
      !userInputInteractions.some((interaction) =>
        interaction.question_ids.includes("route_completion"),
      )
    ) {
      throw new NativeRuntimeBlocker("route_gap_user_input_not_projected", {
        provider_id: provider.id,
        model,
        interactions: userInputInteractions,
      });
    }
    const rounds = state.proxy.rounds
      .filter((round) => round.round > roundStart)
      .slice(-40)
      .map((round) => ({
        round: round.round,
        tools_present: round.tools_present,
        visible_tool_count: round.tool_count,
        tool_choice: round.tool_choice,
        original_tool_choice: round.original_tool_choice,
        effective_tool_choice: round.effective_tool_choice,
        tool_choice_overridden: round.tool_choice_overridden,
        current_turn_has_structured_tool_activity:
          round.current_turn_has_structured_tool_activity,
        structured_tool_calls: round.structured_tool_calls,
        wire_tool_names: round.wire_tool_names,
        invalid_wire_tool_names: round.invalid_wire_tool_names,
        http_status: round.http_status,
        response_terminal: round.response_terminal,
      }));
    assertProviderTransportSafety(
      state.proxy.rounds.filter((round) => round.round > roundStart),
      provider.id,
    );
    log("[ROUNDS] " + JSON.stringify(rounds));
    log(
      "[CANONICAL] " +
        JSON.stringify({
          native_tool_names: nativeNames,
          terminal_events: eventSummary(events)
            .filter((event) => /completed|failed|cancelled|error/i.test(event.event_type ?? ""))
            .slice(-80),
        }),
    );
    const dataHandoffEvent = allEvents.find(
      (event) =>
        /(prepare_network_input|resolve_prepared_network_input)/.test(String(eventTool(event))) &&
        event.event_type === "codex.item.completed" &&
        eventData(event)?.status === "completed" &&
        ["ready", "prepared_ready"].includes(
          eventData(event)?.result?.structuredContent?.outcome,
        ),
    );
    const dataHandoff = dataHandoffEvent
      ? eventData(dataHandoffEvent)?.result?.structuredContent
      : undefined;
    const baselineEvent = allEvents.find(
      (event) =>
        /evaluate_network_baseline/.test(String(eventTool(event))) &&
        event.event_type === "codex.item.completed" &&
        eventData(event)?.status === "completed",
    );
    const baseline = baselineEvent
      ? eventData(baselineEvent)?.result?.structuredContent
      : undefined;
    const baselineCoverage = Array.isArray(baseline?.coverage_metrics)
      ? baseline.coverage_metrics.find((entry) => entry?.target_hours === serviceTargetHours)
      : undefined;
    const agents = await api("/runs/" + encodeURIComponent(record.run.id) + "/agents");
    const roles = new Set(
      (Array.isArray(agents) ? agents : []).map(
        (agent) => agent.agentRole ?? agent.agent_role,
      ),
    );
    const facilityChange = allEvents.find(
      (event) =>
        eventTool(event) === "assess_facility_change" &&
        event.event_type === "codex.item.completed" &&
        eventData(event)?.status === "completed",
    );
    const facilityChangeResult = facilityChangeMatches({
      event: facilityChange,
      eventData,
      facilityChangeEvidence,
      candidateWarehouseId: balikpapanCandidateWarehouseId,
      serviceTargetHours,
    });
    const timeline = await diagnosticTimeline(record);
    logTimeline("ACTOR TIMELINE", timeline);
    const mapProducerEvent = allEvents.find(
      (event) =>
        /create_network_map_card/.test(String(eventTool(event))) &&
        event.event_type === "codex.item.completed" &&
        eventData(event)?.status === "completed",
    );
    const rootFinalMapEmbed = hasStandaloneMapEmbedInFinalRootMessage(
      allEvents,
      record.run.codex_thread_id,
    );
    const workspaceFiles = await api(
      "/workspaces/" + encodeURIComponent(record.workspace.id) + "/files",
    );
    const generatedWorkspaceFiles = workspaceFiles.filter(
      (relativePath) => !relativePath.startsWith("mock_data/"),
    );
    const businessValidation = validateBusinessEvidence({
      dataHandoff,
      baseline,
      baselineCoverage,
      roles,
      facilityChangeValid: facilityChangeResult.valid,
      facilityChangeDetails: facilityChangeResult.evidence,
      mapProducerCompleted:
        mapProducerEvent?.event_type === "codex.item.completed" &&
        eventData(mapProducerEvent)?.status === "completed",
      rootFinalMapEmbed,
      generatedWorkspaceFiles,
    });
    if (!businessValidation.valid) {
      throw new NativeRuntimeBlocker(businessValidation.code, {
        provider_id: provider.id,
        native_tool_names: nativeNames,
        data_handoff: safeStructuredSummary(dataHandoff),
        baseline: safeStructuredSummary(baseline),
        ...(businessValidation.details ?? {}),
      });
    }
    return {
      status: "passed",
      provider_id: provider.id,
      model,
      run_id: record.run?.id,
      map_producer_item_id: mapProducerEvent.item_id,
      map_producer_thread_id: mapProducerEvent.thread_id,
      map_producer_turn_id: mapProducerEvent.turn_id,
      map_producer_status: mapProducerEvent.payload?.data?.status,
      root_final_map_embed: rootFinalMapEmbed,
      user_input_count: userInputInteractions.length,
      round_count: rounds.length,
      native_tool_names: nativeNames,
    };
  } catch (error) {
    const invalidRound = invalidWireToolRound(
      state.proxy.rounds.filter((round) => round.round > roundStart),
    );
    const classified = invalidRound && error?.code !== "provider_tool_call_not_visible"
      ? new NativeRuntimeBlocker("provider_tool_call_not_visible", {
          provider_id: provider.id,
          model,
          stage: "current_turn",
          round: {
            round: invalidRound.round,
            visible_tool_count: invalidRound.tool_count,
            visible_tool_names: invalidRound.visible_tool_names,
            wire_tool_names: invalidRound.wire_tool_names,
            invalid_wire_tool_names: invalidRound.invalid_wire_tool_names,
            original_tool_choice: invalidRound.original_tool_choice,
            effective_tool_choice: invalidRound.effective_tool_choice,
          },
        })
      : error;
    throw await attachTimeline(classified, record);
  } finally {
    await cleanupCase(record);
  }
}

async function runSingleAgentGate(provider) {
  const record = await createTaskAndRun(
    provider.id,
    "Real DeepSeek single-agent 90pct network " + stamp,
  );
  const roundStart = state.proxy.rounds.length;
  try {
    const response = await send(record.task.id);
    assert(response.turn_id);
    const firstRound = await eventually(
      async () =>
        state.proxy.rounds.find(
          (round) => round.round > roundStart && round.wire_api === "chat",
        ),
      "first real single-Agent Chat request",
      180_000,
      250,
    );
    await waitForTurn(record.task.id, response.turn_id, 600_000);
    const projection = await awaitProjectionConvergence(record.task.id, {
      label: "single-agent typed business projection",
      turnId: response.turn_id,
      predicate: (current) =>
        hasCompletedStructuredTool(current, "plan_cost_matrix") &&
        hasCompletedStructuredTool(current, "solve_p_median"),
    });
    const allEvents = projection.events;
    const rounds = state.proxy.rounds
      .filter((round) => round.round > roundStart)
      .slice(-80)
      .map((round) => ({
        round: round.round,
        tools_present: round.tools_present,
        visible_tool_count: round.tool_count,
        tool_choice: round.tool_choice,
        structured_tool_calls: round.structured_tool_calls,
        wire_tool_names: round.wire_tool_names,
        invalid_wire_tool_names: round.invalid_wire_tool_names,
        http_status: round.http_status,
        response_terminal: round.response_terminal,
      }));
    assertProviderTransportSafety(
      state.proxy.rounds.filter((round) => round.round > roundStart),
      provider.id,
    );
    const timeline = await diagnosticTimeline(record);
    logTimeline("SINGLE-AGENT TIMELINE", timeline);
    const nativeNames = nativeToolNames(allEvents);
    if (!hasNoChildCollaboration({
      collaboration: timeline.collaboration,
      nativeToolNames: nativeNames,
    })) {
      throw new NativeRuntimeBlocker("single_agent_created_child", {
        provider_id: provider.id,
        native_tool_names: nativeNames,
        collaboration: timeline.collaboration,
      });
    }

    const costEvent = allEvents.find(
      (event) =>
        /plan_cost_matrix/.test(String(eventTool(event))) &&
        event.event_type === "codex.item.completed" &&
        eventData(event)?.status === "completed",
    );
    const costResult = costEvent ? eventData(costEvent)?.result?.structuredContent : undefined;
    if (
      !costEvent ||
      costResult?.calculation_rule_source !== "observed_quote_mean" ||
      costResult?.calculation_rule_evidence_path === undefined ||
      eventData(costEvent)?.arguments?.cost_policy?.kind !== "observed_quote_mean"
    ) {
      throw new NativeRuntimeBlocker("single_agent_typed_mean_cost_not_completed", {
        provider_id: provider.id,
        cost_result: safeStructuredSummary(costResult),
        native_tool_names: nativeNames,
      });
    }

    const solveEvents = allEvents.filter(
      (event) =>
        /solve_p_median/.test(String(eventTool(event))) &&
        event.event_type === "codex.item.completed" &&
        eventData(event)?.status === "completed",
    );
    const successfulSolve = solveEvents.find((event) => {
      const result = eventData(event)?.result?.structuredContent;
      return isMinimumFeasibleSolution(result, serviceTargetHours);
    });
    const solveResult = successfulSolve
      ? eventData(successfulSolve)?.result?.structuredContent
      : undefined;
    if (!successfulSolve) {
      throw new NativeRuntimeBlocker("single_agent_90pct_solution_not_completed", {
        provider_id: provider.id,
        solve_results: solveEvents.map((event) => {
          const result = eventData(event)?.result?.structuredContent;
          return {
            status: result?.status,
            opened_candidate_ids: result?.opened_candidate_ids,
            coverage: result?.coverage,
          };
        }),
        native_tool_names: nativeNames,
      });
    }
    const solution = solveResult;
    const targetCoverage = solution.coverage.find(
      (metric) => metric.target_hours === serviceTargetHours,
    );

    const workspaceFiles = await api(
      "/workspaces/" + encodeURIComponent(record.workspace.id) + "/files",
    );
    const generatedWorkspaceFiles = workspaceFiles.filter(
      (relativePath) => !relativePath.startsWith("mock_data/"),
    );
    const calculationFiles = generatedWorkspaceFiles.filter((relativePath) =>
      relativePath.startsWith("outputs/warehouse-network/calculations/"),
    );
    const outputValidation = validateGeneratedFiles({
      generatedWorkspaceFiles,
      calculationFiles,
    });
    if (!outputValidation.valid) {
      throw new NativeRuntimeBlocker(outputValidation.code, {
        provider_id: provider.id,
        ...(outputValidation.details ?? {}),
      });
    }
    const calculationJsonPath = costResult.calculation_rule_evidence_path;
    if (
      typeof calculationJsonPath !== "string" ||
      !calculationFiles.includes(calculationJsonPath) ||
      !calculationJsonPath.endsWith(".json")
    ) {
      throw new NativeRuntimeBlocker("single_agent_typed_evidence_path_invalid", {
        provider_id: provider.id,
        evidence_path: calculationJsonPath,
        calculation_files: calculationFiles,
      });
    }
    const calculationResponse = await api(
      "/workspaces/" +
        encodeURIComponent(record.workspace.id) +
        "/files/content?path=" +
        encodeURIComponent(calculationJsonPath),
    );
    if (calculationResponse.truncated) {
      throw new NativeRuntimeBlocker("single_agent_calculation_evidence_truncated", {
        provider_id: provider.id,
        path: calculationJsonPath,
      });
    }
    let calculation;
    try {
      calculation = JSON.parse(calculationResponse.content);
    } catch {
      throw new NativeRuntimeBlocker("single_agent_calculation_evidence_invalid", {
        provider_id: provider.id,
        path: calculationJsonPath,
      });
    }
    const evidenceValidation = validateQuoteMeanEvidence({
      calculation,
      costResult,
      calculationJsonPath,
      calculationFiles,
      canonicalJson,
    });
    if (!evidenceValidation.valid) {
      throw new NativeRuntimeBlocker(evidenceValidation.code, {
        provider_id: provider.id,
        ...(evidenceValidation.details ?? {}),
        cost_result: safeStructuredSummary(costResult),
      });
    }
    return {
      status: "passed",
      provider_id: provider.id,
      model,
      run_id: record.run?.id,
      round_count: rounds.length,
      native_tool_names: nativeNames,
      opened_candidate_ids: solution.opened_candidate_ids,
      demand_weighted_12h_coverage_rate:
        targetCoverage.demand_weighted_coverage_rate,
      calculation_files: calculationFiles,
      calculation_quote_count: 580,
    };
  } catch (error) {
    throw await attachTimeline(error, record);
  } finally {
    await cleanupCase(record);
  }
}

async function main() {
  if (process.env.E2E_REAL_DEEPSEEK_SELF_TEST === "1") {
    await runSelfTests();
    return;
  }
  if (process.env.E2E_REAL_DEEPSEEK !== "1") {
    log("[SKIP] real DeepSeek gate is opt-in; set E2E_REAL_DEEPSEEK=1");
    return;
  }
  if (!new Set(["observe", "force_first_tool"]).has(toolChoiceMode)) {
    throw new NativeRuntimeBlocker("invalid_tool_choice_mode", {
      mode: toolChoiceMode,
      supported: ["observe", "force_first_tool"],
    });
  }
  if (!new Set(["multi-business", "single-business", "tool-capability"]).has(entry)) {
    throw new NativeRuntimeBlocker("invalid_real_deepseek_entry", {
      entry,
      supported: ["multi-business", "single-business", "tool-capability"],
    });
  }
  state.manifest = await readFixtureManifest();
  state.proxy = await new DeepSeekProbeProxy(toolChoiceMode).start();
  let provider;
  try {
    const version = await ensureAuthenticated();
    await ensureCopilotActive();
    if (!isSingleAgentScenario) await enableMultiAgent();
    provider = await configureTemporaryProvider();
    taskProviderId = provider.id;
    state.proxy.bindExactProviderModel(provider.id, model);
    log(
      "[TOOL CHOICE POLICY] " +
        JSON.stringify({
          provider_id: provider.id,
          model,
          mode: toolChoiceMode,
        }),
    );
    log(
      "server=" +
        version +
        " fixture_files=" +
        state.manifest.files.length +
        " provider=" +
        provider.id,
    );
    const started = Date.now();
    try {
      if (isToolCapabilityGate) {
        const minimal = await runToolSearchGate(provider);
        results.push({
          name: "real DeepSeek D2 native tool gate",
          ...minimal,
          durationMs: Date.now() - started,
        });
        log("[PASS] real DeepSeek D2 native tool gate");
      } else {
        const details = isSingleAgentScenario
          ? await runSingleAgentGate(provider)
          : await runGate(provider);
        const name = isSingleAgentScenario
          ? "real DeepSeek single-agent 90pct gate"
          : "real DeepSeek multi-agent gate";
        results.push({ name, ...details, durationMs: Date.now() - started });
        log("[PASS] " + name);
      }
    } catch (error) {
      if (error instanceof NativeRuntimeBlocker) {
        const name = isToolCapabilityGate
          ? "real DeepSeek D2 native tool gate"
          : isSingleAgentScenario
            ? "real DeepSeek single-agent 90pct gate"
            : "real DeepSeek multi-agent gate";
        results.push({
          name,
          status: "typed_failure",
          code: error.code,
          details: error.details,
          durationMs: Date.now() - started,
        });
        throw error;
      }
      throw error;
    }
  } finally {
    await removeTemporaryProvider(provider);
    await state.proxy.close();
  }
  if (state.cleanupErrors.length > 0) {
    throw new NativeRuntimeBlocker("cleanup_incomplete", {
      errors: state.cleanupErrors,
    });
  }
  log("Real DeepSeek E2E summary");
  for (const result of results) log("- " + JSON.stringify(result));
}

main().catch((error) => {
  if (error instanceof NativeRuntimeBlocker) {
    const details = error.details ?? {};
    const { timeline, ...summary } = details;
    if (state.cleanupErrors.length > 0) {
      summary.cleanup_errors = state.cleanupErrors;
    }
    log(
      "[TYPED_FAILURE] " +
        JSON.stringify({ code: error.code, details: summary }),
    );
    if (timeline) logTimeline("FAILURE TIMELINE", timeline);
    process.exitCode = 2;
  } else if (error instanceof ApiError) {
    const code =
      error.status === 401 || error.status === 403
        ? "platform_authorization_failure"
        : error.status >= 500
          ? "platform_server_failure"
          : "platform_api_failure";
    log(
      "[TYPED_FAILURE] " +
        JSON.stringify({
          code,
          details: {
            status: error.status,
            method: error.method,
            pathname: error.pathname,
          },
        }),
    );
    process.exitCode = 2;
  } else if (error?.name === "TypeError" && /fetch failed/i.test(String(error.message))) {
    log(
      "[TYPED_FAILURE] " +
        JSON.stringify({
          code: "platform_transport_failure",
          details: { reason: "fetch_failed" },
        }),
    );
    process.exitCode = 2;
  } else {
    log("[FAIL] " + (boundedAssistantSummary(error?.stack) ?? "unavailable"));
    process.exitCode = 1;
  }
});
