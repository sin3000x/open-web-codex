import { afterEach, describe, expect, it, vi } from "vitest";

import { CodexMonitorWebClient } from "./webClient";

const project = {
  id: "workspace-1",
  name: "Workspace",
  git_url: "managed://018f854d-2d2c-7363-99a9-804e6cc4a99e",
  default_branch: "main",
  created_at: "2026-07-22T00:00:00Z",
  updated_at: "2026-07-22T00:00:00Z",
};

const task = {
  id: "task-1",
  project_id: project.id,
  title: "Thread",
  status: "pending",
  model_provider: "deepseek",
  model: "deepseek-v4-flash",
  created_at: "2026-07-22T00:00:00Z",
  updated_at: "2026-07-22T00:00:00Z",
};

const run = {
  id: "run-1",
  task_id: task.id,
  status: "running",
  codex_thread_id: "thread-1",
  active_turn_id: null,
  workspace_id: "workspace-checkout-1",
  source_ref: "main",
  workspace_kind: "main",
  workspace_name: null,
  workspace_parent_run_id: null,
  workspace_group_run_id: null,
  attempt: 1,
  created_at: "2026-07-22T00:00:00Z",
  updated_at: "2026-07-22T00:00:00Z",
};

function json(value: unknown) {
  return new Response(JSON.stringify(value), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

function resourceFetch(events: unknown[] = [], turns: unknown[] = []) {
  return vi.fn(async (input: RequestInfo | URL) => {
    const url = new URL(String(input));
    if (url.pathname === "/api/projects") return json([project]);
    if (url.pathname === `/api/projects/${project.id}`) return json(project);
    if (url.pathname === `/api/projects/${project.id}/thread-contexts`) {
      return json([{ project, task, run }]);
    }
    if (url.pathname === "/api/tasks") return json([task]);
    if (url.pathname === `/api/tasks/${task.id}`) return json(task);
    if (url.pathname === "/api/runs") return json([run]);
    if (url.pathname === `/api/runs/${run.id}`) return json(run);
    if (url.pathname === `/api/runs/${run.id}/thread`) {
      return json({
        thread: {
          id: "thread-1",
          name: task.title,
          preview: task.title,
          createdAt: 1,
          updatedAt: 2,
          status: runtimeStatus(run),
          turns,
        },
      });
    }
    if (url.pathname === `/api/runs/${run.id}/thread/turns`) return json(turns);
    if (url.pathname === `/api/runs/${run.id}/thread/archive`) {
      return json({ status: "archived" });
    }
    if (url.pathname === `/api/tasks/${task.id}/events`) return json(events);
    if (url.pathname === "/api/approvals") return json([]);
    if (url.pathname === "/api/profile/mcp-servers") {
      return json({ data: { data: [{ name: "filesystem" }] } });
    }
    if (url.pathname === "/api/profile/rate-limits") return json({ data: { rateLimits: { primary: {} } } });
    throw new Error(`Unexpected Server request: ${url.pathname}`);
  });
}

function runtimeStatus(value: typeof run) {
  return value.active_turn_id
    ? { type: "active", activeFlags: [] }
    : { type: "idle", activeFlags: [] };
}

describe("WebApp direct Server client", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("restores authoritative chronological Turn history from Codex", async () => {
    const fetchMock = resourceFetch([], [{
      id: "turn-1",
      status: "completed",
      items: [{ id: "message-1", type: "agentMessage", text: "persisted" }],
      startedAt: 1,
      completedAt: 2,
      durationMs: 1000,
    }]);
    vi.stubGlobal("fetch", fetchMock);

    const client = new CodexMonitorWebClient({ baseUrl: "http://server.test" });
    await expect(client.listThreadTurns(project.id, "thread-1")).resolves.toEqual([
      expect.objectContaining({
        id: "turn-1",
        status: "completed",
        startedAt: 1,
        items: [expect.objectContaining({ text: "persisted" })],
      }),
    ]);
    expect(fetchMock.mock.calls.every((call) => !String(call[0]).includes("/api/rpc"))).toBe(true);
  });

  it("returns the object-shaped Thread status consumed by WebApp", async () => {
    const activeRun = { ...run, active_turn_id: "turn-active" };
    const baseFetch = resourceFetch();
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input));
      if (url.pathname === `/api/projects/${project.id}/thread-contexts`) {
        return json([{ project, task, run: activeRun }]);
      }
      if (url.pathname === "/api/runs") return json([activeRun]);
      if (url.pathname === `/api/runs/${run.id}`) return json(activeRun);
      if (url.pathname === `/api/runs/${run.id}/thread`) {
        return json({ thread: {
          id: "thread-1",
          name: task.title,
          preview: task.title,
          createdAt: 1,
          updatedAt: 2,
          status: runtimeStatus(activeRun),
          turns: [],
        } });
      }
      return baseFetch(input, init);
    });
    vi.stubGlobal("fetch", fetchMock);

    const client = new CodexMonitorWebClient({ baseUrl: "http://server.test" });
    await expect(client.resumeThread(project.id, "thread-1")).resolves.toEqual({
      thread: expect.objectContaining({
        status: { type: "active", activeFlags: [] },
      }),
    });
  });

  it("reports a reusable Run without an active Turn as idle in the sidebar", async () => {
    const fetchMock = resourceFetch();
    vi.stubGlobal("fetch", fetchMock);

    const client = new CodexMonitorWebClient({ baseUrl: "http://server.test" });
    await expect(client.listThreads(project.id)).resolves.toEqual({
      data: [expect.objectContaining({
        id: "thread-1",
        activeTurnId: null,
        modelProvider: "deepseek",
        model: "deepseek-v4-flash",
        status: "idle",
      })],
      nextCursor: null,
    });
    const threadListRequest = fetchMock.mock.calls.find((call) =>
      String(call[0]).endsWith(`/api/projects/${project.id}/thread-contexts`));
    expect(threadListRequest?.[1]?.cache).toBe("no-store");
  });

  it("projects the legacy New Agent placeholder as Thread", async () => {
    const legacyTask = { ...task, title: "New Agent" };
    const baseFetch = resourceFetch();
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input));
      if (url.pathname === `/api/projects/${project.id}/thread-contexts`) {
        return json([{ project, task: legacyTask, run }]);
      }
      return baseFetch(input, init);
    });
    vi.stubGlobal("fetch", fetchMock);
    const client = new CodexMonitorWebClient({ baseUrl: "http://server.test" });

    await expect(client.listThreads(project.id)).resolves.toEqual({
      data: [expect.objectContaining({ name: "Thread", preview: "Thread" })],
      nextCursor: null,
    });
  });

  it("persists a Thread-specific Provider and model through the task route", async () => {
    const baseFetch = resourceFetch();
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input));
      if (url.pathname === `/api/tasks/${task.id}/model-selection`) {
        expect(init?.method).toBe("PUT");
        expect(JSON.parse(String(init?.body))).toEqual({
          providerId: "openai",
          modelId: "gpt-5.1-codex",
        });
        return json({ providerId: "openai", modelId: "gpt-5.1-codex" });
      }
      return baseFetch(input, init);
    });
    vi.stubGlobal("fetch", fetchMock);
    const client = new CodexMonitorWebClient({ baseUrl: "http://server.test" });

    await expect(client.updateThreadModelSelection(
      project.id,
      "thread-1",
      "openai",
      "gpt-5.1-codex",
    )).resolves.toEqual({
      providerId: "openai",
      modelId: "gpt-5.1-codex",
    });
  });

  it("returns the Server-persisted Thread name with the started Turn", async () => {
    const baseFetch = resourceFetch();
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input));
      if (url.pathname === `/api/tasks/${task.id}/messages`) {
        expect(init?.method).toBe("POST");
        return json({
          status: "sent",
          thread_id: "thread-1",
          turn_id: "turn-1",
          thread_name: "Show Shanghai on a map",
        });
      }
      return baseFetch(input, init);
    });
    vi.stubGlobal("fetch", fetchMock);
    const client = new CodexMonitorWebClient({ baseUrl: "http://server.test" });

    await client.listThreads(project.id);
    await expect(client.sendUserMessage(
      project.id,
      "thread-1",
      "Show Shanghai on a map",
    )).resolves.toEqual({
      status: "sent",
      threadId: "thread-1",
      threadName: "Show Shanghai on a map",
      turn: { id: "turn-1", status: "inProgress" },
    });
  });

  it("persists multiple model contexts sequentially from one UI action", async () => {
    const requests: Array<{ path: string; contextWindow: number }> = [];
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input));
      if (init?.method !== "PATCH") throw new Error(`Unexpected method for ${url.pathname}`);
      const body = JSON.parse(String(init.body)) as { contextWindow: number };
      requests.push({ path: url.pathname, contextWindow: body.contextWindow });
      return json({ data: [] });
    });
    vi.stubGlobal("fetch", fetchMock);
    const client = new CodexMonitorWebClient({ baseUrl: "http://server.test" });

    await client.writeModelProvider(project.id, {
      action: "contexts",
      id: "deepseek",
      contexts: [
        { modelId: "deepseek-flash", contextWindow: 96_000 },
        { modelId: "deepseek-pro", contextWindow: 192_000 },
      ],
    });

    expect(requests).toEqual([
      {
        path: "/api/providers/deepseek/models/deepseek-flash",
        contextWindow: 96_000,
      },
      {
        path: "/api/providers/deepseek/models/deepseek-pro",
        contextWindow: 192_000,
      },
    ]);
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });

  it("archives the selected Thread through the typed Server Run route", async () => {
    const fetchMock = resourceFetch();
    vi.stubGlobal("fetch", fetchMock);
    const client = new CodexMonitorWebClient({ baseUrl: "http://server.test" });

    await expect(client.archiveThread(project.id, "thread-1")).resolves.toEqual({
      status: "archived",
    });

    const archiveRequest = fetchMock.mock.calls.find((call) =>
      String(call[0]).endsWith(`/api/runs/${run.id}/thread/archive`));
    expect(archiveRequest?.[1]?.method).toBe("POST");
    expect(fetchMock.mock.calls.every((call) => !String(call[0]).includes("/api/rpc"))).toBe(true);
  });

  it("loads MCP and rate-limit snapshots from typed Server resources", async () => {
    const fetchMock = resourceFetch();
    vi.stubGlobal("fetch", fetchMock);

    const client = new CodexMonitorWebClient({ baseUrl: "http://server.test" });
    await expect(client.listMcpServerStatus(project.id)).resolves.toEqual({
      data: [{ name: "filesystem" }],
    });
    await expect(client.getAccountRateLimits(project.id)).resolves.toEqual({
      rateLimits: { primary: {} },
    });

    const urls = fetchMock.mock.calls.map((call) => String(call[0]));
    expect(urls.some((url) => url.includes(`/api/profile/mcp-servers?runId=${run.id}`))).toBe(true);
    expect(urls.some((url) => url.endsWith("/api/profile/rate-limits"))).toBe(true);
    expect(urls.every((url) => !url.includes("/api/rpc"))).toBe(true);
  });

  it("uses the selected Thread Run for files, Git, and MCP resources", async () => {
    const otherTask = { ...task, id: "task-2", title: "Other Thread" };
    const otherRun = {
      ...run,
      id: "run-2",
      task_id: otherTask.id,
      codex_thread_id: "thread-2",
      workspace_id: "workspace-checkout-2",
    };
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = new URL(String(input));
      if (url.pathname === "/api/projects") return json([project]);
      if (url.pathname === `/api/projects/${project.id}`) return json(project);
      if (url.pathname === `/api/projects/${project.id}/thread-contexts`) {
        return json([
          { project, task, run },
          { project, task: otherTask, run: otherRun },
        ]);
      }
      if (url.pathname === "/api/tasks") return json([task, otherTask]);
      if (url.pathname === `/api/tasks/${task.id}`) return json(task);
      if (url.pathname === `/api/tasks/${otherTask.id}`) return json(otherTask);
      if (url.pathname === "/api/runs") {
        return json(url.searchParams.get("task_id") === otherTask.id ? [otherRun] : [run]);
      }
      if (url.pathname === `/api/runs/${otherRun.id}`) return json(otherRun);
      if (url.pathname === `/api/runs/${otherRun.id}/thread`) {
        return json({ thread: {
          id: "thread-2",
          name: otherTask.title,
          preview: otherTask.title,
          createdAt: 1,
          updatedAt: 2,
          status: runtimeStatus(otherRun),
          turns: [],
        } });
      }
      if (url.pathname === `/api/tasks/${otherTask.id}/events`) return json([]);
      if (url.pathname === `/api/runs/${otherRun.id}/workspace/files`) return json(["selected.txt"]);
      if (url.pathname === `/api/runs/${otherRun.id}/workspace/status`) {
        return json({ branch: "main", ahead: 0, behind: 0, changes: [] });
      }
      if (url.pathname === "/api/profile/mcp-servers") return json({ data: { data: [] } });
      throw new Error(`Unexpected Server request: ${url.pathname}`);
    });
    vi.stubGlobal("fetch", fetchMock);
    const client = new CodexMonitorWebClient({ baseUrl: "http://server.test" });

    await expect(client.listWorkspaceFiles(project.id, "thread-2")).resolves.toEqual(["selected.txt"]);
    await client.getGitStatus(project.id, "thread-2");
    await client.listMcpServerStatus(project.id, "thread-2");

    const urls = fetchMock.mock.calls.map((call) => String(call[0]));
    expect(urls).toContain(`http://server.test/api/runs/${otherRun.id}/workspace/files`);
    expect(urls).toContain(`http://server.test/api/runs/${otherRun.id}/workspace/status`);
    expect(urls.some((url) => url.includes(`/api/profile/mcp-servers?runId=${otherRun.id}`)))
      .toBe(true);
  });

  it("uses the configured visible Provider model as the WebApp default", async () => {
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = new URL(String(input));
      if (url.pathname === "/api/providers") {
        return json({
          currentProviderId: "provider-1",
          data: [{
            id: "provider-1",
            name: "Provider",
            wireApi: "responses",
            kind: "custom",
            isCurrent: true,
            modelCount: 3,
            models: [
              { modelId: "first", showInPicker: true },
              { modelId: "hidden", showInPicker: false },
              { modelId: "configured", modelName: "Configured", showInPicker: true },
            ],
          }],
        });
      }
      if (url.pathname === "/api/profile/config/model") return json({ model: "configured" });
      throw new Error(`Unexpected Server request: ${url.pathname}`);
    });
    vi.stubGlobal("fetch", fetchMock);
    const client = new CodexMonitorWebClient({ baseUrl: "http://server.test" });

    await expect(client.listModels(project.id)).resolves.toEqual({
      data: [
        expect.objectContaining({ model: "configured", displayName: "Configured", isDefault: true }),
        expect.objectContaining({ model: "first", isDefault: false }),
      ],
    });
  });

  it("reads a Thread Provider catalog without reading or changing the global model selection", async () => {
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = new URL(String(input));
      if (url.pathname === "/api/providers") {
        return json({
          currentProviderId: "openai",
          currentModelId: "gpt-5",
          data: [
            {
              id: "openai",
              name: "OpenAI",
              wireApi: "responses",
              kind: "builtIn",
              isCurrent: true,
              modelCount: 1,
              models: [{ modelId: "gpt-5", showInPicker: true }],
            },
            {
              id: "deepseek",
              name: "DeepSeek",
              wireApi: "chat",
              kind: "custom",
              isCurrent: false,
              modelCount: 2,
              models: [
                { modelId: "deepseek-v3", showInPicker: true },
                { modelId: "deepseek-v4-flash", showInPicker: true },
              ],
            },
          ],
        });
      }
      throw new Error(`Unexpected Server request: ${url.pathname}`);
    });
    vi.stubGlobal("fetch", fetchMock);
    const client = new CodexMonitorWebClient({ baseUrl: "http://server.test" });

    await expect(client.listModels(
      project.id,
      "deepseek",
      "deepseek-v4-flash",
    )).resolves.toEqual({
      data: [
        expect.objectContaining({ model: "deepseek-v4-flash", isDefault: true }),
        expect.objectContaining({ model: "deepseek-v3", isDefault: false }),
      ],
    });
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(String(fetchMock.mock.calls[0][0])).toBe("http://server.test/api/providers");
  });

  it("projects authenticated Server WebSocket events into the unchanged WebApp contract", async () => {
    const sockets: FakeSocket[] = [];
    class FakeSocket {
      onopen: (() => void) | null = null;
      onmessage: ((message: { data: string }) => void) | null = null;
      onclose: (() => void) | null = null;
      onerror: (() => void) | null = null;
      sent: string[] = [];
      constructor(readonly url: string | URL) { sockets.push(this); }
      send(value: string) { this.sent.push(value); }
      close() {}
    }
    vi.stubGlobal("fetch", resourceFetch());
    vi.stubGlobal("WebSocket", FakeSocket);
    const client = new CodexMonitorWebClient({
      baseUrl: "https://server.test",
      token: "session-token",
    });
    const events: unknown[] = [];
    const onOpen = vi.fn();
    const unsubscribe = client.subscribeAppServerEvents((event) => events.push(event), { onOpen });
    const socket = sockets[0];
    socket?.onopen?.();
    socket?.onmessage?.({ data: JSON.stringify({ type: "ready", version: 1 }) });
    socket?.onmessage?.({
      data: JSON.stringify({
        type: "run.event",
        version: 1,
        event: {
          id: "event-live",
          sequence: 1,
          run_id: run.id,
          event_type: "codex.unknown",
          projection_version: 1,
          thread_id: "thread-1",
          turn_id: "turn-1",
          item_id: null,
          payload: {
            data: {
              sourceType: "thread/status/changed",
              status: { type: "active", activeFlags: [] },
            },
          },
          created_at: "2026-07-22T00:00:03Z",
        },
      }),
    });
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(String(socket?.url)).toBe("wss://server.test/api/events/ws");
    expect(JSON.parse(socket?.sent[0] ?? "{}")).toEqual({
      type: "authenticate",
      token: "session-token",
    });
    expect(onOpen).toHaveBeenCalledOnce();
    expect(events).toContainEqual({
      workspace_id: project.id,
      message: {
        method: "thread/status/changed",
        params: {
          threadId: "thread-1",
          turnId: "turn-1",
          sourceType: "thread/status/changed",
          status: { type: "active", activeFlags: [] },
        },
      },
    });
    unsubscribe();
  });

  it("preserves source event fields required by token, terminal, and reasoning handlers", async () => {
    const sockets: FakeSocket[] = [];
    class FakeSocket {
      onopen: (() => void) | null = null;
      onmessage: ((message: { data: string }) => void) | null = null;
      onclose: (() => void) | null = null;
      onerror: (() => void) | null = null;
      constructor(readonly url: string | URL) { sockets.push(this); }
      send() {}
      close() {}
    }
    vi.stubGlobal("fetch", resourceFetch());
    vi.stubGlobal("WebSocket", FakeSocket);
    const client = new CodexMonitorWebClient({
      baseUrl: "http://server.test",
      token: "session-token",
    });
    const events: Array<{ message?: unknown }> = [];
    client.subscribeAppServerEvents((event) => events.push(event));
    const socket = sockets[0];
    const liveEvent = (
      sequence: number,
      eventType: string,
      itemId: string | null,
      data: Record<string, unknown>,
      itemType?: string,
    ) => socket?.onmessage?.({
      data: JSON.stringify({
        type: "run.event",
        version: 1,
        event: {
          id: `event-${sequence}`,
          sequence,
          run_id: run.id,
          event_type: eventType,
          projection_version: 1,
          thread_id: "thread-1",
          turn_id: "turn-1",
          item_id: itemId,
          payload: { data, ...(itemType ? { itemType } : {}) },
          created_at: `2026-07-22T00:00:0${sequence}Z`,
        },
      }),
    });

    liveEvent(1, "codex.thread.token_usage.updated", null, {
      sourceType: "thread/tokenUsage/updated",
      tokenUsage: { totalTokens: 42 },
    });
    liveEvent(2, "codex.unknown", "command-1", {
      sourceType: "item/commandExecution/terminalInteraction",
      stdin: "yes\n",
    });
    liveEvent(3, "codex.unknown", "reasoning-1", {
      sourceType: "item/reasoning/summaryPartAdded",
    });
    liveEvent(4, "platform.approval.requested", "change-1", {
      approvalId: "approval-file-1",
      requestMethod: "item/fileChange/requestApproval",
      requestParams: { reason: "Apply the generated patch" },
    });
    liveEvent(5, "platform.approval.requested", null, {
      approvalId: "approval-mcp-1",
      requestMethod: "mcpServer/elicitation/request",
      requestParams: {
        serverName: "workspace_maps",
        mode: "form",
        message: "Allow the workspace_maps MCP server to run tool \"batch_geocode\"?",
        requestedSchema: {
          type: "object",
          properties: {},
        },
      },
    });
    liveEvent(6, "platform.approval.requested", null, {
      approvalId: "approval-mcp-url-1",
      requestMethod: "mcpServer/elicitation/request",
      requestParams: {
        serverName: "workspace_maps",
        mode: "url",
        message: "No google maps API key is stored for this workspace.",
        url: "http://127.0.0.1:43123/one-time-token",
      },
    });
    liveEvent(7, "codex.item.completed", "assistant-1", {
      text: "Done.",
      phase: "final_answer",
    }, "agentMessage");
    await vi.waitFor(() => expect(events).toHaveLength(7));

    expect(events.map((event) => event.message)).toEqual([
      {
        method: "thread/tokenUsage/updated",
        params: {
          threadId: "thread-1",
          turnId: "turn-1",
          sourceType: "thread/tokenUsage/updated",
          tokenUsage: { totalTokens: 42 },
        },
      },
      {
        method: "item/commandExecution/terminalInteraction",
        params: {
          threadId: "thread-1",
          turnId: "turn-1",
          itemId: "command-1",
          sourceType: "item/commandExecution/terminalInteraction",
          stdin: "yes\n",
        },
      },
      {
        method: "item/reasoning/summaryPartAdded",
        params: {
          threadId: "thread-1",
          turnId: "turn-1",
          itemId: "reasoning-1",
          sourceType: "item/reasoning/summaryPartAdded",
        },
      },
      {
        method: "item/commandExecution/requestApproval",
        id: "approval-file-1",
        params: {
          threadId: "thread-1",
          turnId: "turn-1",
          itemId: "change-1",
          reason: "Apply the generated patch",
          command: "Apply the generated patch",
        },
      },
      {
        method: "item/commandExecution/requestApproval",
        id: "approval-mcp-1",
        params: {
          threadId: "thread-1",
          turnId: "turn-1",
          serverName: "workspace_maps",
          mode: "form",
          message: "Allow the workspace_maps MCP server to run tool \"batch_geocode\"?",
          requestedSchema: {
            type: "object",
            properties: {},
          },
          command: "Allow the workspace_maps MCP server to run tool \"batch_geocode\"?",
        },
      },
      {
        method: "item/commandExecution/requestApproval",
        id: "approval-mcp-url-1",
        params: {
          threadId: "thread-1",
          turnId: "turn-1",
          serverName: "workspace_maps",
          mode: "url",
          message: "No google maps API key is stored for this workspace.",
          url: "http://127.0.0.1:43123/one-time-token",
          command: "No google maps API key is stored for this workspace.",
        },
      },
      {
        method: "item/completed",
        params: {
          threadId: "thread-1",
          turnId: "turn-1",
          itemId: "assistant-1",
          item: {
            id: "assistant-1",
            type: "agentMessage",
            text: "Done.",
            phase: "final_answer",
          },
        },
      },
    ]);
  });

  it("baselines durable Task history on first connect and replays only reconnect gaps", async () => {
    const replayEvent = {
      id: "event-replay",
      sequence: 2,
      run_id: run.id,
      event_type: "codex.thread.token_usage.updated",
      projection_version: 1,
      thread_id: "thread-1",
      turn_id: "turn-1",
      item_id: null,
      payload: {
        data: {
          sourceType: "thread/tokenUsage/updated",
          tokenUsage: { totalTokens: 2 },
        },
      },
      created_at: "2026-07-22T00:00:02Z",
    };
    const fetchMock = resourceFetch([replayEvent]);
    const sockets: FakeSocket[] = [];
    class FakeSocket {
      onopen: (() => void) | null = null;
      onmessage: ((message: { data: string }) => void) | null = null;
      onclose: (() => void) | null = null;
      onerror: (() => void) | null = null;
      constructor(readonly url: string | URL) { sockets.push(this); }
      send() {}
      close() {}
    }
    vi.stubGlobal("fetch", fetchMock);
    vi.stubGlobal("WebSocket", FakeSocket);
    const client = new CodexMonitorWebClient({
      baseUrl: "http://server.test",
      token: "session-token",
    });
    const methods: string[] = [];
    client.subscribeAppServerEvents((event) => {
      const message = event.message as { method?: string };
      if (message.method) methods.push(message.method);
    });
    const socket = sockets[0];
    const sendLive = (sequence: number, sourceType: string) => socket?.onmessage?.({
      data: JSON.stringify({
        type: "run.event",
        version: 1,
        event: {
          id: `event-live-${sequence}`,
          sequence,
          run_id: run.id,
          event_type: "codex.unknown",
          projection_version: 1,
          thread_id: "thread-1",
          turn_id: "turn-1",
          item_id: null,
          payload: { data: { sourceType } },
          created_at: `2026-07-22T00:00:0${sequence}Z`,
        },
      }),
    });

    socket?.onmessage?.({ data: JSON.stringify({ type: "ready", version: 1 }) });
    sendLive(1, "thread/status/changed");
    await vi.waitFor(() => expect(fetchMock.mock.calls.some((call) => {
      const url = new URL(String(call[0]));
      return url.pathname === `/api/tasks/${task.id}/events`
        && url.searchParams.get("limit") === "1"
        && !url.searchParams.has("after_sequence");
    })).toBe(true));
    expect(methods).toEqual([]);
    sendLive(3, "thread/settings/updated");
    await vi.waitFor(() => expect(methods).toEqual(["thread/settings/updated"]));

    socket?.onmessage?.({ data: JSON.stringify({ type: "resyncRequired", version: 1 }) });
    await vi.waitFor(() => expect(fetchMock.mock.calls.some((call) => {
      const url = new URL(String(call[0]));
      return url.pathname === `/api/tasks/${task.id}/events`
        && url.searchParams.get("after_sequence") === "3";
    })).toBe(true));

    socket?.onmessage?.({ data: JSON.stringify({ type: "ready", version: 1 }) });
    await vi.waitFor(() => expect(fetchMock.mock.calls.some((call) => {
      const url = new URL(String(call[0]));
      return url.pathname === `/api/tasks/${task.id}/events`
        && url.searchParams.get("after_sequence") === "3";
    })).toBe(true));
  });

  it("retries delivery-unknown approvals through the durable typed Server resource", async () => {
    const approval = {
      id: "018f854d-2d2c-7363-99a9-804e6cc4a99a",
      runId: run.id,
      threadId: "thread-1",
      requestType: "command",
      state: "delivery_unknown",
      version: 4,
      createdAt: "2026-07-22T00:00:00Z",
    };
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = new URL(String(input));
      if (url.pathname === "/api/approvals") return json([approval]);
      if (url.pathname === `/api/approvals/${approval.id}/decision`) {
        return new Response(null, { status: 204 });
      }
      throw new Error(`Unexpected Server request: ${url.pathname}`);
    });
    vi.stubGlobal("fetch", fetchMock);
    const client = new CodexMonitorWebClient({ baseUrl: "http://server.test" });

    await client.respondToServerRequest(project.id, approval.id, { decision: "accept" });

    expect(fetchMock.mock.calls[1]?.[0]).toBe(
      `http://server.test/api/approvals/${approval.id}/decision`,
    );
    expect(JSON.parse(String(fetchMock.mock.calls[1]?.[1]?.body))).toEqual({
      decision: "accept",
      version: 4,
    });
  });

  it("replays pending approval requests when the authenticated event stream becomes ready", async () => {
    const approval = {
      id: "018f854d-2d2c-7363-99a9-804e6cc4a99a",
      runId: run.id,
      threadId: "thread-1",
      requestType: "command",
      state: "pending",
      version: 1,
      createdAt: "2026-07-22T00:00:00Z",
    };
    const approvalEvent = {
      id: "event-approval",
      sequence: 7,
      run_id: run.id,
      event_type: "platform.approval.requested",
      projection_version: 1,
      thread_id: "thread-1",
      turn_id: "turn-1",
      item_id: null,
      payload: {
        data: {
          approvalId: approval.id,
          requestMethod: "item/commandExecution/requestApproval",
          requestParams: { command: "git status" },
        },
      },
      created_at: "2026-07-22T00:00:03Z",
    };
    const baseFetch = resourceFetch([approvalEvent]);
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input));
      if (url.pathname === "/api/approvals") return json([approval]);
      return baseFetch(input, init);
    });
    const sockets: FakeSocket[] = [];
    class FakeSocket {
      onopen: (() => void) | null = null;
      onmessage: ((message: { data: string }) => void) | null = null;
      onclose: (() => void) | null = null;
      onerror: (() => void) | null = null;
      constructor(readonly url: string | URL) { sockets.push(this); }
      send() {}
      close() {}
    }
    vi.stubGlobal("fetch", fetchMock);
    vi.stubGlobal("WebSocket", FakeSocket);
    const client = new CodexMonitorWebClient({
      baseUrl: "http://server.test",
      token: "session-token",
    });
    const events: unknown[] = [];
    client.subscribeAppServerEvents((event) => events.push(event));

    sockets[0]?.onmessage?.({ data: JSON.stringify({ type: "ready", version: 1 }) });
    await vi.waitFor(() => expect(events).toHaveLength(1));

    expect(events[0]).toEqual({
      workspace_id: project.id,
      message: {
        method: "item/commandExecution/requestApproval",
        id: approval.id,
        params: {
          threadId: "thread-1",
          turnId: "turn-1",
          command: "git status",
        },
      },
    });
  });

  it("answers structured user input through the typed approval resource", async () => {
    const approval = {
      id: "018f854d-2d2c-7363-99a9-804e6cc4a99b",
      runId: run.id,
      threadId: "thread-1",
      requestType: "user_input",
      state: "pending",
      version: 3,
      createdAt: "2026-07-22T00:00:00Z",
    };
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = new URL(String(input));
      if (url.pathname === "/api/approvals") return json([approval]);
      if (url.pathname === `/api/approvals/${approval.id}/user-input`) {
        return new Response(null, { status: 204 });
      }
      throw new Error(`Unexpected Server request: ${url.pathname}`);
    });
    vi.stubGlobal("fetch", fetchMock);
    const client = new CodexMonitorWebClient({ baseUrl: "http://server.test" });
    const answers = { environment: { answers: ["staging"] } };

    await client.respondToServerRequest(project.id, approval.id, { answers });

    expect(fetchMock.mock.calls[1]?.[0]).toBe(
      `http://server.test/api/approvals/${approval.id}/user-input`,
    );
    expect(JSON.parse(String(fetchMock.mock.calls[1]?.[1]?.body))).toEqual({
      answers,
      version: 3,
    });
  });
});
