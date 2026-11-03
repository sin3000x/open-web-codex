#!/usr/bin/env node

// Deterministic native Runtime acceptance gate for warehouse-network-copilot.
// The model server below only returns structured Chat tool calls. Agent/Role,
// Skill, MCP, Workspace, domain computation, Resource and map delivery all run
// through the real Platform and Codex Runtime.

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
  process.env.E2E_COPILOT_PACKAGE_ID ?? "warehouse-network-copilot";
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
        "必须走真实 native multi-agent 协同：先 spawn data_agent 清理 raw data，",
        "再由 network_agent 使用 Workspace prepared input 计算路线、12h baseline，",
        "最后生成包含点线面和图例的 map card。不要模拟业务结果，不要使用 shell 代替 MCP。",
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

function findFirstKey(value, keys) {
  const parsed = parseStructuredText(value);
  if (parsed !== undefined) {
    return findFirstKey(parsed, keys);
  }
  if (!value || typeof value !== "object") return undefined;
  if (!Array.isArray(value)) {
    for (const key of keys) {
      if (Object.prototype.hasOwnProperty.call(value, key)) {
        return value[key];
      }
    }
    for (const child of Object.values(value)) {
      const found = findFirstKey(child, keys);
      if (found !== undefined) return found;
    }
  } else {
    for (const child of value) {
      const found = findFirstKey(child, keys);
      if (found !== undefined) return found;
    }
  }
  return undefined;
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

function findAgentId(body) {
  const value = findFirstKey(body, ["agent_id", "agentId"]);
  return typeof value === "string" ? value : undefined;
}

function findLatestAgentId(value) {
  const parsed = parseStructuredText(value);
  if (parsed !== undefined) return findLatestAgentId(parsed);
  if (!value || typeof value !== "object") return undefined;
  if (Array.isArray(value)) {
    for (const child of [...value].reverse()) {
      const found = findLatestAgentId(child);
      if (found) return found;
    }
    return undefined;
  }
  if (typeof value.agent_id === "string") return value.agent_id;
  if (typeof value.agentId === "string") return value.agentId;
  for (const child of Object.values(value).reverse()) {
    const found = findLatestAgentId(child);
    if (found) return found;
  }
  return undefined;
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
    const markerMatch =
      bodyText.match(/OWC_DETERMINISTIC_E2E:([A-Za-z0-9_-]+)/) ??
      bodyText.match(/E2E_AGENT=(?:data_agent|network_agent):([A-Za-z0-9_-]+)/);
    const runId = markerMatch?.[1] ?? "unknown";
    const reversedUserMessages = Array.isArray(body.messages)
      ? [...body.messages].reverse().filter((message) => message?.role === "user")
      : [];
    const latestUserText =
      reversedUserMessages.find((message) =>
        /E2E_AGENT=(?:data_agent|network_agent):/.test(
          typeof message?.content === "string"
            ? message.content
            : JSON.stringify(message?.content ?? ""),
        ),
      )?.content ?? reversedUserMessages[0]?.content;
    const latestUserTextString =
      typeof latestUserText === "string"
        ? latestUserText
        : JSON.stringify(latestUserText ?? "");
    const role = latestUserTextString.includes("E2E_AGENT=data_agent:" + runId)
      ? "data"
      : latestUserTextString.includes("E2E_AGENT=network_agent:" + runId)
        ? "network"
        : "root";
    const text = JSON.stringify(body);
    const isChat = request.url?.includes("/chat/completions") ?? false;
    const toolSearchOutput = chatToolSearchOutput(body);
    if (toolSearchOutput) {
      this.toolSearchOutputs.push({ runId, role, count: toolSearchOutput.length });
    }
    this.requests.push({
      path: request.url,
      runId,
      role,
      body,
      visibleTools: visibleToolNames(body),
    });
    const plan = this.responseFor(body, text, runId, role, isChat, toolSearchOutput);
    if (plan.spec) {
      this.calls.push({
        runId,
        role,
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

  responseFor(body, text, runId, role, isChat, toolSearchOutput) {
    // This Copilot explicitly declares native Multi-Agent V1. Chat flattens
    // that namespace, and the deferred schema is intentionally absent from
    // the request `tools` list, so infer neither version from wire JSON.
    const v2 = !isChat && !text.includes('"multi_agent_v1"');
    const id = (suffix) => "e2e:" + runId + ":" + suffix;
    const has = (suffix) =>
      hasCall(text, id(suffix)) ||
      this.calls.some(
        (call) => call.runId === runId && call.role === role && call.id === id(suffix),
      );
    const toolSearch = (suffix, query) => ({
      responseId: id("response:" + suffix),
      spec: toolSearchSpec(id(suffix), query),
      events: [],
    });
    const mcp = (suffix, name, args) => ({
      responseId: id("response:" + suffix),
      spec: functionSpec(id(suffix), "mcp__supply_chain_data", name, args),
      events: [],
    });
    const network = (suffix, name, args) => ({
      responseId: id("response:" + suffix),
      spec: functionSpec(id(suffix), "mcp__supply_chain", name, args),
      events: [],
    });
    const map = (suffix, name, args) => ({
      responseId: id("response:" + suffix),
      spec: functionSpec(id(suffix), "mcp__map_utils", name, args),
      events: [],
    });
    const collaboration = (suffix, name, args) => ({
      responseId: id("response:" + suffix),
      spec: functionSpec(
        id(suffix),
        v2 ? undefined : "multi_agent_v1",
        name,
        args,
      ),
      events: [],
    });
    const message = (suffix, value) => ({
      responseId: id("response:" + suffix),
      spec: undefined,
      events: [{ text: value }],
    });

    if (role === "data") {
      if (!has("data:search")) {
        return toolSearch("data:search", "inspect workspace sources");
      }
      if (toolSearchOutput?.length === 0) {
        return message(
          "data:blocked",
          "E2E_TYPED_BLOCKER=data_mcp_tools_not_discovered Data Role tool_search returned no callable Data MCP tools.",
        );
      }
      if (!has("data:inspect")) {
        return mcp("data:inspect", "inspect_workspace_sources", {
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
      if (!has("data:prepare")) {
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
        return mcp("data:prepare", "prepare_network_input", {
          inspection_identity: inspectionIdentity,
          inspected_relative_paths: inspectedRelativePaths,
          source_selections: sourceSelections(),
          country_code: "ID",
          output_relative_path: preparedOutputPath,
          administrative_catalog_relative_path: "mock_data/administrative-areas.json",
        });
      }
      const preparedPath = findPreparedPath(body) ?? preparedOutputPath;
      const identity =
        findLatestKey(
          body,
          ["input_identity"],
          (value) =>
            (value?.schema_version === "prepared_network_input.v2" ||
              value?.schemaVersion === "prepared_network_input.v2") &&
            typeof value?.content_sha256 === "string",
        ) ?? {};
      return message(
        "data:done",
        "E2E data_agent completed native cleanup. prepared_input_relative_path=" +
          preparedPath +
          " input_identity=" +
          JSON.stringify(identity),
      );
    }

    if (role === "network") {
      const preparedPath = findPreparedPath(body) ?? preparedOutputPath;
      if (!has("network:search-tools")) {
        return toolSearch(
          "network:search-tools",
          "prepare route matrix cost matrix evaluate 12 hour baseline coverage map",
        );
      }
      if (!has("network:route")) {
        return network("network:route", "prepare_route_matrix", {
          prepared_input_relative_path: preparedPath,
          route_method: "provided",
          warehouse_scope: { kind: "existing_only" },
        });
      }
      const routeRef = findResourceRef(body, "route_matrix.v3");
      if (!routeRef) throw new Error("deterministic model could not find route_matrix.v3");
      if (!has("network:baseline")) {
        return network("network:baseline", "evaluate_network_baseline", {
          prepared_input_relative_path: preparedPath,
          route_matrix_ref: routeRef,
          objective: "min_time",
          service_targets: [12],
        });
      }
      const baselineRef = findResourceRef(body, "network_baseline.v2");
      if (!baselineRef) throw new Error("deterministic model could not find network_baseline.v2");
      if (!has("network:coverage")) {
        return network("network:coverage", "prepare_network_coverage_map", {
          prepared_input_relative_path: preparedPath,
          assignment_result_ref: baselineRef,
          service_target_hours: 12,
        });
      }
      if (!has("network:search-map")) {
        return toolSearch("network:search-map", "create warehouse network map card with points lines legend");
      }
      const dataRef = findDataRef(body);
      if (!dataRef) throw new Error("deterministic model could not find coverage GeoJSON data_ref");
      if (!has("network:map")) {
        return map("network:map", "create_network_map_card", {
          title: "12 小时仓网覆盖地图",
          network_data_ref: dataRef,
        });
      }
      const embed = findMapEmbed(body);
      return message(
        "network:done",
        "network_agent completed route and coverage delivery: 12h service coverage is ready. " +
          "\n\n" + (embed || "::codex-inline-vis{artifact=\"deterministic-map\"}"),
      );
    }

    if (!has("root:search")) {
      return toolSearch("root:search", "spawn data_agent and network_agent for warehouse planning");
    }
    if (!has("root:spawn-data")) {
      const task = [
        "E2E_AGENT=data_agent:" + runId,
        "清理并核验 Workspace mock_data 文件，使用 native tool_search 后调用 Data MCP，",
        "返回精确 prepared_input_relative_path 与 input_identity。",
      ].join(" ");
      return collaboration("root:spawn-data", "spawn_agent", {
        agent_type: "data_agent",
        ...(v2
          ? { message: task, task_name: "data_agent" }
          : {
              items: [
                {
                  type: "skill",
                  name: "warehouse-data",
                  path: exactSkillPath(text, "warehouse-data"),
                },
                { type: "text", text: task },
              ],
              fork_context: false,
            }),
      });
    }
    if (!has("root:search-after-child")) {
      return toolSearch(
        "root:search-after-child",
        "wait agent child completion and continue warehouse planning",
      );
    }
    if (!has("root:wait-data")) {
      const target = findAgentId(body);
      if (!target) throw new Error("deterministic model could not find data agent id");
      return collaboration("root:wait-data", "wait_agent", v2 ? { timeout_ms: 300_000 } : {
        targets: [target],
        timeout_ms: 300_000,
      });
    }
    if (text.includes("E2E_TYPED_BLOCKER=data_mcp_tools_not_discovered")) {
      return message(
        "root:data-blocked",
        "E2E_TYPED_BLOCKER=data_mcp_tools_not_discovered Data Role could not discover its configured MCP tools.",
      );
    }
    if (!has("root:spawn-network")) {
      const preparedPath = findPreparedPath(body);
      if (!preparedPath) throw new Error("deterministic model could not find prepared input path");
      const task = [
        "E2E_AGENT=network_agent:" + runId,
        "Use this exact prepared_input_relative_path unchanged: " + preparedPath,
        "Continue with native tool_search, route matrix, 12h baseline and map delivery.",
      ].join(" ");
      return collaboration("root:spawn-network", "spawn_agent", {
        agent_type: "network_agent",
        ...(v2
          ? { message: task, task_name: "network_agent" }
          : {
              items: [
                {
                  type: "skill",
                  name: "warehouse-network-planning",
                  path: exactSkillPath(text, "warehouse-network-planning"),
                },
                {
                  type: "skill",
                  name: "warehouse-map-delivery",
                  path: exactSkillPath(text, "warehouse-map-delivery"),
                },
                { type: "text", text: task },
              ],
              fork_context: false,
            }),
      });
    }
    if (!has("root:wait-network")) {
      const target = findLatestAgentId(body);
      if (!target) throw new Error("deterministic model could not find network agent id");
      return collaboration("root:wait-network", "wait_agent", v2 ? { timeout_ms: 300_000 } : {
        targets: [target],
        timeout_ms: 300_000,
      });
    }
    const embed = findMapEmbed(body);
    return message(
      "root:done",
      "已完成真实 multi-agent 仓网规划。12h 时效达标率已计算，地图已生成。" +
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

function exactSkillPath(text, skillName) {
  const escaped = skillName.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = text.match(
    new RegExp(`(?:r\\d+|/[^"\\\\]*)/${escaped}/SKILL\\.md`),
  );
  if (!match) throw new Error("deterministic model could not find Skill path for " + skillName);
  const skillPath = match[0];
  const alias = skillPath.split("/", 1)[0];
  if (!/^r\d+$/.test(alias)) return skillPath;
  const rootMatch = text.match(
    new RegExp("- `" + alias + "` = `([^`]+)`"),
  );
  if (!rootMatch) throw new Error("deterministic model could not expand Skill root " + alias);
  return rootMatch[1].replace(/\/$/, "") + skillPath.slice(alias.length);
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
      "stale warehouse-network-copilot deactivation",
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

async function enableMultiAgent() {
  const settings = await api("/profile/agents/settings", {
    method: "PUT",
    body: {
      multiAgentEnabled: true,
      maxThreads: 4,
      maxDepth: 3,
    },
  });
  assert.equal(settings.multiAgentEnabled, true);
  return settings;
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
    const dataRequestText = JSON.stringify(
      modelRequests.find((request) => request.role === "data")?.body ?? {},
    );
    const networkRequestText = JSON.stringify(
      modelRequests.find((request) => request.role === "network")?.body ?? {},
    );
    const dataSkillContract = {
      injected: dataRequestText.includes("<name>warehouse-data</name>"),
      preparedReady: dataRequestText.includes("prepared_ready"),
      sourceSelections: dataRequestText.includes("source_selections"),
      sourceChanged: dataRequestText.includes("source_changed"),
    };
    if (!Object.values(dataSkillContract).every(Boolean)) {
      throw new NativeRuntimeBlocker("child_skill_body_not_injected", {
        role: "data_agent",
        data_skill_contract: dataSkillContract,
        spawn_input: modelCalls
          .filter((call) => call.role === "root" && call.name === "spawn_agent")
          .map((call) => ({
            has_message: typeof call.arguments?.message === "string",
            item_types: Array.isArray(call.arguments?.items)
              ? call.arguments.items.map((item) => item?.type)
              : [],
            skill_names: Array.isArray(call.arguments?.items)
              ? call.arguments.items
                  .filter((item) => item?.type === "skill")
                  .map((item) => item.name)
              : [],
          })),
        request_messages: (modelRequests.find((request) => request.role === "data")?.body
          ?.messages ?? [])
          .slice(-12)
          .map((message) => ({
            role: message?.role,
            content: String(message?.content ?? "").replace(/\s+/g, " ").slice(0, 180),
          })),
        request_inputs: (modelRequests.find((request) => request.role === "data")?.body
          ?.input ?? [])
          .slice(-20)
          .map((item) => ({
            type: item?.type,
            role: item?.role,
            content: JSON.stringify(item?.content ?? item?.text ?? "")
              .replace(/\s+/g, " ")
              .slice(0, 220),
          })),
        request_keys: Object.keys(
          modelRequests.find((request) => request.role === "data")?.body ?? {},
        ),
        request_path: modelRequests.find((request) => request.role === "data")?.path,
        request_roles: modelRequests.map((request) => ({
          role: request.role,
          path: request.path,
        })),
        canonical_items: events
          .filter((event) => /item|thread|turn/i.test(event.event_type ?? ""))
          .slice(-30)
          .map((event) => ({
            event_type: event.event_type,
            item_type: itemType(event),
            tool: eventTool(event),
            status: event.payload?.data?.status,
            message: String(
              event.payload?.data?.error?.message ??
                event.payload?.data?.message ??
                "",
            )
              .replace(/\s+/g, " ")
              .slice(0, 200),
          })),
        runtime_errors: events
          .filter(
            (event) =>
              event.event_type === "codex.unknown" ||
              /error|failed/i.test(event.event_type ?? ""),
          )
          .slice(-20)
          .map((event) => ({
            event_type: event.event_type,
            source_type: event.payload?.data?.sourceType,
            code: event.payload?.data?.code,
            message: String(
              event.payload?.data?.message ??
                event.payload?.data?.error?.message ??
                "",
            )
              .replace(/\s+/g, " ")
              .slice(0, 300),
          })),
        skill_warnings: events
          .filter((event) =>
            /skill/i.test(JSON.stringify(event.payload?.data ?? event.payload ?? {})),
          )
          .slice(-10)
          .map((event) => ({
            event_type: event.event_type,
            summary: String(event.payload?.data?.message ?? event.payload?.message ?? "")
              .replace(/\s+/g, " ")
              .slice(0, 240),
          })),
      });
    }
    const networkSkillContract = {
      injected: networkRequestText.includes("<name>warehouse-network-planning</name>"),
      minimumFeasible: networkRequestText.includes("minimum_feasible"),
    };
    const mapSkillContract = {
      injected: networkRequestText.includes("<name>warehouse-map-delivery</name>"),
      createNetworkMap: networkRequestText.includes("create_network_map_card"),
      mapSpec: networkRequestText.includes("map_spec_ref"),
    };
    if (
      !Object.values(networkSkillContract).every(Boolean) ||
      !Object.values(mapSkillContract).every(Boolean)
    ) {
      throw new NativeRuntimeBlocker("child_skill_body_not_injected", {
        role: "network_agent",
        network_skill_contract: networkSkillContract,
        map_skill_contract: mapSkillContract,
        request_text_length: networkRequestText.length,
        call_sequence: modelCalls.map((call) => ({
          role: call.role,
          name: call.name,
          id: call.id,
        })),
        request_roles: modelRequests.map((request) => request.role),
        data_request_lengths: modelRequests
          .filter((request) => request.role === "data")
          .map((request) => JSON.stringify(request.body).length),
        last_data_tail: (
          modelRequests.filter((request) => request.role === "data").at(-1)?.body?.messages ?? []
        )
          .slice(-4)
          .map((message) => ({
            role: message?.role,
            content_length: String(message?.content ?? "").length,
            content_head: String(message?.content ?? "")
              .replace(/\s+/g, " ")
              .slice(0, 180),
            content_tail: String(message?.content ?? "")
              .replace(/\s+/g, " ")
              .slice(-240),
          })),
        root_tail: (
          modelRequests.filter((request) => request.role === "root").at(-1)?.body?.messages ?? []
        )
          .slice(-12)
          .map((message) => ({
            role: message?.role,
            content: String(message?.content ?? "")
              .replace(/\s+/g, " ")
              .slice(0, 240),
          })),
        terminal_events: events
          .filter((event) => /completed|failed|cancelled|interrupted|error/i.test(event.event_type ?? ""))
          .slice(-20)
          .map((event) => ({
            event_type: event.event_type,
            item_type: itemType(event),
            tool: eventTool(event),
            status: event.payload?.data?.status,
            code: event.payload?.data?.code ?? event.payload?.data?.error?.code,
            message: String(
              event.payload?.data?.error?.message ?? event.payload?.data?.message ?? "",
            )
              .replace(/\s+/g, " ")
              .slice(0, 300),
          })),
      });
    }
    const spawnCalls = modelCalls.filter((call) => call.name === "spawn_agent");
    const expectedSingleCalls = [
      "prepare_network_input",
      "prepare_route_matrix",
      "evaluate_network_baseline",
      "prepare_network_coverage_map",
      "create_network_map_card",
    ];
    for (const toolName of expectedSingleCalls) {
      assert.equal(
        modelCalls.filter((call) => call.name === toolName).length,
        1,
        toolName + " must be called exactly once in the golden path",
      );
    }
    for (const forbiddenTool of [
      "prepare_network_geography",
      "read_mcp_resource",
      "list_mcp_resources",
      "list_mcp_resource_templates",
    ]) {
      assert.equal(
        modelCalls.filter((call) => call.name === forbiddenTool).length,
        0,
        forbiddenTool + " is not part of the golden path",
      );
    }
    assert.equal(spawnCalls.length, 2, "golden path must spawn exactly Data and Network");
    assert.deepEqual(
      spawnCalls.map((call) =>
        (call.arguments?.items ?? [])
          .filter((item) => item?.type === "skill")
          .map((item) => item.name),
      ),
      [
        ["warehouse-data"],
        [
          "warehouse-network-planning",
          "warehouse-map-delivery",
        ],
      ],
      "child spawn must use exact structured Skill selections",
    );
    const failedMcpItems = events.filter(
      (event) =>
        itemType(event) === "mcpToolCall" &&
        event.event_type === "codex.item.completed" &&
        /failed|cancelled|interrupted|error/i.test(
          String(event.payload?.data?.status ?? ""),
        ),
    );
    assert.deepEqual(failedMcpItems, [], "golden path must not contain failed MCP Items");
    const projectedCollaboration = events.some(
      (event) =>
        itemType(event) === "collabAgentToolCall" &&
        /spawn|wait/i.test(String(eventTool(event))),
    );
    if (spawnCalls.length > 1 && !projectedCollaboration) {
      throw new NativeRuntimeBlocker("chat_deferred_tool_target_unresolved", {
        expected_tool: "multi_agent_v1.spawn_agent",
        prompt_tools_after_search: modelRequests[1]?.visibleTools ?? [],
        model_calls: modelCalls,
        runtime_events: events
          .filter((event) => event.event_type === "codex.unknown" || event.event_type === "codex.item.completed")
          .map((event) => event.payload?.data ?? event.payload),
      });
    }
    const emptyDataSearch = state.modelServer.toolSearchOutputs.find(
      (output) => output.runId === runId && output.role === "data" && output.count === 0,
    );
    if (emptyDataSearch || eventText.includes("E2E_TYPED_BLOCKER=data_mcp_tools_not_discovered")) {
      throw new NativeRuntimeBlocker("child_mcp_tools_not_discovered", {
        role: "data_agent",
        tool_search_output: emptyDataSearch ?? { count: 0 },
        model_calls: modelCalls,
      });
    }
    assert(/12\s*h|12\s*小时|12-hour/i.test(eventText), "12h result was not reported");
    if (!events.some(
      (event) =>
        itemType(event) === "collabAgentToolCall" &&
        /spawn|wait/i.test(String(eventTool(event))),
    )) {
      throw new NativeRuntimeBlocker("native_collaboration_item_not_projected", {
        model_calls: modelCalls,
        spawn_call_count: spawnCalls.length,
        visible_tools_after_search: modelRequests[1]?.visibleTools ?? [],
        canonical_items: events
          .filter((event) => event.event_type === "codex.item.completed")
          .map((event) => ({
            item_type: itemType(event),
            tool: eventTool(event),
            turn_id: event.turn_id,
          })),
      });
    }
    assert(
      /data_agent/.test(eventText) && /network_agent/.test(eventText),
      "data_agent/network_agent Role provenance was not projected",
    );
    if (!events.some(
      (event) =>
        itemType(event) === "mcpToolCall" &&
        /inspect_workspace_sources|prepare_network_input|prepare_route_matrix|evaluate_network_baseline|create_network_map_card/.test(
          String(eventTool(event)),
        ),
    )) {
      throw new NativeRuntimeBlocker("domain_mcp_items_not_projected", {
        model_calls: modelCalls,
        projected_items: events
          .filter((event) => event.event_type === "codex.item.completed")
          .map((event) => ({ item_type: itemType(event), tool: eventTool(event) })),
      });
    }
    const refs = await api("/tasks/" + record.task.id + "/resource-refs");
    const schemas = new Set(refs.map((ref) => ref.resourceSchema));
    for (const schema of [
      "route_matrix.v3",
      "network_baseline.v2",
      "network_coverage_geojson.v2",
    ]) {
      assert(schemas.has(schema), "missing Resource provenance for " + schema);
    }
    assert(
      eventText.includes("prepared_input_relative_path=" + preparedOutputPath) &&
        eventText.includes('"content_sha256":"') &&
        eventText.includes('"candidate_warehouse_count":12') &&
        eventText.includes("WH-CANDIDATE-BALIKPAPAN"),
      "prepared Workspace input identity or candidate catalog was not reported",
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
      throw new NativeRuntimeBlocker("root_map_embed_not_forwarded", {
        root_thread_id: response.thread_id,
        map_producer_projected: true,
      });
    }
    const agents = await api("/runs/" + record.run.id + "/agents");
    assert(agents.length >= 3, "Runtime agent projection did not include root and two children");
    const roleByThread = new Map(
      agents.map((agent) => [
        agent.thread_id,
        agent.agent_role,
      ]),
    );
    assert(
      [...roleByThread.values()].includes("data_agent") &&
        [...roleByThread.values()].includes("network_agent"),
      "Runtime agent projection missing Role identities",
    );
    const executions = await api("/runs/" + record.run.id + "/agent-executions");
    assert(
      executions.some(
        (execution) => roleByThread.get(execution.thread_id) === "data_agent",
      ) &&
        executions.some(
          (execution) =>
            roleByThread.get(execution.thread_id) === "network_agent",
        ),
      "Runtime agent execution provenance missing child Roles",
    );
    const runtimeStatus = await eventually(
      async () => {
        const status = await api("/profile/copilots");
        const target = installationId(status, copilotPackageId);
        return target?.state === "configured" &&
          target.runtimeDiscoveredSkillIds.includes("warehouse-supervisor")
          ? target
          : undefined;
      },
      "warehouse-network-copilot Skill discovery",
      120_000,
      1_000,
    );
    assert.equal(runtimeStatus.state, "configured");
    assert(
      runtimeStatus.runtimeDiscoveredSkillIds.includes(
        "warehouse-supervisor",
      ),
    );
    const childMcpCompleted = events.some(
      (event) =>
        itemType(event) === "mcpToolCall" &&
        event.event_type === "codex.item.completed" &&
        event.thread_id !== response.thread_id &&
        event.payload?.data?.status === "completed",
    );
    assert(childMcpCompleted, "child MCP completion must be proven by a native MCP item");
    const calls = state.modelServer.calls.filter((call) => call.runId === runId);
    assert(calls.some((call) => call.name === "tool_search"), "fixture did not issue native tool_search");
    assert(
      calls.some((call) => /spawn_agent/.test(call.name)) &&
        calls.some((call) => /prepare_route_matrix/.test(call.name)) &&
        calls.some((call) => /create_network_map_card/.test(call.name)),
      "fixture did not issue the canonical multi-agent/domain/map call chain",
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
    await enableMultiAgent();
    log("server=" + version + " fixture_files=" + state.manifest.files.length);
    for (const index of [1, 2]) {
      const started = Date.now();
      log("[RUN] deterministic multi-agent gate " + index);
      const details = await runCase(index);
      results.push({
        name: "deterministic multi-agent gate " + index,
        status: "passed",
        durationMs: Date.now() - started,
        details,
      });
      log("[PASS] deterministic multi-agent gate " + index);
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
