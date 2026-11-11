#!/usr/bin/env node

// Deterministic native Runtime acceptance gate for warehouse-network-single-agent.
// The model server below only returns structured Chat tool calls. Root Skill,
// MCP, Workspace, domain computation, Resource and map delivery all run through
// the real Platform and Codex Runtime.

import assert from "node:assert/strict";
import { createHash, randomUUID } from "node:crypto";
import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(scriptDir, "../../..");
const fixtureDir = path.join(
  scriptDir,
  "fixtures",
  "warehouse-network",
  "mock_data",
);
const fixtureManifestPath = path.join(fixtureDir, "manifest.json");
const preparedOutputPath =
  "outputs/warehouse-network/prepared/prepared_network_input.json";
const baseUrl = (
  process.env.E2E_BASE_URL ?? "http://127.0.0.1:4810"
).replace(/\/$/, "");
const apiBase = baseUrl + "/api";
const providerId = process.env.E2E_PROVIDER_ID ?? "deterministic-chat-e2e";
const model = process.env.E2E_MODEL ?? "gpt-5.4";
const copilotPackageId =
  process.env.E2E_COPILOT_PACKAGE_ID ?? "warehouse-network-single-agent";
const username = process.env.E2E_ADMIN_USERNAME ?? "deterministic-e2e";
const email =
  process.env.E2E_ADMIN_EMAIL ?? "deterministic-e2e@open-web-codex.local";
const password =
  process.env.E2E_ADMIN_PASSWORD ?? "open-web-codex-deterministic-e2e";
const secrets = [password].filter(Boolean);
const stamp = new Date().toISOString().replace(/[-:.TZ]/g, "").slice(0, 14);
const results = [];
const state = {
  token: undefined,
  manifest: undefined,
  modelServer: undefined,
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
        sanitize(body),
    );
    this.status = status;
    this.body = body;
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
  let text = typeof value === "string" ? value : JSON.stringify(value);
  for (const secret of secrets) {
    text = text.split(secret).join("[redacted]");
  }
  return text;
}

function visibleToolNames(body) {
  return Array.isArray(body.tools)
    ? body.tools.map(
        (tool) =>
          tool?.function?.name ??
          tool?.name ??
          tool?.namespace ??
          tool?.type ??
          "unknown",
      )
    : [];
}

function chatToolSearchOutput(body) {
  const messages = Array.isArray(body.messages) ? body.messages : [];
  const toolMessage = [...messages].reverse().find((message) => message?.role === "tool");
  if (typeof toolMessage?.content !== "string") return undefined;
  try {
    const content = JSON.parse(toolMessage.content);
    return Array.isArray(content) ? content : undefined;
  } catch {
    return undefined;
  }
}

function log(message) {
  process.stdout.write(sanitize(message) + "\n");
}

async function api(pathname, options = {}) {
  const headers = new Headers(options.headers);
  if (state.token) headers.set("authorization", "Bearer " + state.token);
  if (options.body !== undefined) headers.set("content-type", "application/json");
  const response = await fetch(apiBase + pathname, {
    ...options,
    headers,
    body:
      options.body === undefined ? undefined : JSON.stringify(options.body),
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
  if (!response.ok) {
    throw new ApiError(options.method ?? "GET", pathname, response.status, body);
  }
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
    description +
      " timed out" +
      (lastError ? ": " + sanitize(lastError.message) : ""),
  );
}

async function readFixtureManifest() {
  const manifest = JSON.parse(await readFile(fixtureManifestPath, "utf8"));
  assert.equal(manifest.schema_version, 1);
  assert.deepEqual(manifest.excluded_files, [".DS_Store"]);
  assert.equal(manifest.files.length, 6);
  for (const entry of manifest.files) {
    const fixturePath = path.join(fixtureDir, entry.path);
    const bytes = await readFile(fixturePath);
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
  const body = new FormData();
  body.append(
    "file",
    new Blob([await readFile(fixturePath)]),
    relativePath,
  );
  const headers = new Headers();
  if (state.token) headers.set("authorization", "Bearer " + state.token);
  const response = await fetch(
    apiBase + "/workspaces/" + encodeURIComponent(workspaceId) + "/files",
    { method: "POST", headers, body },
  );
  const text = await response.text();
  if (!response.ok) {
    throw new ApiError(
      "POST",
      "/workspaces/" + workspaceId + "/files",
      response.status,
      text,
    );
  }
  return text ? JSON.parse(text) : undefined;
}

async function createTaskAndRun(project, workspace, title) {
  const task = await api("/tasks", {
    method: "POST",
    body: {
      project_id: project.id,
      workspace_id: workspace.id,
      title,
      model_provider: providerId,
      model,
      copilot_package_id: copilotPackageId,
    },
  });
  const started = await api("/tasks/" + task.id + "/runs", {
    method: "POST",
    body: {
      idempotency_key: "deterministic-e2e-" + randomUUID(),
      fork_thread_id: null,
      fork_source_run_id: null,
    },
  });
  const run = await eventually(
    async () => {
      const current = await api("/runs/" + (started.run ?? started).id);
      return current.codex_thread_id && current.workspace_id ? current : undefined;
    },
    "Run readiness",
    90_000,
    500,
  );
  return { task, run };
}

async function taskEvents(taskId) {
  return api("/tasks/" + taskId + "/events?limit=5000");
}

async function waitForTurn(taskId, turnId, timeoutMs = 300_000) {
  return eventually(
    async () => {
      const events = await taskEvents(taskId);
      const failure = events.find(
        (event) =>
          event.turn_id === turnId &&
          (event.event_type === "codex.thread.failed" ||
            event.payload?.data?.failureReason ||
            event.payload?.data?.artifactDelivery?.state === "failed"),
      );
      if (failure) {
        throw new Error("Turn failed: " + sanitize(failure.payload));
      }
      const completed = events.some(
        (event) =>
          event.turn_id === turnId &&
          event.event_type === "codex.turn.completed",
      );
      return completed ? events : undefined;
    },
    "Turn " + turnId + " completion",
    timeoutMs,
    500,
  );
}

function itemType(event) {
  return event.payload?.itemType ?? event.payload?.data?.type;
}

function itemData(event) {
  return event.payload?.data ?? {};
}

function textFromEvents(events) {
  return events.map((event) => sanitize(event.payload)).join("\n");
}

function eventTool(event) {
  const data = itemData(event);
  return (
    data.tool ??
    data.name ??
    data.data?.tool ??
    data.result?.tool ??
    ""
  );
}

async function send(taskId, marker) {
  return api("/tasks/" + taskId + "/messages", {
    method: "POST",
    body: {
      text: [
        "E2E_ROOT_MARKER=" + marker,
        "根据已上传的 mock_data 文件，计算 12 小时时效达标率并展示地图。",
        "先准备数据，再评估 12h 现状基线，最后生成包含点线面和图例的 map card。",
        "不要模拟业务结果，不要使用 shell 代替 MCP。",
      ].join("\n"),
      model,
      model_provider: providerId,
      effort: "none",
      service_tier: null,
      access_mode: "workspace-write",
      images: [],
      collaboration_mode: null,
    },
  });
}

function findLatestKey(value, keys, predicate) {
  const parsed = parseStructuredText(value);
  if (parsed !== undefined) {
    return findLatestKey(parsed, keys, predicate);
  }
  if (!value || typeof value !== "object") return undefined;
  if (Array.isArray(value)) {
    for (const child of [...value].reverse()) {
      const found = findLatestKey(child, keys, predicate);
      if (found !== undefined) return found;
    }
    return undefined;
  }
  for (const child of Object.values(value).reverse()) {
    const found = findLatestKey(child, keys, predicate);
    if (found !== undefined) return found;
  }
  for (const key of keys) {
    if (
      Object.prototype.hasOwnProperty.call(value, key) &&
      predicate(value[key])
    ) {
      return value[key];
    }
  }
  return undefined;
}

function parseStructuredText(value) {
  if (typeof value !== "string") return undefined;
  const candidates = [
    value,
    value.match(/(?:^|\n)Output:\s*([\[{][\s\S]*)\s*$/)?.[1],
  ].filter(Boolean);
  for (const candidate of candidates) {
    try {
      return JSON.parse(candidate);
    } catch {
      // Try the next canonical tool-output envelope candidate.
    }
  }
  return undefined;
}

function findResourceRef(value, schema) {
  const parsed = parseStructuredText(value);
  if (parsed !== undefined) {
    return findResourceRef(parsed, schema);
  }
  if (!value || typeof value !== "object") return undefined;
  if (!Array.isArray(value)) {
    if (
      value.resource_schema === schema &&
      typeof value.uri === "string" &&
      typeof value.server === "string"
    ) {
      return value;
    }
    for (const child of Object.values(value)) {
      const found = findResourceRef(child, schema);
      if (found) return found;
    }
  } else {
    for (const child of value) {
      const found = findResourceRef(child, schema);
      if (found) return found;
    }
  }
  return undefined;
}

function findDataRef(value) {
  const parsed = parseStructuredText(value);
  if (parsed !== undefined) {
    return findDataRef(parsed);
  }
  if (!value || typeof value !== "object") return undefined;
  if (!Array.isArray(value)) {
    if (
      typeof value.server === "string" &&
      typeof value.uri === "string" &&
      typeof value.resource_schema === "string" &&
      value.profile &&
      typeof value.profile === "object"
    ) {
      return value;
    }
    for (const child of Object.values(value)) {
      const found = findDataRef(child);
      if (found) return found;
    }
  } else {
    for (const child of value) {
      const found = findDataRef(child);
      if (found) return found;
    }
  }
  return undefined;
}

function findPreparedPath(body) {
  const value = findLatestKey(
    body,
    ["prepared_input_relative_path", "preparedInputRelativePath"],
    (candidate) => typeof candidate === "string" && candidate.length > 0,
  );
  if (typeof value === "string") return value;
  const match = JSON.stringify(body).match(
    /prepared_input_relative_path[=:]\\?"?([A-Za-z0-9_./-]+)/,
  );
  return match?.[1];
}

function findMapEmbed(body) {
  const text = JSON.stringify(body);
  const match = text.match(
    /::codex-inline-vis\{artifact="([^"]+)"\}/,
  );
  return match ? match[0] : "";
}

function rootFinalMessageHasStandaloneMapEmbed(events, rootThreadId) {
  const rootMessages = events
    .filter((event) => {
      if (event.thread_id !== rootThreadId) return false;
      const data = event.payload?.data ?? {};
      const type = itemType(event);
      return type === "agentMessage" || data.type === "message";
    })
    .map((event) => {
      const data = event.payload?.data ?? {};
      return typeof data.text === "string"
        ? data.text
        : typeof data.message === "string"
          ? data.message
          : typeof data.content === "string"
            ? data.content
            : "";
    });
  const finalMessage = rootMessages.at(-1) ?? "";
  return finalMessage
    .split(/\r?\n/)
    .some((line) => /^::codex-inline-vis\{artifact="[^"]+"\}$/.test(line.trim()));
}

function hasCall(text, callId) {
  return text.includes(callId);
}

function toolSearchSpec(callId, query) {
  return {
    id: callId,
    namespace: undefined,
    name: "tool_search",
    arguments: { query, limit: 20 },
  };
}

function functionSpec(callId, namespace, name, argumentsObject) {
  return {
    id: callId,
    namespace,
    name,
    arguments: argumentsObject,
  };
}

function responseEvents(responseId, item) {
  return [
    {
      type: "response.created",
      response: { id: responseId },
    },
    item,
    {
      type: "response.completed",
      response: {
        id: responseId,
        usage: {
          input_tokens: 0,
          input_tokens_details: null,
          output_tokens: 0,
          output_tokens_details: null,
          total_tokens: 0,
        },
      },
    },
  ];
}

function responseMessage(responseId, text) {
  return responseEvents(responseId, {
    type: "response.output_item.done",
    item: {
      type: "message",
      role: "assistant",
      id: responseId + "-message",
      content: [{ type: "output_text", text }],
    },
  });
}

function responseToolCall(responseId, spec) {
  return responseEvents(responseId, {
    type: "response.output_item.done",
    item: {
      type: spec.name === "tool_search" ? "tool_search_call" : "function_call",
      call_id: spec.id,
      ...(spec.namespace ? { namespace: spec.namespace } : {}),
      ...(spec.name === "tool_search" ? { execution: "client" } : {}),
      name: spec.name === "tool_search" ? undefined : spec.name,
      arguments: spec.arguments,
    },
  }).map((event) => {
    if (event.type === "response.output_item.done") {
      delete event.item.name;
    }
    return event;
  });
}

function chatSse(events, responseId, spec) {
  const chunks = [];
  if (spec) {
    const wireName = spec.namespace
      ? spec.namespace + "__" + spec.name
      : spec.name;
    chunks.push({
      id: responseId,
      object: "chat.completion.chunk",
      model,
      choices: [
        {
          index: 0,
          delta: {
            tool_calls: [
              {
                index: 0,
                id: spec.id,
                function: {
                  name: wireName,
                  arguments: JSON.stringify(spec.arguments),
                },
              },
            ],
          },
          finish_reason: null,
        },
      ],
    });
    chunks.push({
      id: responseId,
      object: "chat.completion.chunk",
      model,
      choices: [{ index: 0, delta: {}, finish_reason: "tool_calls" }],
    });
  } else {
    chunks.push({
      id: responseId,
      object: "chat.completion.chunk",
      model,
      choices: [
        {
          index: 0,
          delta: { content: events[0]?.text ?? "" },
          finish_reason: null,
        },
      ],
    });
    chunks.push({
      id: responseId,
      object: "chat.completion.chunk",
      model,
      choices: [{ index: 0, delta: {}, finish_reason: "stop" }],
    });
  }
  return chunks;
}

function ssePayload(events) {
  return (
    events
      .map((event) => {
        const type =
          event.type === "chat"
            ? JSON.stringify(event.payload)
            : "event: " +
              event.type +
              "\ndata: " +
              JSON.stringify(event) +
              "\n\n";
        return event.type === "chat" ? "data: " + type + "\n\n" : type;
      })
      .join("") + (events.some((event) => event.type === "chat") ? "data: [DONE]\n\n" : "")
  );
}

class DeterministicModelServer {
  constructor() {
    this.server = createServer((request, response) => {
      this.handle(request, response).catch((error) => {
        log("[MODEL_SERVER_ERROR] " + String(error?.stack ?? error));
        response.statusCode = 500;
        response.setHeader("content-type", "application/json");
        response.end(JSON.stringify({ error: String(error.message ?? error) }));
      });
    });
    this.requests = [];
    this.calls = [];
    this.toolSearchOutputs = [];
    this.address = undefined;
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

  async handle(request, response) {
    const bodyText = await new Promise((resolve, reject) => {
      const chunks = [];
      request.on("data", (chunk) => chunks.push(chunk));
      request.on("end", () => resolve(Buffer.concat(chunks).toString("utf8")));
      request.on("error", reject);
    });
    if (request.url?.endsWith("/models")) {
      const body = JSON.stringify({
        data: [{ id: model, object: "model", owned_by: "deterministic-e2e" }],
      });
      response.writeHead(200, { "content-type": "application/json" });
      response.end(body);
      return;
    }
    const body = bodyText ? JSON.parse(bodyText) : {};
    const markerMatch = bodyText.match(/OWC_DETERMINISTIC_E2E:([A-Za-z0-9_-]+)/);
    const runId = markerMatch?.[1] ?? "unknown";
    const text = JSON.stringify(body);
    const toolSearchOutput = chatToolSearchOutput(body);
    if (toolSearchOutput) {
      this.toolSearchOutputs.push({ runId, count: toolSearchOutput.length });
    }
    this.requests.push({
      path: request.url,
      runId,
      body,
      visibleTools: visibleToolNames(body),
    });
    const plan = this.responseFor(body, text, runId, toolSearchOutput);
    if (plan.spec) {
      this.calls.push({
        runId,
        namespace: plan.spec.namespace,
        name: plan.spec.name,
        id: plan.spec.id,
        arguments: plan.spec.arguments,
      });
    }
    if (request.url?.includes("/chat/completions")) {
      const events = chatSse(plan.events, plan.responseId, plan.spec);
      response.writeHead(200, {
        "content-type": "text/event-stream",
        connection: "close",
      });
      response.end(
        events
          .map((event) => "data: " + JSON.stringify(event) + "\n\n")
          .join("") + "data: [DONE]\n\n",
      );
      return;
    }
    response.writeHead(200, {
      "content-type": "text/event-stream",
      connection: "close",
    });
    response.end(ssePayload(plan.events));
  }

  responseFor(body, text, runId, toolSearchOutput) {
    const id = (suffix) => "e2e:" + runId + ":" + suffix;
    const has = (suffix) =>
      hasCall(text, id(suffix)) ||
      this.calls.some((call) => call.runId === runId && call.id === id(suffix));
    const call = (suffix, namespace, name, args) => ({
      responseId: id("response:" + suffix),
      spec: functionSpec(id(suffix), namespace, name, args),
      events: [],
    });
    const toolSearch = (suffix, query) => ({
      responseId: id("response:" + suffix),
      spec: toolSearchSpec(id(suffix), query),
      events: [],
    });
    const message = (suffix, value) => ({
      responseId: id("response:" + suffix),
      spec: undefined,
      events: [{ text: value }],
    });

    if (!has("search-data")) {
      return toolSearch("search-data", "inspect_workspace_sources prepare_network_input");
    }
    if (toolSearchOutput?.length === 0) {
      return message(
        "blocked",
        "E2E_TYPED_BLOCKER=mcp_tools_not_discovered tool_search returned no callable MCP tools.",
      );
    }
    if (!has("inspect")) {
      return call("inspect", "mcp__supply_chain_data", "inspect_workspace_sources", {
        relative_paths: [
          "mock_data/administrative-areas.json",
          "mock_data/candidate-warehouses.csv",
          "mock_data/demand-cities.csv",
          "mock_data/existing-warehouses.csv",
          "mock_data/population-snapshot.csv",
          "mock_data/route-quotes.csv",
        ],
        country_code: "ID",
      });
    }
    if (!has("prepare")) {
      const inspectionIdentity = findLatestKey(
        body,
        ["inspection_identity"],
        (value) =>
          value?.schemaVersion === "workspace_source_inspection.v2" &&
          typeof value?.content_sha256 === "string" &&
          Number.isInteger(value?.source_count),
      );
      const inspectedRelativePaths = findLatestKey(
        body,
        ["inspected_relative_paths"],
        (value) =>
          Array.isArray(value) && value.length > 0 && value.every((item) => typeof item === "string"),
      );
      if (!inspectionIdentity || !inspectedRelativePaths) {
        throw new Error("deterministic model could not find inline source inspection identity");
      }
      return call("prepare", "mcp__supply_chain_data", "prepare_network_input", {
        inspection_identity: inspectionIdentity,
        inspected_relative_paths: inspectedRelativePaths,
        source_selections: sourceSelections(),
        country_code: "ID",
        output_relative_path: preparedOutputPath,
        administrative_catalog_relative_path: "mock_data/administrative-areas.json",
      });
    }
    if (!has("search-analysis")) {
      return toolSearch(
        "search-analysis",
        "evaluate_baseline prepare_network_coverage_map create_network_map_card",
      );
    }
    // No prepared path is passed: the analysis Tool resolves the single fresh
    // prepared input itself and builds routes and costs internally.
    if (!has("baseline")) {
      return call("baseline", "mcp__supply_chain", "evaluate_baseline", {
        service_targets: [12],
      });
    }
    const baselineRef = findResourceRef(body, "network_baseline.v2");
    if (!baselineRef) throw new Error("deterministic model could not find network_baseline.v2");
    if (!has("coverage")) {
      return call("coverage", "mcp__supply_chain", "prepare_network_coverage_map", {
        prepared_input_relative_path: findPreparedPath(body) ?? preparedOutputPath,
        assignment_result_ref: baselineRef,
        service_target_hours: 12,
      });
    }
    const dataRef = findDataRef(body);
    if (!dataRef) throw new Error("deterministic model could not find coverage GeoJSON data_ref");
    if (!has("map")) {
      return call("map", "mcp__map_utils", "create_network_map_card", {
        title: "12 小时仓网覆盖地图",
        network_data_ref: dataRef,
      });
    }
    const embed = findMapEmbed(body);
    return message(
      "done",
      "已完成仓网现状评估。12h 时效达标率已计算，地图已生成。" +
        "\n\n" + (embed || "::codex-inline-vis{artifact=\"deterministic-map\"}"),
    );
  }
}

function sourceSelections() {
  return [
    {
      relative_path: "mock_data/demand-cities.csv",
      unit_ref: "table",
      role: "demand",
    },
    {
      relative_path: "mock_data/existing-warehouses.csv",
      unit_ref: "table",
      role: "existing_warehouse",
    },
    {
      relative_path: "mock_data/candidate-warehouses.csv",
      unit_ref: "table",
      role: "candidate_warehouse",
    },
    {
      relative_path: "mock_data/route-quotes.csv",
      unit_ref: "table",
      role: "route_quote",
    },
  ];
}

async function ensureAuthenticated() {
  const health = await api("/health");
  assert.equal(health.ok, true);
  let auth;
  try {
    auth = await api("/bootstrap", {
      method: "POST",
      body: { name: "Deterministic E2E Owner", username, email, password },
    });
  } catch (error) {
    if (!(error instanceof ApiError) || error.status !== 409) throw error;
    auth = await api("/sessions/local", { method: "POST" });
  }
  state.token = auth.session_token;
  const me = await api("/me");
  assert.equal(me.username, auth.user.username);
  return health.version;
}

function installationId(status, packageId) {
  return status.installations.find(
    (entry) => entry.packageId === packageId,
  );
}

async function ensureCopilotActive() {
  let status = await api("/profile/copilots");
  for (const installation of status.installations) {
    const active = installation.active;
    const packageId = installation.packageId;
    if (active && packageId !== copilotPackageId) {
      await api("/profile/copilots/deactivate", {
        method: "POST",
        body: { packageId },
      });
    }
  }
  const current = installationId(status, copilotPackageId);
  const currentState = String(current?.state ?? "").toLowerCase();
  if (current?.active && currentState !== "configured") {
    await api("/profile/copilots/deactivate", {
      method: "POST",
      body: { packageId: copilotPackageId },
    });
    await eventually(
      async () => {
        const next = await api("/profile/copilots");
        const target = installationId(next, copilotPackageId);
        return target?.active === false ? next : undefined;
      },
      "stale Copilot package deactivation",
      120_000,
      1_000,
    );
  }
  if (!current?.active || currentState !== "configured") {
    await api("/profile/copilots/activate", {
      method: "POST",
      body: { packageId: copilotPackageId },
    });
  }
  status = await api("/profile/copilots");
  const target = installationId(status, copilotPackageId);
  assert(target);
  assert.equal(target.active, true);
  assert.equal(target.state, "configured");
  return status;
}

async function configureProvider() {
  const catalog = await api("/providers/" + encodeURIComponent(providerId), {
    method: "PUT",
    body: {
      name: "Deterministic Chat E2E",
      baseUrl: state.modelServer.address + "/v1",
      wireApi: "chat",
      supportsFunctionTools: true,
      credentials: { mode: "none" },
      select: true,
    },
  });
  const provider = catalog.data.find((entry) => entry.id === providerId);
  assert(provider);
  assert.equal(provider.wireApi, "chat");
  assert.equal(provider.supportsFunctionTools, true);
  assert.equal(catalog.currentProviderId, providerId);
  return provider;
}

async function configureModelToolSearch() {
  const configured = await api(
    "/providers/" +
      encodeURIComponent(providerId) +
      "/models/" +
      encodeURIComponent(model),
    {
      method: "PATCH",
      body: {
        contextWindow: 128_000,
        supportsSearchTool: true,
      },
    },
  );
  const configuredProvider = configured.data.find((entry) => entry.id === providerId);
  const configuredModel = configuredProvider?.models?.find(
    (entry) => entry.modelId === model,
  );
  assert.equal(configuredModel?.supportsSearchTool, true);
}

async function cleanupRun(runId) {
  if (!runId) return;
  try {
    const run = await api("/runs/" + runId);
    if (["running", "queued", "recovery_pending"].includes(run.status)) {
      await api("/runs/" + runId + "/cancel", { method: "POST" });
    }
  } catch {
    // Cleanup is best effort; the caller reports the original gate failure.
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
    "Run " + runId + " terminal cleanup",
    30_000,
    500,
  ).catch(() => undefined);
  let previousAgentSnapshot;
  await eventually(
    async () => {
      try {
        const [agents, activities] = await Promise.all([
          api("/runs/" + runId + "/agents"),
          api("/runs/" + runId + "/agent-activities"),
        ]);
        const active = agents.filter((agent) =>
          ["starting", "pending", "running", "active", "in_progress", "reconnecting"]
            .includes(String(agent.status_type ?? "").toLowerCase()),
        );
        const snapshot = JSON.stringify({
          agents: agents.map((agent) => ({
            thread_id: agent.thread_id,
            status_type: agent.status_type,
          })),
          activity_count: activities.length,
        });
        const stable = snapshot === previousAgentSnapshot;
        previousAgentSnapshot = snapshot;
        return active.length === 0 && stable ? true : undefined;
      } catch {
        return undefined;
      }
    },
    "Runtime Agent projection cleanup",
    30_000,
    250,
  ).catch(() => undefined);
}

async function cleanupCase(record) {
  await cleanupRun(record.run?.id);
  if (record.workspace?.id) {
    await api("/workspaces/" + record.workspace.id, { method: "DELETE" }).catch(() => undefined);
  }
  if (record.project?.id) {
    await api("/projects/" + record.project.id, { method: "DELETE" }).catch(() => undefined);
  }
}

async function runCase(index) {
  const runId = "run" + index + "_" + stamp;
  const marker = "OWC_DETERMINISTIC_E2E:" + runId;
  const record = { project: undefined, workspace: undefined, task: undefined, run: undefined };
  try {
    record.project = await api("/projects/managed", {
      method: "POST",
      body: { name: "Deterministic warehouse E2E " + runId },
    });
    record.workspace = await api("/workspaces", {
      method: "POST",
      body: {
        project_id: record.project.id,
        idempotency_key: "deterministic-workspace-" + runId,
        kind: "main",
        name: record.project.name,
        source_ref: null,
        parent_workspace_id: null,
        copy_agents_md: false,
      },
    });
    for (const entry of state.manifest.files) {
      await uploadWorkspaceFile(
        record.workspace.id,
        path.join(fixtureDir, entry.path),
        "mock_data/" + entry.path,
      );
    }
    const files = await api("/workspaces/" + record.workspace.id + "/files");
    assert.deepEqual(
      state.manifest.files.map((entry) => "mock_data/" + entry.path).sort(),
      files.filter((entry) => entry.startsWith("mock_data/")).sort(),
    );

    const created = await createTaskAndRun(
      record.project,
      record.workspace,
      "Deterministic 12h network " + runId,
    );
    record.task = created.task;
    record.run = created.run;
    assert(record.run.codex_thread_id);
    const response = await send(record.task.id, marker);
    assert.equal(response.thread_id, record.run.codex_thread_id);
    const events = await waitForTurn(record.task.id, response.turn_id);
    const turnEvents = events.filter((event) => event.turn_id === response.turn_id);
    const eventText = textFromEvents(events);
    const modelRequests = state.modelServer.requests.filter(
      (request) => request.runId === runId,
    );
    const modelCalls = state.modelServer.calls.filter((call) => call.runId === runId);
    const rootRequestText = JSON.stringify(modelRequests[0]?.body ?? {});
    if (!rootRequestText.includes("<name>warehouse-single-agent</name>")) {
      throw new NativeRuntimeBlocker("root_skill_body_not_injected", {
        request_keys: Object.keys(modelRequests[0]?.body ?? {}),
        request_path: modelRequests[0]?.path,
      });
    }
    for (const toolName of [
      "inspect_workspace_sources",
      "prepare_network_input",
      "evaluate_baseline",
      "prepare_network_coverage_map",
      "create_network_map_card",
    ]) {
      assert.equal(
        modelCalls.filter((call) => call.name === toolName).length,
        1,
        toolName + " must be called exactly once in the golden path",
      );
    }
    assert.deepEqual(
      modelCalls.find((call) => call.name === "evaluate_baseline").arguments,
      { service_targets: [12] },
      "the baseline question must need no route, cost or prepared-input arguments",
    );
    for (const forbiddenTool of [
      "prepare_network_geography",
      "read_mcp_resource",
      "list_mcp_resources",
      "list_mcp_resource_templates",
      "spawn_agent",
      "wait_agent",
    ]) {
      assert.equal(
        modelCalls.filter((call) => call.name === forbiddenTool).length,
        0,
        forbiddenTool + " is not part of the golden path",
      );
    }
    const failedMcpItems = events.filter(
      (event) =>
        itemType(event) === "mcpToolCall" &&
        event.event_type === "codex.item.completed" &&
        /failed|cancelled|interrupted|error/i.test(
          String(event.payload?.data?.status ?? ""),
        ),
    );
    assert.deepEqual(failedMcpItems, [], "golden path must not contain failed MCP Items");
    const emptySearch = state.modelServer.toolSearchOutputs.find(
      (output) => output.runId === runId && output.count === 0,
    );
    if (emptySearch || eventText.includes("E2E_TYPED_BLOCKER=mcp_tools_not_discovered")) {
      throw new NativeRuntimeBlocker("mcp_tools_not_discovered", {
        tool_search_output: emptySearch ?? { count: 0 },
        model_calls: modelCalls,
      });
    }
    assert(/12\s*h|12\s*小时|12-hour/i.test(eventText), "12h result was not reported");
    assert(
      !events.some((event) => itemType(event) === "collabAgentToolCall"),
      "a single-Agent Copilot must not project collaboration Items",
    );
    const completedMcpTools = new Set(
      events
        .filter(
          (event) =>
            itemType(event) === "mcpToolCall" &&
            event.event_type === "codex.item.completed" &&
            event.thread_id === response.thread_id &&
            event.payload?.data?.status === "completed",
        )
        .map((event) => String(eventTool(event))),
    );
    for (const toolName of [
      "inspect_workspace_sources",
      "prepare_network_input",
      "evaluate_baseline",
      "prepare_network_coverage_map",
      "create_network_map_card",
    ]) {
      if (![...completedMcpTools].some((name) => name.includes(toolName))) {
        throw new NativeRuntimeBlocker("domain_mcp_items_not_projected", {
          missing_tool: toolName,
          projected_items: events
            .filter((event) => event.event_type === "codex.item.completed")
            .map((event) => ({ item_type: itemType(event), tool: eventTool(event) })),
        });
      }
    }
    const refs = await api("/tasks/" + record.task.id + "/resource-refs");
    const schemas = new Set(refs.map((ref) => ref.resourceSchema));
    for (const schema of ["network_baseline.v2", "network_coverage_geojson.v2"]) {
      assert(schemas.has(schema), "missing Resource provenance for " + schema);
    }
    for (const internalSchema of ["route_matrix.v3", "cost_matrix.v3"]) {
      assert(
        !schemas.has(internalSchema),
        internalSchema + " is an internal step and must not be linked from a Tool Item",
      );
    }
    assert(
      eventText.includes(preparedOutputPath) && eventText.includes("content_sha256"),
      "prepared Workspace input identity was not reported",
    );
    const mapEvent = events.find(
      (event) =>
        (event.event_type === "codex.item.completed" ||
          event.lifecycle === "completed") &&
        itemType(event) === "mcpToolCall" &&
        /create_network_map_card/.test(String(eventTool(event))),
    );
    assert(mapEvent, "map producer Tool Item was not projected");
    const mapEventText = JSON.stringify(mapEvent);
    assert(
      mapEventText.includes("map.v3") &&
        (mapEventText.includes("map_card_spec.v1") ||
          mapEventText.includes("map-card-spec-")),
      "map delivery lacked renderer and map spec provenance",
    );
    const preparedFiles = await api(
      "/workspaces/" + record.workspace.id + "/files",
    );
    assert(
      preparedFiles.includes(preparedOutputPath),
      "prepared Workspace file was not persisted",
    );
    if (!rootFinalMessageHasStandaloneMapEmbed(events, response.thread_id)) {
      throw new NativeRuntimeBlocker("root_map_embed_not_delivered", {
        root_thread_id: response.thread_id,
        map_producer_projected: true,
      });
    }
    const agents = await api("/runs/" + record.run.id + "/agents");
    assert(
      agents.every((agent) => agent.thread_id === response.thread_id),
      "a single-Agent Run must not project child Agents",
    );
    const runtimeStatus = await eventually(
      async () => {
        const status = await api("/profile/copilots");
        const target = installationId(status, copilotPackageId);
        return target?.state === "configured" &&
          target.runtimeDiscoveredSkillIds.includes("warehouse-single-agent")
          ? target
          : undefined;
      },
      "single-Agent Copilot Skill discovery",
      120_000,
      1_000,
    );
    assert.equal(runtimeStatus.state, "configured");
    assert(
      modelCalls.some((call) => call.name === "tool_search"),
      "fixture did not issue native tool_search",
    );
    return {
      runId,
      run: record.run.id,
      rootThread: response.thread_id,
      rootTurn: response.turn_id,
      eventCount: turnEvents.length,
      resourceSchemas: [...schemas].sort(),
    };
  } finally {
    await cleanupCase(record);
  }
}

async function main() {
  state.manifest = await readFixtureManifest();
  state.modelServer = await new DeterministicModelServer().start();
  try {
    const version = await ensureAuthenticated();
    await ensureCopilotActive();
    await configureProvider();
    await configureModelToolSearch();
    log("server=" + version + " fixture_files=" + state.manifest.files.length);
    for (const index of [1, 2]) {
      const started = Date.now();
      log("[RUN] deterministic single-agent gate " + index);
      const details = await runCase(index);
      results.push({
        name: "deterministic single-agent gate " + index,
        status: "passed",
        durationMs: Date.now() - started,
        details,
      });
      log("[PASS] deterministic single-agent gate " + index);
    }
  } finally {
    await state.modelServer.close();
  }
  log("Deterministic E2E summary");
  for (const result of results) {
    log("- " + result.status.toUpperCase() + " " + result.name + " " + JSON.stringify(result.details));
  }
  assert.equal(results.length, 2);
  assert(results.every((result) => result.status === "passed"));
}

main().catch((error) => {
  if (error instanceof NativeRuntimeBlocker) {
    log("[BLOCKED] " + error.message);
  } else {
    log("[FAIL] " + error.stack);
  }
  process.exitCode = 1;
});
