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
  workspace_id: project.id,
  title: "Thread",
  status: "pending",
  model_provider: "deepseek",
  model: "deepseek-v4-flash",
  created_at: "2026-07-22T00:00:00Z",
  updated_at: "2026-07-22T00:00:00Z",
};

const workspace = {
  id: project.id,
  project_id: project.id,
  name: project.name,
  kind: "main",
  state: "ready",
  source_ref: "main",
  branch_name: "main",
  parent_workspace_id: null,
  group_workspace_id: null,
  managed: true,
  created_at: project.created_at,
  updated_at: project.updated_at,
};

const run = {
  id: "run-1",
  task_id: task.id,
  status: "running",
  failure_code: null,
  codex_thread_id: "thread-1",
  active_turn_id: null,
  workspace_id: workspace.id,
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

function resourceFetch(
  events: unknown[] = [],
  turns: unknown[] = [],
  runValue: typeof run = run,
) {
  return vi.fn(async (input: RequestInfo | URL) => {
    const url = new URL(String(input));
    if (url.pathname === "/api/workspaces") return json([workspace]);
    if (url.pathname === `/api/workspaces/${workspace.id}`) return json(workspace);
    if (url.pathname === "/api/projects") return json([project]);
    if (url.pathname === `/api/projects/${project.id}`) return json(project);
    if (url.pathname === `/api/projects/${project.id}/thread-contexts`) {
      return json([{ project, task, run: runValue }]);
    }
    if (url.pathname === "/api/tasks") return json([task]);
    if (url.pathname === `/api/tasks/${task.id}`) return json(task);
    if (url.pathname === "/api/runs") return json([runValue]);
    if (url.pathname === `/api/runs/${run.id}`) return json(runValue);
    if (url.pathname === `/api/runs/${run.id}/thread`) {
      return json({
        thread: {
          id: "thread-1",
          name: task.title,
          preview: task.title,
          createdAt: 1,
          updatedAt: 2,
          status: runtimeStatus(runValue),
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

  it("projects real Workspace ids without exposing a server-local root", async () => {
    vi.stubGlobal("fetch", resourceFetch());
    const client = new CodexMonitorWebClient({ baseUrl: "http://server.test" });

    await expect(client.listWorkspaces()).resolves.toEqual([expect.objectContaining({
      id: workspace.id,
      name: workspace.name,
      path: workspace.name,
      connected: true,
    })]);
  });

  it("keeps Copilot desired-state writes as narrow package-id adapters", async () => {
    const status = { packages: [], installations: [] };
    const fetchMock = vi.fn(async () => json(status));
    vi.stubGlobal("fetch", fetchMock);
    const client = new CodexMonitorWebClient({ baseUrl: "http://server.test" });

    await client.activateCopilot("meeting-action-review");
    await client.deactivateCopilot("meeting-action-review");

    expect(fetchMock.mock.calls.map(([input, init]) => ({
      path: new URL(String(input)).pathname,
      method: init?.method,
      body: JSON.parse(String(init?.body)),
    }))).toEqual([
      {
        path: "/api/profile/copilots/activate",
        method: "POST",
        body: { packageId: "meeting-action-review" },
      },
      {
        path: "/api/profile/copilots/deactivate",
        method: "POST",
        body: { packageId: "meeting-action-review" },
      },
    ]);
  });

  it("does not expose an interrupted pre-restart Turn as active during recovery", async () => {
    const recoveringRun = {
      ...run,
      status: "recovery_pending",
      active_turn_id: "turn-before-restart",
    };
    vi.stubGlobal("fetch", resourceFetch([], [], recoveringRun));
    const client = new CodexMonitorWebClient({ baseUrl: "http://server.test" });

    await expect(client.listThreads(workspace.id)).resolves.toEqual({
      data: [expect.objectContaining({
        id: "thread-1",
        activeTurnId: null,
        status: "idle",
      })],
      nextCursor: null,
    });
    await expect(client.resumeThread(workspace.id, "thread-1")).resolves.toEqual({
      thread: expect.objectContaining({
        activeTurnId: null,
        status: { type: "idle", activeFlags: [] },
      }),
    });
  });

  it("keeps polling an accepted Run through a transient read failure without creating a second Task or Run", async () => {
    const baseFetch = resourceFetch();
    const pendingRun = {
      ...run,
      status: "pending",
      codex_thread_id: null,
    };
    let runReads = 0;
    let taskCreates = 0;
    let runStarts = 0;
    const fetchMock = vi.fn(
      async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = new URL(String(input));
        if (url.pathname === "/api/tasks" && init?.method === "POST") {
          taskCreates += 1;
          return json(task);
        }
        if (
          url.pathname === `/api/tasks/${task.id}/runs` &&
          init?.method === "POST"
        ) {
          runStarts += 1;
          return json({ run: pendingRun });
        }
        if (
          url.pathname === `/api/runs/${run.id}` &&
          (!init?.method || init.method === "GET")
        ) {
          runReads += 1;
          if (runReads === 1) {
            throw new Error("temporary network outage");
          }
          return json(run);
        }
        return baseFetch(input, init);
      },
    );
    vi.stubGlobal("fetch", fetchMock);
    vi.stubGlobal("crypto", {
      randomUUID: () => "018f-idempotency-key",
    });
    const client = new CodexMonitorWebClient({
      baseUrl: "http://server.test",
    });
    const options = {
      operationId: "stable-launch-operation",
      providerId: task.model_provider,
      modelId: task.model,
    };

    await expect(
      client.startThread(workspace.id, options),
    ).resolves.toEqual({
      thread: expect.objectContaining({ id: "thread-1" }),
    });
    expect(taskCreates).toBe(1);
    expect(runStarts).toBe(1);
    expect(runReads).toBeGreaterThanOrEqual(2);
  });

  it("reports the safe failure code when an accepted Run fails before creating its Thread", async () => {
    const baseFetch = resourceFetch();
    const pendingRun = {
      ...run,
      status: "pending",
      codex_thread_id: null,
    };
    const failedRun = {
      ...pendingRun,
      status: "failed",
      failure_code: "codex_unavailable" as const,
    };
    const onRunAccepted = vi.fn();
    const fetchMock = vi.fn(
      async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = new URL(String(input));
        if (url.pathname === "/api/tasks" && init?.method === "POST") {
          return json(task);
        }
        if (
          url.pathname === `/api/tasks/${task.id}/runs` &&
          init?.method === "POST"
        ) {
          return json({ run: pendingRun });
        }
        if (
          url.pathname === `/api/runs/${run.id}` &&
          (!init?.method || init.method === "GET")
        ) {
          return json(failedRun);
        }
        return baseFetch(input, init);
      },
    );
    vi.stubGlobal("fetch", fetchMock);
    vi.stubGlobal("crypto", {
      randomUUID: () => "018f-idempotency-key",
    });
    const client = new CodexMonitorWebClient({
      baseUrl: "http://server.test",
    });

    await expect(
      client.startThread(workspace.id, {
        operationId: "accepted-terminal-run",
        providerId: task.model_provider,
        modelId: task.model,
        onRunAccepted,
      }),
    ).rejects.toMatchObject({
      code: "run_terminal",
      message:
        "Run failed before its Codex Thread was ready. Failure code: codex_unavailable.",
    });
    expect(onRunAccepted).toHaveBeenCalledWith({
      taskId: task.id,
      runId: run.id,
    });
  });

  it("restores Runtime Agent projections and Artifacts for a Thread", async () => {
    const baseFetch = resourceFetch();
    const agents = [{
      run_id: run.id,
      thread_id: run.codex_thread_id,
      parent_thread_id: null,
      source_kind: "root",
      agent_path: null,
      agent_nickname: null,
      agent_role: null,
      status_type: "active",
      active_flags: [],
      is_root: true,
      first_observed_at: "2026-07-26T00:00:01Z",
      last_observed_at: "2026-07-26T00:00:02Z",
    }];
    const artifacts = [{
      id: "artifact-1",
      task_id: task.id,
      artifact_schema: "planning-dataset.v1",
      display_name: "planning-dataset.v1",
      mime_type: "application/json",
      expected_size: 100,
      byte_size: 100,
      content_sha256: "b".repeat(64),
      state: "ready",
      failure: null,
      content_url: "/api/artifacts/artifact-1/content",
      download_url: "/api/artifacts/artifact-1/download",
      producer_run_id: run.id,
      producer_thread_id: "data-thread",
      producer_turn_id: "data-turn",
      producer_item_id: "data-item",
      producer_agent_role: "data_agent",
      created_at: "2026-07-26T00:00:03Z",
      updated_at: "2026-07-26T00:00:04Z",
    }];
    const activities = [{
      run_id: run.id,
      sequence: 5,
      thread_id: "data-thread",
      turn_id: "data-turn",
      item_id: "data-item",
      kind: "tool_started",
      status: "running",
      title: "Using planning data · load network",
      detail: null,
      created_at: "2026-07-26T00:00:03Z",
    }];
    const executions = [{
      id: "execution-data-1",
      run_id: run.id,
      thread_id: "data-thread",
      turn_id: "data-turn",
      ordinal: 1,
      task: "Load the planning network.",
      status: "running",
      current_behavior: "Using planning data · load network",
      latest_progress: null,
      first_observed_sequence: 4,
      last_observed_sequence: 5,
      started_at: "2026-07-26T00:00:02Z",
      completed_at: null,
      created_at: "2026-07-26T00:00:02Z",
      updated_at: "2026-07-26T00:00:03Z",
    }];
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input));
      if (url.pathname === `/api/runs/${run.id}/agents`) return json(agents);
      if (url.pathname === `/api/runs/${run.id}/agent-activities`) return json(activities);
      if (url.pathname === `/api/runs/${run.id}/agent-executions`) return json(executions);
      if (url.pathname === `/api/tasks/${task.id}/artifacts`) return json(artifacts);
      return baseFetch(input, init);
    });
    vi.stubGlobal("fetch", fetchMock);
    const client = new CodexMonitorWebClient({ baseUrl: "http://server.test" });

    await client.listThreads(workspace.id);
    await expect(client.getSupervisorOverview("thread-1")).resolves.toEqual({
      taskTitle: task.title,
      agents,
      activities,
      executions,
      artifacts,
    });
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

  it("restores the persisted Thread token usage for an already-bound Thread", async () => {
    const tokenUsage = {
      total: { totalTokens: 900, inputTokens: 800, cachedInputTokens: 100, outputTokens: 100, reasoningOutputTokens: 0 },
      last: { totalTokens: 300, inputTokens: 250, cachedInputTokens: 50, outputTokens: 50, reasoningOutputTokens: 0 },
      modelContextWindow: 128_000,
    };
    const baseFetch = resourceFetch();
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input));
      if (url.pathname === `/api/runs/${run.id}/thread/token-usage`) return json({ tokenUsage });
      return baseFetch(input, init);
    });
    vi.stubGlobal("fetch", fetchMock);

    const client = new CodexMonitorWebClient({ baseUrl: "http://server.test" });
    await expect(client.readThreadTokenUsage("thread-1")).resolves.toEqual(tokenUsage);
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

    await client.writeModelProvider({
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

  it("writes a Profile Provider without a Workspace-scoped argument", async () => {
    const requests: Array<{ path: string; method: string; body: unknown }> = [];
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input));
      requests.push({
        path: url.pathname,
        method: init?.method ?? "GET",
        body: init?.body ? JSON.parse(String(init.body)) : null,
      });
      return json({ currentProviderId: "deepseek", data: [] });
    });
    vi.stubGlobal("fetch", fetchMock);
    const client = new CodexMonitorWebClient({ baseUrl: "http://server.test" });

    await client.writeModelProvider({
      action: "upsert",
      id: "deepseek",
      name: "DeepSeek",
      baseUrl: "https://api.deepseek.com",
      wireApi: "chat",
      credentialMode: "none",
      select: true,
    });

    expect(requests).toEqual([{
      path: "/api/providers/deepseek",
      method: "PUT",
      body: {
        name: "DeepSeek",
        baseUrl: "https://api.deepseek.com",
        wireApi: "chat",
        credentials: { mode: "none" },
        supportsFunctionTools: true,
        select: true,
      },
    }]);
  });

  it.each([
    ["chat", true],
    ["responses", false],
  ])("derives function-tool capability from Wire API %s", async (wireApi, expected) => {
    const fetchMock = vi.fn(async (_input: RequestInfo | URL, init?: RequestInit) => {
      expect(init?.method).toBe("PUT");
      return json({ currentProviderId: "deepseek", data: [] });
    });
    vi.stubGlobal("fetch", fetchMock);
    const client = new CodexMonitorWebClient({ baseUrl: "http://server.test" });

    await client.writeModelProvider({
      action: "upsert",
      id: "deepseek",
      name: "DeepSeek",
      baseUrl: "https://api.deepseek.com",
      wireApi,
      credentialMode: "none",
    });

    const body = JSON.parse(String(fetchMock.mock.calls[0][1]?.body));
    expect(body.supportsFunctionTools).toBe(expected);
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

  it("uses the Workspace directly for files and Git while MCP stays Run-scoped", async () => {
    const otherTask = { ...task, id: "task-2", title: "Other Thread" };
    const otherRun = {
      ...run,
      id: "run-2",
      task_id: otherTask.id,
      codex_thread_id: "thread-2",
      workspace_id: workspace.id,
    };
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = new URL(String(input));
      if (url.pathname === "/api/workspaces") return json([workspace]);
      if (url.pathname === `/api/workspaces/${workspace.id}`) return json(workspace);
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
      if (url.pathname === `/api/workspaces/${workspace.id}/files`) return json(["selected.txt"]);
      if (url.pathname === `/api/workspaces/${workspace.id}/status`) {
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
    expect(urls).toContain(`http://server.test/api/workspaces/${workspace.id}/files`);
    expect(urls).toContain(`http://server.test/api/workspaces/${workspace.id}/status`);
    expect(urls.some((url) => url.includes(`/api/profile/mcp-servers?runId=${otherRun.id}`)))
      .toBe(true);
  });

  it("downloads and deletes Workspace files through typed platform methods", async () => {
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input));
      if (url.pathname === `/api/workspaces/${workspace.id}/files` && init?.method === "POST") {
        expect(init.body).toBeInstanceOf(FormData);
        return json({ status: "uploaded", paths: ["planning.csv"] });
      }
      if (url.pathname === `/api/workspaces/${workspace.id}/files/download`) {
        return new Response("workspace data", {
          status: 200,
          headers: {
            "content-type": "application/octet-stream",
            "content-disposition": "attachment; filename=\"download\"; filename*=UTF-8''report.csv",
          },
        });
      }
      if (url.pathname === `/api/workspaces/${workspace.id}/files` && init?.method === "DELETE") {
        expect(JSON.parse(String(init.body))).toEqual({ path: "data/report.csv" });
        return json({ status: "deleted", path: "data/report.csv" });
      }
      throw new Error(`Unexpected Server request: ${url.pathname}`);
    });
    vi.stubGlobal("fetch", fetchMock);
    const client = new CodexMonitorWebClient({ baseUrl: "http://server.test" });

    await expect(client.uploadWorkspaceFiles(project.id, [
      new File(["city,demand\nJakarta,10\n"], "planning.csv", { type: "text/csv" }),
    ])).resolves.toEqual({ status: "uploaded", paths: ["planning.csv"] });
    const downloaded = await client.downloadWorkspaceFile(project.id, "data/report.csv");
    expect(downloaded.filename).toBe("report.csv");
    await expect(client.deleteWorkspaceFile(project.id, "data/report.csv")).resolves.toEqual({
      status: "deleted",
      path: "data/report.csv",
    });

    expect(fetchMock.mock.calls[2]).toEqual([
      "http://server.test/api/workspaces/workspace-1/files",
      expect.objectContaining({ method: "DELETE" }),
    ]);
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
      run_id: run.id,
      root_thread_id: "thread-1",
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
    socket?.onmessage?.({
      data: JSON.stringify({
        type: "run.event",
        version: 1,
        event: {
          id: "artifact-event-live",
          sequence: 3,
          run_id: run.id,
          event_type: "platform.artifact.changed",
          projection_version: 1,
          thread_id: "thread-1",
          turn_id: "turn-1",
          item_id: "item-final",
          payload: {
            data: {
              sourceType: "platform/artifact/changed",
              artifact: {
                artifactId: "artifact-1",
                schema: "network_planning_report_markdown.v2",
                state: "ready",
                url: "/api/artifacts/artifact-1/content",
              },
            },
          },
          created_at: "2026-07-22T00:00:05Z",
        },
      }),
    });
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(events).toContainEqual({
      workspace_id: project.id,
      run_id: run.id,
      root_thread_id: "thread-1",
      message: {
        method: "platform/artifact/changed",
        params: {
          threadId: "thread-1",
          turnId: "turn-1",
          itemId: "item-final",
          sourceType: "platform/artifact/changed",
          artifact: {
            artifactId: "artifact-1",
            schema: "network_planning_report_markdown.v2",
            state: "ready",
            url: "/api/artifacts/artifact-1/content",
          },
        },
      },
    });
    unsubscribe();
  });

  it("delivers child Agent events through their authorized root Run context", async () => {
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
      baseUrl: "https://server.test",
      token: "session-token",
    });
    const events: unknown[] = [];
    client.subscribeAppServerEvents((event) => events.push(event));
    const socket = sockets[0];
    socket?.onopen?.();
    socket?.onmessage?.({ data: JSON.stringify({ type: "ready", version: 1 }) });
    await vi.waitFor(() =>
      expect(
        events.length,
      ).toBe(0),
    );
    socket?.onmessage?.({
      data: JSON.stringify({
        type: "run.event",
        version: 1,
        event: {
          id: "child-event-live",
          sequence: 1,
          run_id: run.id,
          event_type: "codex.thread.started",
          projection_version: 1,
          thread_id: "child-thread",
          turn_id: null,
          item_id: null,
          payload: {
            data: {
              sourceType: "thread/started",
              thread: {
                id: "child-thread",
                parentThreadId: "thread-1",
                source: {
                  subAgent: {
                    thread_spawn: {
                      parent_thread_id: "thread-1",
                      agent_nickname: "Network",
                      agent_role: "network_planning_agent",
                    },
                  },
                },
              },
            },
          },
          created_at: "2026-07-22T00:00:03Z",
        },
      }),
    });

    await vi.waitFor(() => expect(events).toHaveLength(1));
    expect(events[0]).toEqual({
      workspace_id: project.id,
      run_id: run.id,
      root_thread_id: "thread-1",
      message: {
        method: "thread/started",
        params: {
          threadId: "child-thread",
          sourceType: "thread/started",
          thread: expect.objectContaining({
            id: "child-thread",
            parentThreadId: "thread-1",
          }),
        },
      },
    });
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
        mode: "url",
        credentialKind: "maps",
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
        method: "platform/mcpFormRequested",
        params: {
          threadId: "thread-1",
          turnId: "turn-1",
          runId: "run-1",
          approvalId: "approval-mcp-1",
        },
      },
      {
        method: "item/commandExecution/requestApproval",
        id: "approval-mcp-url-1",
        params: {
          threadId: "thread-1",
          turnId: "turn-1",
          mode: "url",
          credentialKind: "maps",
          command: "Map provider and API key required",
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
      run_id: run.id,
      root_thread_id: "thread-1",
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

  it("restores and answers MCP forms without replaying event schemas", async () => {
    const form = {
      id: "018f854d-2d2c-7363-99a9-804e6cc4a77b",
      runId: run.id,
      source: { kind: "agent", executionId: "execution-1", displayTitle: "Data Agent" },
      serverName: "supply_chain_data",
      message: "Provide route inputs",
      fields: [{
        name: "factor",
        title: "Detour factor",
        description: "",
        required: true,
        schema: { kind: "number", default: 1.2, minimum: 1, maximum: 2 },
      }],
      state: "pending",
      version: 4,
      createdAt: "2026-08-09T00:00:00Z",
    };
    const baseFetch = resourceFetch();
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input));
      if (url.pathname === `/api/runs/${run.id}/mcp-form-requests`) return json([form]);
      if (url.pathname === `/api/approvals/${form.id}/mcp-form`) {
        return new Response(null, { status: 204 });
      }
      return baseFetch(input, init);
    });
    vi.stubGlobal("fetch", fetchMock);
    const client = new CodexMonitorWebClient({ baseUrl: "http://server.test" });

    await expect(client.listThreadMcpFormRequests("thread-1")).resolves.toEqual([form]);
    await client.respondToMcpForm(form.id, form.version, "accept", { factor: 1.35 });

    const submit = fetchMock.mock.calls.find(([input]) =>
      String(input).endsWith(`/api/approvals/${form.id}/mcp-form`));
    expect(JSON.parse(String(submit?.[1]?.body))).toEqual({
      action: "accept",
      content: { factor: 1.35 },
      version: 4,
    });
    expect(fetchMock.mock.calls.some(([input]) => String(input).includes("/events"))).toBe(false);
  });
});
