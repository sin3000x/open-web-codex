// @vitest-environment jsdom

import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
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
  listMcpServerStatus: vi.fn(),
  getAccountRateLimits: vi.fn(),
  startThread: vi.fn(),
  sendUserMessage: vi.fn(),
};

vi.mock("./services/webClient", () => ({
  CodexMonitorWebClient: vi.fn(() => client),
}));

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
    client.listMcpServerStatus.mockResolvedValue({ data: [] });
    client.getAccountRateLimits.mockResolvedValue({});
    client.startThread.mockResolvedValue({ thread: { id: "thread-new" } });
    client.sendUserMessage.mockResolvedValue({ turn: { id: "turn-1" } });
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
});
