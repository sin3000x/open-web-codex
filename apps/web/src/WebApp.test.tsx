// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import WebApp from "./WebApp";
import type { AppServerEvent } from "./types";

let appServerEventHandler: ((event: AppServerEvent) => void) | null = null;

const client = {
  health: vi.fn(),
  listWorkspaces: vi.fn(),
  subscribeAppServerEvents: vi.fn(),
  listThreads: vi.fn(),
  connectWorkspace: vi.fn(),
  listModelProviders: vi.fn(),
  listModels: vi.fn(),
  writeModelProvider: vi.fn(),
  selectProviderModel: vi.fn(),
  updateThreadModelSelection: vi.fn(),
  listMcpServerStatus: vi.fn(),
  getAccountRateLimits: vi.fn(),
  startThread: vi.fn(),
  resumeThread: vi.fn(),
  listThreadTurns: vi.fn(),
  readThread: vi.fn(),
  sendUserMessage: vi.fn(),
  interruptTurn: vi.fn(),
  respondToServerRequest: vi.fn(),
};

vi.mock("./services/webClient", () => ({
  CodexMonitorWebClient: vi.fn(() => client),
}));

afterEach(cleanup);

describe("WebApp workspace-first messaging", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    appServerEventHandler = null;
    window.matchMedia = vi.fn().mockReturnValue({
      matches: false,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    }) as unknown as typeof window.matchMedia;
    window.localStorage.clear();
    window.sessionStorage.clear();
    client.health.mockResolvedValue({ version: "test" });
    client.listWorkspaces.mockResolvedValue([{
      id: "workspace-1",
      name: "Demo",
      path: "/tmp/demo",
      connected: true,
      settings: { sidebarCollapsed: false },
    }]);
    client.subscribeAppServerEvents.mockImplementation((handler: (event: AppServerEvent) => void) => {
      appServerEventHandler = handler;
      return () => undefined;
    });
    client.listThreads.mockResolvedValue({ data: [] });
    client.connectWorkspace.mockResolvedValue({});
    client.listModelProviders.mockResolvedValue({ data: [] });
    client.listModels.mockResolvedValue({ data: [] });
    client.writeModelProvider.mockResolvedValue({ data: [] });
    client.selectProviderModel.mockResolvedValue({ data: [] });
    client.updateThreadModelSelection.mockResolvedValue({});
    client.listMcpServerStatus.mockResolvedValue({ data: [] });
    client.getAccountRateLimits.mockResolvedValue({});
    client.startThread.mockResolvedValue({ thread: { id: "thread-new" } });
    client.resumeThread.mockResolvedValue({ thread: { id: "thread-new", turns: [] } });
    client.listThreadTurns.mockResolvedValue([]);
    client.readThread.mockResolvedValue({ thread: { id: "thread-new", turns: [] } });
    client.sendUserMessage.mockResolvedValue({ turn: { id: "turn-1" } });
    client.interruptTurn.mockResolvedValue({ status: "interrupted" });
    client.respondToServerRequest.mockResolvedValue({});
  });

  it("creates a thread before sending when only a workspace is selected", async () => {
    render(<WebApp />);

    const composer = await screen.findByPlaceholderText("Ask Codex to do something...");
    await waitFor(() => expect((composer as HTMLTextAreaElement).disabled).toBe(false));
    fireEvent.change(composer, { target: { value: "Start from this workspace" } });
    fireEvent.click(screen.getByRole("button", { name: "Send" }));

    await waitFor(() => expect(client.startThread).toHaveBeenCalledWith("workspace-1"));
    await waitFor(() => expect(client.sendUserMessage).toHaveBeenCalledWith(
      "workspace-1",
      "thread-new",
      "Start from this workspace",
      null,
      null,
    ));
    expect(client.startThread.mock.invocationCallOrder[0]).toBeLessThan(
      client.sendUserMessage.mock.invocationCallOrder[0],
    );
    expect(screen.getAllByText("Thread").length).toBeGreaterThan(0);

    act(() => {
      appServerEventHandler?.({
        workspace_id: "workspace-1",
        message: {
          method: "thread/name/updated",
          params: { threadId: "thread-new", threadName: "Generated title" },
        },
      });
    });

    await waitFor(() => expect(screen.getAllByText("Generated title").length).toBeGreaterThan(0));
    expect(screen.queryByText("thread-n…")).toBeNull();
  });

  it("rolls back a Provider switch when its model catalog is empty", async () => {
    client.listModelProviders.mockResolvedValue({
      currentProviderId: "deepseek",
      currentModelId: "deepseek-v4-flash",
      data: [
        {
          id: "openai",
          name: "OpenAI",
          kind: "builtIn",
          isCurrent: false,
          modelCount: 0,
          models: [],
        },
        {
          id: "deepseek",
          name: "DeepSeek",
          kind: "custom",
          isCurrent: true,
          modelCount: 1,
          models: [{ modelId: "deepseek-v4-flash", showInPicker: true }],
        },
      ],
    });
    client.listModels.mockResolvedValueOnce({ data: [] });

    render(<WebApp />);

    fireEvent.click(await screen.findByRole("button", { name: "DeepSeek" }));
    fireEvent.click(screen.getByRole("button", { name: /Built-in providers/i }));
    fireEvent.click(screen.getByRole("button", { name: /OpenAI.*Built-in catalog/i }));

    await screen.findByText("This Provider has no selectable models");
    expect(client.writeModelProvider).toHaveBeenNthCalledWith(
      1,
      "workspace-1",
      { action: "select", id: "openai" },
    );
    expect(client.writeModelProvider).toHaveBeenNthCalledWith(
      2,
      "workspace-1",
      { action: "select", id: "deepseek" },
    );
  });

  it("opens a temporary Thread immediately and binds concurrent creations by temporary ID", async () => {
    const pending: Array<{
      resolve: (value: { thread: { id: string; name: string } }) => void;
      promise: Promise<{ thread: { id: string; name: string } }>;
    }> = [];
    client.startThread.mockImplementation(() => {
      let resolve!: (value: { thread: { id: string; name: string } }) => void;
      const promise = new Promise<{ thread: { id: string; name: string } }>((done) => {
        resolve = done;
      });
      pending.push({ resolve, promise });
      return promise;
    });
    render(<WebApp />);

    const createButton = await screen.findByRole("button", { name: "New thread in Demo" });
    await waitFor(() => expect((createButton as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(createButton);

    expect(screen.getByText("正在创建 Thread…")).toBeTruthy();
    expect(
      (screen.getByPlaceholderText("Ask Codex to do something...") as HTMLTextAreaElement).disabled,
    ).toBe(true);

    fireEvent.click(createButton);
    await waitFor(() => expect(client.startThread).toHaveBeenCalledTimes(2));

    act(() => pending[1].resolve({
      thread: { id: "thread-second", name: "Server second title" },
    }));
    await waitFor(() => expect(screen.queryByText("正在创建 Thread…")).toBeNull());
    await waitFor(() => expect(screen.getAllByText("Server second title")).toHaveLength(2));
    act(() => pending[0].resolve({
      thread: { id: "thread-first", name: "Server first title" },
    }));
    await waitFor(() => {
      expect(screen.getAllByText("Server second title")).toHaveLength(2);
      expect(screen.getByText("Server first title")).toBeTruthy();
    });

    const composer = screen.getByPlaceholderText("Ask Codex to do something...");
    await waitFor(() => expect((composer as HTMLTextAreaElement).disabled).toBe(false));
    fireEvent.change(composer, { target: { value: "Bound to the visible Thread" } });
    fireEvent.click(screen.getByRole("button", { name: "Send" }));

    await waitFor(() => expect(client.sendUserMessage).toHaveBeenCalledWith(
      "workspace-1",
      "thread-second",
      "Bound to the visible Thread",
      null,
      null,
    ));
  });

  it("keeps the returned Thread name while the task list still has the placeholder", async () => {
    client.listThreads
      .mockResolvedValueOnce({ data: [] })
      .mockResolvedValue({
        data: [{
          id: "thread-named",
          name: "Thread",
          cwd: "/tmp/demo",
          updatedAt: Date.now(),
          status: "idle",
        }],
      });
    client.startThread.mockResolvedValue({
      thread: { id: "thread-named", name: "Server generated title" },
    });
    render(<WebApp />);

    const createButton = await screen.findByRole("button", { name: "New thread in Demo" });
    await waitFor(() => expect((createButton as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(createButton);

    await waitFor(() => expect(screen.getAllByText("Server generated title")).toHaveLength(2));
    expect(screen.queryByText("正在创建 Thread…")).toBeNull();
  });

  it("replaces a new Thread placeholder with the name returned after its first message", async () => {
    client.sendUserMessage.mockResolvedValue({
      status: "sent",
      threadId: "thread-new",
      threadName: "Show Shanghai on a map",
      turn: { id: "turn-1", status: "inProgress" },
    });
    render(<WebApp />);

    const composer = await screen.findByPlaceholderText("Ask Codex to do something...");
    await waitFor(() => expect((composer as HTMLTextAreaElement).disabled).toBe(false));
    fireEvent.change(composer, { target: { value: "Show Shanghai on a map" } });
    fireEvent.click(screen.getByRole("button", { name: "Send" }));

    await waitFor(() => expect(client.sendUserMessage).toHaveBeenCalled());
    await waitFor(() => {
      expect(document.querySelector(".web-ws-thread-label")?.textContent)
        .toBe("Show Shanghai on a map");
      expect(document.querySelector(".web-chat-title")?.textContent)
        .toBe("Show Shanghai on a map");
    });
  });

  it("shows a retryable creation error and keeps the composer disabled", async () => {
    client.startThread
      .mockRejectedValueOnce(new Error("Thread startup failed"))
      .mockResolvedValueOnce({ thread: { id: "thread-retried" } });
    render(<WebApp />);

    const createButton = await screen.findByRole("button", { name: "New thread in Demo" });
    await waitFor(() => expect((createButton as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(createButton);

    await screen.findByText("创建 Thread 失败");
    expect(screen.getByText("Thread startup failed")).toBeTruthy();
    expect(
      (screen.getByPlaceholderText("Ask Codex to do something...") as HTMLTextAreaElement).disabled,
    ).toBe(true);

    fireEvent.click(screen.getByRole("button", { name: "重试" }));
    expect(screen.getByText("正在创建 Thread…")).toBeTruthy();
    await waitFor(() => expect(screen.queryByText("正在创建 Thread…")).toBeNull());
    expect(
      (screen.getByPlaceholderText("Ask Codex to do something...") as HTMLTextAreaElement).disabled,
    ).toBe(false);
  });

  it("keeps history hidden behind a loader until Thread hydration is complete", async () => {
    let resolveTurns!: (value: Record<string, unknown>[]) => void;
    client.listThreads.mockResolvedValue({
      data: [{
        id: "thread-first",
        name: "First thread",
        cwd: "/tmp/demo",
        updatedAt: Date.now(),
      }],
    });
    client.listThreadTurns.mockReturnValue(new Promise((resolve) => {
      resolveTurns = resolve;
    }));
    render(<WebApp />);

    fireEvent.click(await screen.findByText("First thread"));

    expect(screen.getByText("正在加载 Thread…")).toBeTruthy();
    expect(screen.queryByText("Hydrated history")).toBeNull();
    expect(
      (screen.getByPlaceholderText("Ask Codex to do something...") as HTMLTextAreaElement).disabled,
    ).toBe(true);

    act(() => {
      resolveTurns([{
        id: "turn-1",
        status: "completed",
        items: [{
          id: "assistant-1",
          type: "agentMessage",
          text: "Hydrated history",
        }],
      }]);
    });

    await screen.findByText("Hydrated history");
    await waitFor(() => expect(screen.queryByText("正在加载 Thread…")).toBeNull());
    expect(client.resumeThread).not.toHaveBeenCalled();
    expect(client.readThread).not.toHaveBeenCalled();
    expect(client.writeModelProvider).not.toHaveBeenCalled();
    expect(
      (screen.getByPlaceholderText("Ask Codex to do something...") as HTMLTextAreaElement).disabled,
    ).toBe(false);
  });

  it("does not block Thread history on a model catalog cache miss", async () => {
    let resolveModels!: (value: Record<string, unknown>) => void;
    client.listThreads.mockResolvedValue({
      data: [{
        id: "thread-first",
        name: "First thread",
        cwd: "/tmp/demo",
        updatedAt: Date.now(),
        modelProvider: "deepseek",
        model: "deepseek-v4-flash",
      }],
    });
    client.listThreadTurns.mockResolvedValue([{
      id: "turn-1",
      status: "completed",
      items: [{
        id: "assistant-1",
        type: "agentMessage",
        text: "History without catalog wait",
      }],
    }]);
    client.listModels.mockReturnValue(new Promise((resolve) => {
      resolveModels = resolve;
    }));
    render(<WebApp />);

    fireEvent.click(await screen.findByText("First thread"));

    await screen.findByText("History without catalog wait");
    await waitFor(() => expect(screen.queryByText("正在加载 Thread…")).toBeNull());
    expect(client.listModels).toHaveBeenCalledWith(
      "workspace-1",
      "deepseek",
      "deepseek-v4-flash",
    );
    expect(client.writeModelProvider).not.toHaveBeenCalled();
    expect(
      (screen.getByPlaceholderText("Ask Codex to do something...") as HTMLTextAreaElement).disabled,
    ).toBe(false);

    await act(async () => {
      resolveModels({
        data: [{
          id: "deepseek-v4-flash",
          model: "deepseek-v4-flash",
          displayName: "DeepSeek V4 Flash",
          isDefault: true,
        }],
      });
      await Promise.resolve();
    });
  });

  it("does not gate Thread hydration on slow Profile status reads", async () => {
    client.listThreads.mockResolvedValue({
      data: [{
        id: "thread-first",
        name: "First thread",
        cwd: "/tmp/demo",
        updatedAt: Date.now(),
        status: "idle",
      }],
    });
    client.listThreadTurns.mockResolvedValue([{
      id: "turn-1",
      status: "completed",
      items: [{
        id: "assistant-1",
        type: "agentMessage",
        text: "History before Profile status",
      }],
    }]);
    client.listMcpServerStatus.mockReturnValue(new Promise(() => undefined));
    client.getAccountRateLimits.mockReturnValue(new Promise(() => undefined));
    render(<WebApp />);

    fireEvent.click(await screen.findByText("First thread"));

    await screen.findByText("History before Profile status");
    await waitFor(() => expect(screen.queryByText("正在加载 Thread…")).toBeNull());
    expect(client.listMcpServerStatus).toHaveBeenCalledWith("workspace-1", "thread-first");
    expect(client.getAccountRateLimits).toHaveBeenCalledWith("workspace-1");
    expect(
      (screen.getByPlaceholderText("Ask Codex to do something...") as HTMLTextAreaElement).disabled,
    ).toBe(false);
  });

  it("reuses a completed Thread transcript when switching back to an unchanged Thread", async () => {
    client.listThreads.mockResolvedValue({
      data: [
        {
          id: "thread-first",
          name: "First thread",
          cwd: "/tmp/demo",
          updatedAt: 100,
          status: "idle",
        },
        {
          id: "thread-second",
          name: "Second thread",
          cwd: "/tmp/demo",
          updatedAt: 200,
          status: "idle",
        },
      ],
    });
    client.listThreadTurns.mockImplementation((_workspaceId: string, threadId: string) =>
      Promise.resolve([{
        id: `turn-${threadId}`,
        status: "completed",
        items: [{
          id: `assistant-${threadId}`,
          type: "agentMessage",
          text: threadId === "thread-first" ? "First history" : "Second history",
        }],
      }]));
    render(<WebApp />);

    fireEvent.click(await screen.findByText("First thread"));
    await screen.findByText("First history");
    await waitFor(() => expect(screen.queryByText("正在加载 Thread…")).toBeNull());

    fireEvent.click(screen.getByText("Second thread"));
    await screen.findByText("Second history");
    await waitFor(() => expect(screen.queryByText("正在加载 Thread…")).toBeNull());

    fireEvent.click(screen.getByText("First thread"));
    expect(screen.getByText("First history")).toBeTruthy();
    expect(screen.queryByText("正在加载 Thread…")).toBeNull();
    expect(client.listThreadTurns).toHaveBeenCalledTimes(2);
  });

  it("does not activate or render a replayed thread until the user selects it", async () => {
    client.listThreads.mockResolvedValue({
      data: [{
        id: "thread-first",
        name: "First thread",
        cwd: "/tmp/demo",
        updatedAt: Date.now(),
      }],
    });
    const view = render(<WebApp />);

    await waitFor(() => expect(client.listThreads).toHaveBeenCalledWith("workspace-1"));
    await screen.findByText("First thread");

    act(() => {
      appServerEventHandler?.({
        workspace_id: "workspace-1",
        message: {
          method: "item/completed",
          params: {
            threadId: "thread-first",
            item: {
              id: "assistant-item-1",
              type: "agentMessage",
              text: "Replayed first-thread content",
            },
          },
        },
      });
    });

    expect(screen.queryByText("Replayed first-thread content")).toBeNull();
    expect(view.container.querySelector(".web-ws-thread-active")).toBeNull();
  });

  it("clears Working and live item state after stopping succeeds", async () => {
    render(<WebApp />);

    const composer = await screen.findByPlaceholderText("Ask Codex to do something...");
    await waitFor(() => expect((composer as HTMLTextAreaElement).disabled).toBe(false));
    fireEvent.change(composer, { target: { value: "Run a long task" } });
    fireEvent.click(screen.getByRole("button", { name: "Send" }));
    await waitFor(() => expect(client.sendUserMessage).toHaveBeenCalled());

    act(() => {
      appServerEventHandler?.({
        workspace_id: "workspace-1",
        message: {
          method: "item/started",
          params: {
            threadId: "thread-new",
            turnId: "turn-1",
            item: {
              id: "command-1",
              type: "commandExecution",
              command: "sleep 30",
              status: "inProgress",
            },
          },
        },
      });
    });

    expect(screen.getByText("Working…")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Stop" }));

    await waitFor(() => expect(client.interruptTurn).toHaveBeenCalledWith(
      "workspace-1",
      "thread-new",
      "turn-1",
    ));
    await waitFor(() => expect(screen.queryByText("Working…")).toBeNull());

    act(() => {
      appServerEventHandler?.({
        workspace_id: "workspace-1",
        message: {
          method: "turn/completed",
          params: {
            threadId: "thread-new",
            turn: { id: "turn-1" },
          },
        },
      });
    });

    act(() => {
      appServerEventHandler?.({
        workspace_id: "workspace-1",
        message: {
          method: "thread/status/changed",
          params: {
            threadId: "thread-new",
            status: { type: "idle" },
          },
        },
      });
    });

    act(() => {
      appServerEventHandler?.({
        workspace_id: "workspace-1",
        message: {
          method: "item/started",
          params: {
            threadId: "thread-new",
            turnId: "turn-1",
            item: {
              id: "late-tool-1",
              type: "mcpToolCall",
              server: "workspace_maps",
              tool: "batch_geocode",
              status: "inProgress",
            },
          },
        },
      });
    });

    expect(screen.queryByText("Working…")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "1 tool call, 0 messages" }));
    expect(screen.getByText(/interrupted/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Send" })).toBeTruthy();
  });

  it("keeps phase from an empty agentMessage started item on later deltas", async () => {
    render(<WebApp />);

    const composer = await screen.findByPlaceholderText("Ask Codex to do something...");
    await waitFor(() => expect((composer as HTMLTextAreaElement).disabled).toBe(false));
    fireEvent.change(composer, { target: { value: "Write a Python script" } });
    fireEvent.click(screen.getByRole("button", { name: "Send" }));
    await waitFor(() => expect(client.sendUserMessage).toHaveBeenCalled());

    act(() => {
      appServerEventHandler?.({
        workspace_id: "workspace-1",
        message: {
          method: "item/started",
          params: {
            threadId: "thread-new",
            turnId: "turn-1",
            item: {
              id: "agent-message-1",
              type: "agentMessage",
              text: "",
              phase: "commentary",
            },
          },
        },
      });
      appServerEventHandler?.({
        workspace_id: "workspace-1",
        message: {
          method: "item/agentMessage/delta",
          params: {
            threadId: "thread-new",
            turnId: "turn-1",
            itemId: "agent-message-1",
            delta: "Preparing the Python script.",
          },
        },
      });
    });

    expect(document.querySelector(".web-execution-current .web-msg-commentary-body")).toBeTruthy();
    expect(screen.getAllByText("Preparing the Python script.")).toHaveLength(1);
  });

  it("merges an agentMessage started event into its existing streamed message", async () => {
    render(<WebApp />);

    const composer = await screen.findByPlaceholderText("Ask Codex to do something...");
    await waitFor(() => expect((composer as HTMLTextAreaElement).disabled).toBe(false));
    fireEvent.change(composer, { target: { value: "Show Shanghai" } });
    fireEvent.click(screen.getByRole("button", { name: "Send" }));
    await waitFor(() => expect(client.sendUserMessage).toHaveBeenCalled());

    act(() => {
      appServerEventHandler?.({
        workspace_id: "workspace-1",
        message: {
          method: "item/agentMessage/delta",
          params: {
            threadId: "thread-new",
            turnId: "turn-1",
            itemId: "agent-message-1",
            delta: "I will find the boundary data.",
          },
        },
      });
    });
    act(() => {
      appServerEventHandler?.({
        workspace_id: "workspace-1",
        message: {
          method: "item/started",
          params: {
            threadId: "thread-new",
            turnId: "turn-1",
            item: {
              id: "agent-message-1",
              type: "agentMessage",
              text: "I will find the boundary data.",
              phase: "commentary",
            },
          },
        },
      });
    });
    act(() => {
      appServerEventHandler?.({
        workspace_id: "workspace-1",
        message: {
          method: "item/completed",
          params: {
            threadId: "thread-new",
            turnId: "turn-1",
            item: {
              id: "agent-message-1",
              type: "agentMessage",
              text: "I will find the boundary data.",
              phase: "commentary",
            },
          },
        },
      });
    });

    expect(screen.getAllByText("I will find the boundary data.")).toHaveLength(1);

    act(() => {
      appServerEventHandler?.({
        workspace_id: "workspace-1",
        message: {
          method: "turn/completed",
          params: {
            threadId: "thread-new",
            turn: { id: "turn-1", status: "completed" },
          },
        },
      });
    });

    expect(screen.queryByText("I will find the boundary data.")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "0 tool calls, 1 message" }));
    expect(screen.getByText("I will find the boundary data.")).toBeTruthy();
  });
});
