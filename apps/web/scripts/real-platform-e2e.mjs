#!/usr/bin/env node

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(scriptDir, "../../..");
const baseUrl = (process.env.E2E_BASE_URL ?? "http://127.0.0.1:4810").replace(/\/$/, "");
const apiBase = `${baseUrl}/api`;
const providerId = process.env.E2E_PROVIDER_ID ?? "deepseek-e2e";
const providerBaseUrl = process.env.E2E_PROVIDER_BASE_URL ?? "https://api.deepseek.com";
const model = process.env.E2E_MODEL ?? "deepseek-v4-flash";
const username = process.env.E2E_ADMIN_USERNAME ?? "real-e2e";
const email = process.env.E2E_ADMIN_EMAIL ?? "real-e2e@open-web-codex.local";
const password = process.env.E2E_ADMIN_PASSWORD ?? "open-web-codex-real-e2e";
const mcpBinary = process.env.E2E_MCP_BIN ?? path.join(
  repoRoot,
  "codex/codex-rs/target/debug/test_stdio_server",
);
const deepseekKey = await loadSecret(
  "DEEPSEEK_API_KEY",
  process.env.DEEPSEEK_API_KEY_FILE,
);

const secrets = [deepseekKey, password].filter(Boolean);
const stamp = new Date().toISOString().replace(/[-:.TZ]/g, "").slice(0, 14);
const marker = `OWC_E2E_${stamp}`;
const results = [];
const state = {
  token: undefined,
  project: undefined,
  firstTask: undefined,
  firstRun: undefined,
  firstTurnEvents: [],
  secondTask: undefined,
  secondRun: undefined,
  ws: undefined,
};

function sanitize(value) {
  let text = typeof value === "string" ? value : JSON.stringify(value);
  for (const secret of secrets) {
    if (secret) text = text.split(secret).join("[redacted]");
  }
  return text;
}

function log(message) {
  process.stdout.write(`${sanitize(message)}\n`);
}

async function loadSecret(envName, fileName) {
  if (fileName) return (await readFile(fileName, "utf8")).trim();
  if (process.env[envName]) return process.env[envName].trim();
  throw new Error(`${envName} or ${envName}_FILE is required`);
}

async function api(pathname, options = {}) {
  const headers = new Headers(options.headers);
  if (state.token) headers.set("authorization", `Bearer ${state.token}`);
  if (options.body !== undefined) headers.set("content-type", "application/json");
  const response = await fetch(`${apiBase}${pathname}`, {
    ...options,
    headers,
    body: options.body === undefined ? undefined : JSON.stringify(options.body),
  });
  const text = await response.text();
  let body = undefined;
  if (text) {
    try {
      body = JSON.parse(text);
    } catch {
      body = text;
    }
  }
  if (!response.ok) {
    throw new Error(`${options.method ?? "GET"} ${pathname} failed (${response.status}): ${sanitize(body)}`);
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
  throw new Error(`${description} timed out${lastError ? `: ${sanitize(lastError.message)}` : ""}`);
}

async function runCase(name, test) {
  const started = performance.now();
  log(`\n[RUN] ${name}`);
  try {
    const details = await test();
    const durationMs = Math.round(performance.now() - started);
    results.push({ name, status: "passed", durationMs, details });
    log(`[PASS] ${name} (${durationMs} ms)${details ? ` — ${details}` : ""}`);
  } catch (error) {
    const durationMs = Math.round(performance.now() - started);
    results.push({ name, status: "failed", durationMs, error: sanitize(error.message) });
    log(`[FAIL] ${name} (${durationMs} ms) — ${error.message}`);
    throw error;
  }
}

function findProvider(catalog, id) {
  return catalog.data.find((provider) => provider.id === id);
}

function currentProviderId(catalog) {
  return catalog.currentProviderId ?? catalog.current_provider_id;
}

async function createTaskAndRun(title) {
  const task = await api("/tasks", {
    method: "POST",
    body: { project_id: state.project.id, title },
  });
  const response = await api(`/tasks/${task.id}/runs`, {
    method: "POST",
    body: {
      idempotency_key: `real-e2e-${crypto.randomUUID()}`,
      git_ref: null,
      workspace_kind: "main",
      workspace_name: null,
      workspace_parent_run_id: null,
      workspace_group_run_id: null,
      copy_agents_md: false,
      fork_thread_id: null,
      fork_source_run_id: null,
    },
  });
  const run = await eventually(async () => {
    const current = await api(`/runs/${response.run.id}`);
    return current.codex_thread_id && current.workspace_id ? current : undefined;
  }, `Run ${response.run.id} readiness`, 90_000, 500);
  return { task, run };
}

async function taskEvents(taskId, afterSequence) {
  const query = new URLSearchParams({ limit: "200" });
  if (afterSequence !== undefined) query.set("after_sequence", String(afterSequence));
  return api(`/tasks/${taskId}/events?${query}`);
}

async function waitForTurn(taskId, turnId, timeoutMs = 180_000) {
  return eventually(async () => {
    const events = await taskEvents(taskId);
    const failure = events.find((event) =>
      event.turn_id === turnId &&
      (event.event_type === "codex.thread.failed" || event.payload?.data?.failureReason),
    );
    if (failure) throw new Error(`Turn ${turnId} failed: ${sanitize(failure.payload)}`);
    return events.some((event) =>
      event.event_type === "codex.turn.completed" && event.turn_id === turnId,
    ) ? events : undefined;
  }, `Turn ${turnId} completion`, timeoutMs, 500);
}

function itemData(event) {
  return event.payload?.data ?? {};
}

function itemType(event) {
  return event.payload?.itemType ?? itemData(event).type;
}

function textFromEvents(events) {
  return events.map((event) => sanitize(event.payload)).join("\n");
}

async function send(taskId, text, accessMode = "workspace-write") {
  return api(`/tasks/${taskId}/messages`, {
    method: "POST",
    body: {
      text,
      model,
      effort: "none",
      service_tier: null,
      access_mode: accessMode,
      images: [],
      collaboration_mode: null,
    },
  });
}

class EventSocket {
  constructor(url, token) {
    this.events = [];
    this.messages = [];
    this.socket = new WebSocket(url);
    this.ready = new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error("event WebSocket readiness timed out")), 10_000);
      this.socket.addEventListener("open", () => {
        this.socket.send(JSON.stringify({ type: "authenticate", token }));
      });
      this.socket.addEventListener("message", ({ data }) => {
        const message = JSON.parse(String(data));
        this.messages.push(message);
        if (message.type === "ready") {
          clearTimeout(timer);
          resolve();
        }
        if (message.type === "run.event") this.events.push(message.event);
      });
      this.socket.addEventListener("error", () => reject(new Error("event WebSocket failed")));
    });
  }

  close() {
    this.socket.close();
  }
}

function updateE2eMcpBlock(content) {
  const begin = "# BEGIN open-web-codex real E2E MCP";
  const end = "# END open-web-codex real E2E MCP";
  const escaped = mcpBinary.replaceAll("\\", "\\\\").replaceAll('"', '\\"');
  const block = [
    begin,
    "[mcp_servers.e2e_tools]",
    `command = "${escaped}"`,
    'env = { MCP_TEST_VALUE = "MCP_ENV_OK" }',
    "startup_timeout_sec = 20.0",
    "tool_timeout_sec = 30.0",
    "required = true",
    end,
  ].join("\n");
  const pattern = new RegExp(`${begin.replaceAll("-", "\\-")}[^]*?${end.replaceAll("-", "\\-")}\\n?`, "m");
  const cleaned = content.replace(pattern, "").trimEnd();
  return `${cleaned}${cleaned ? "\n\n" : ""}${block}\n`;
}

await runCase("health and authenticated bootstrap", async () => {
  const health = await api("/health");
  assert.equal(health.ok, true);
  let auth;
  try {
    auth = await api("/bootstrap", {
      method: "POST",
      body: { name: "Real E2E Owner", username, email, password },
    });
  } catch (error) {
    if (!error.message.includes("409")) throw error;
    auth = await api("/sessions", {
      method: "POST",
      body: { username, password, organization_id: null },
    });
  }
  state.token = auth.session_token;
  const me = await api("/me");
  assert.equal(me.username, username);
  return `server ${health.version}`;
});

await runCase("Provider add, refresh, switch, and context update", async () => {
  let catalog = await api(`/providers/${providerId}`, {
    method: "PUT",
    body: {
      name: "DeepSeek E2E",
      baseUrl: providerBaseUrl,
      wireApi: "chat",
      credentials: { mode: "direct", apiKey: deepseekKey },
      select: true,
    },
  });
  let provider = findProvider(catalog, providerId);
  assert(provider, "created Provider is missing");
  assert.equal(currentProviderId(catalog), providerId);
  assert.equal(provider.wireApi, "chat");
  assert(!JSON.stringify(catalog).includes(deepseekKey), "Provider catalog leaked the API key");

  catalog = await api(`/providers/${providerId}/models/refresh`, { method: "POST" });
  provider = findProvider(catalog, providerId);
  assert(provider.models.some((entry) => entry.modelId === model), `${model} was not discovered`);

  catalog = await api(`/providers/${providerId}/models/${encodeURIComponent(model)}`, {
    method: "PATCH",
    body: { contextWindow: 131072 },
  });
  provider = findProvider(catalog, providerId);
  assert.equal(provider.models.find((entry) => entry.modelId === model)?.contextWindow, 131072);

  const alternate = catalog.data.find((entry) => entry.id !== providerId && entry.kind === "builtIn");
  assert(alternate, "no built-in Provider exists for switch coverage");
  catalog = await api(`/providers/${alternate.id}/select`, { method: "POST" });
  assert.equal(currentProviderId(catalog), alternate.id);
  catalog = await api(`/providers/${providerId}/select`, { method: "POST" });
  assert.equal(currentProviderId(catalog), providerId);

  const config = await api("/profile/files/config");
  assert(!config.content.includes(deepseekKey), "Profile config leaked the direct Provider key");
  assert(provider.envKey, "secured Provider did not expose its safe environment key name");
  return `${provider.modelCount} models; current=${currentProviderId(catalog)}`;
});

await runCase("MCP registration through the Server profile API", async () => {
  const config = await api("/profile/files/config");
  const content = updateE2eMcpBlock(config.content);
  await api("/profile/files/config", { method: "PUT", body: { content } });
  const stored = await api("/profile/files/config");
  assert(stored.content.includes("[mcp_servers.e2e_tools]"));
  assert(stored.content.includes(mcpBinary));
  return "e2e_tools registered without browser path or raw JSON-RPC";
});

await runCase("managed workspace and first Codex thread", async () => {
  state.project = await api("/projects/managed", {
    method: "POST",
    body: { name: `Real E2E ${stamp}` },
  });
  const created = await createTaskAndRun(`Primary ${stamp}`);
  state.firstTask = created.task;
  state.firstRun = created.run;
  assert.equal(created.run.status, "running");
  assert(created.run.codex_thread_id);
  assert(created.run.workspace_id);
  const mcp = await eventually(async () => {
    const projection = await api(`/profile/mcp-servers?runId=${created.run.id}`);
    return JSON.stringify(projection).includes("e2e_tools") ? projection : undefined;
  }, "MCP server discovery", 30_000, 500);
  assert(JSON.stringify(mcp).includes("e2e_tools"));
  return `project=${state.project.id}; thread ready`;
});

await runCase("message streaming, reasoning projection, and code execution", async () => {
  const wsUrl = baseUrl.replace(/^http/, "ws") + "/api/events/ws";
  state.ws = new EventSocket(wsUrl, state.token);
  await state.ws.ready;
  const sentAt = Date.now();
  const response = await send(
    state.firstTask.id,
    `This is ${marker}. Use the shell or file tools to create e2e/fibonacci.py. ` +
      "The program must print exactly FIB_OK=55, run it, verify that output, and then briefly report completion.",
  );
  assert.equal(response.thread_id, state.firstRun.codex_thread_id);
  const events = await waitForTurn(state.firstTask.id, response.turn_id);
  state.firstTurnEvents = events;
  const turnEvents = events.filter((event) => event.turn_id === response.turn_id);
  assert(turnEvents.some((event) => event.event_type === "codex.turn.started"));
  assert(turnEvents.some((event) => event.event_type === "codex.turn.completed"));
  assert(turnEvents.some((event) => event.event_type === "codex.item.delta"), "no streaming delta was projected");
  assert(turnEvents.some((event) => itemType(event) === "commandExecution"), "no command execution item was projected");
  assert(turnEvents.some((event) => itemType(event) === "fileChange") || textFromEvents(turnEvents).includes("fibonacci.py"));
  assert(textFromEvents(turnEvents).includes("FIB_OK=55"), "verified command output was not projected");
  const timestamps = turnEvents.map((event) => Date.parse(event.created_at));
  assert(timestamps.every((time, index) => index === 0 || time >= timestamps[index - 1]), "event timestamps regressed");
  const live = state.ws.events.filter((event) => event.turn_id === response.turn_id);
  assert(live.some((event) => event.event_type === "codex.item.delta"), "live socket missed streaming deltas");
  assert(live.some((event) => event.event_type === "codex.turn.completed"), "live socket missed Turn completion");
  const reasoningProjected = turnEvents.some((event) => itemType(event) === "reasoning") ||
    turnEvents.some((event) => event.payload?.data?.sourceType?.startsWith("item/reasoning"));
  const elapsedMs = Date.now() - sentAt;
  return `events=${turnEvents.length}, live=${live.length}, elapsed=${elapsedMs}ms, reasoning=${reasoningProjected}`;
});

await runCase("workspace file tree and file preview", async () => {
  const files = await api(`/runs/${state.firstRun.id}/workspace/files`);
  assert(files.includes("e2e/fibonacci.py"), "generated source file is absent from file tree");
  const preview = await api(
    `/runs/${state.firstRun.id}/workspace/files/content?path=${encodeURIComponent("e2e/fibonacci.py")}`,
  );
  assert.equal(preview.truncated, false);
  assert(preview.content.includes("FIB_OK"));
  return `previewed ${preview.content.length} bytes`;
});

await runCase("third-party Provider MCP tool invocation", async () => {
  const response = await send(
    state.firstTask.id,
    `Call the e2e_tools MCP echo tool with message "${marker}_MCP" and env_var "MCP_TEST_VALUE". ` +
      "Do not simulate the tool. Return both the echo and environment value.",
  );
  const events = await waitForTurn(state.firstTask.id, response.turn_id);
  const turnEvents = events.filter((event) => event.turn_id === response.turn_id);
  const mcpCall = turnEvents.find((event) => itemType(event) === "mcpToolCall");
  assert(mcpCall, "no mcpToolCall event was projected");
  const projected = textFromEvents(turnEvents);
  assert(projected.includes(`${marker}_MCP`), "MCP echo marker was not projected");
  assert(projected.includes("MCP_ENV_OK"), "MCP environment result was not projected");
  return "DeepSeek emitted a tool call and the real stdio MCP server replied";
});

await runCase("approval request and decision event lifecycle", async () => {
  const approvalMarker = `/Users/zhaoyu/Documents/open-web-codex-approval-${stamp}.txt`;
  const response = await send(
    state.firstTask.id,
    `Use the shell to write the exact text APPROVAL_OK into ${approvalMarker}, read it back, ` +
      "and remove it in the same command. This is intentionally outside the workspace; request approval.",
  );
  const approval = await eventually(async () => {
    const pending = await api("/approvals");
    return pending.find((entry) => entry.runId === state.firstRun.id) ?? undefined;
  }, "approval request", 60_000, 250);
  assert.equal(approval.state, "pending");
  await api(`/approvals/${approval.id}/decision`, {
    method: "POST",
    body: { decision: "accept", version: approval.version },
  });
  const events = await waitForTurn(state.firstTask.id, response.turn_id);
  const turnEvents = events.filter((event) => event.turn_id === response.turn_id);
  assert(turnEvents.some((event) => event.event_type === "platform.approval.requested"));
  assert(turnEvents.some((event) => event.event_type === "platform.approval.resolved"));
  return `approval=${approval.id} resolved`;
});

await runCase("thread running state and conversation history restoration", async () => {
  const created = await createTaskAndRun(`Delayed ${stamp}`);
  state.secondTask = created.task;
  state.secondRun = created.run;
  const historyBefore = await taskEvents(state.firstTask.id);
  const beforeSignature = historyBefore.map((event) => `${event.sequence}:${event.event_type}:${event.item_id ?? ""}`).join("|");
  const response = await send(
    state.secondTask.id,
    "Run this exact shell command: sleep 8 && mkdir -p e2e && printf DELAY_DONE > e2e/delay-done.txt. " +
      "Wait for it to finish, then report DELAY_DONE.",
  );
  const active = await eventually(async () => {
    const run = await api(`/runs/${state.secondRun.id}`);
    return run.active_turn_id === response.turn_id ? run : undefined;
  }, "delayed thread active state", 15_000, 200);
  assert.equal(active.status, "running");

  await new Promise((resolve) => setTimeout(resolve, 1_500));
  const stillActive = await api(`/runs/${state.secondRun.id}`);
  assert.equal(stillActive.active_turn_id, response.turn_id, "delayed Turn stopped reporting active too early");
  const restored = await taskEvents(state.firstTask.id);
  const restoredPrefix = restored.slice(0, historyBefore.length);
  const restoredSignature = restoredPrefix.map((event) => `${event.sequence}:${event.event_type}:${event.item_id ?? ""}`).join("|");
  assert.equal(restoredSignature, beforeSignature, "first Thread history changed while second Thread was active");
  assert(textFromEvents(restored).includes(marker), "first Thread message history did not restore");

  const completed = await waitForTurn(state.secondTask.id, response.turn_id, 120_000);
  const finishedRun = await api(`/runs/${state.secondRun.id}`);
  assert.equal(finishedRun.active_turn_id, null);
  assert(completed.some((event) => event.turn_id === response.turn_id && itemType(event) === "commandExecution"));
  const delayedFile = await api(
    `/runs/${state.secondRun.id}/workspace/files/content?path=${encodeURIComponent("e2e/delay-done.txt")}`,
  );
  assert.equal(delayedFile.content, "DELAY_DONE");
  return `history=${restored.length} events; delayed Turn stayed active and completed`;
});

await runCase("durable event replay matches live ordering", async () => {
  const durable = await taskEvents(state.firstTask.id);
  const sequences = durable.map((event) => event.sequence);
  assert.equal(new Set(sequences).size, sequences.length, "durable event sequence contains duplicates");
  assert(sequences.every((sequence, index) => index === 0 || sequence > sequences[index - 1]), "durable event sequence is not strictly increasing");
  const liveSequences = new Set(state.ws.events.map((event) => event.sequence));
  const comparable = durable.filter((event) => liveSequences.has(event.sequence));
  assert(comparable.length > 0, "no overlap between live and durable event streams");
  for (const event of comparable) {
    const live = state.ws.events.find((candidate) => candidate.sequence === event.sequence);
    assert.equal(live.event_type, event.event_type);
    assert.equal(live.run_id, event.run_id);
  }
  return `${comparable.length} live events matched durable replay`;
});

state.ws?.close();
log("\nReal platform E2E summary");
for (const result of results) log(`- ${result.status.toUpperCase()} ${result.name} (${result.durationMs} ms)`);
log(`\n${results.length}/${results.length} cases passed.`);
