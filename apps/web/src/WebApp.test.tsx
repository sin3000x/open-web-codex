// @vitest-environment jsdom

import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import WebApp, {
  providerCatalogFailureMessage,
  resolveTurnStartedAt,
} from "./WebApp";
import { PlatformRequestError } from "../browser/client";
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
  getSupervisorOverview: vi.fn(),
  runIdForThread: vi.fn(),
  listRunUserInputRequests: vi.fn(),
  listThreadUserInputRequests: vi.fn(),
  listRunMcpFormRequests: vi.fn(),
  listThreadMcpFormRequests: vi.fn(),
  respondToMcpForm: vi.fn(),
  startThread: vi.fn(),
  copilotProfileStatus: vi.fn(),
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

function warehouseCopilotStatus() {
  return {
    packages: [{
      packageId: "warehouse-network-copilot",
      available: true,
      displayName: "Warehouse Network · Multi-Agent",
    }],
    installations: [{
      packageId: "warehouse-network-copilot",
      sourceRevision: "revision-1",
      active: true,
      state: "configured",
      restartRequired: false,
      managedSkillIds: [],
      managedAgentRoleIds: [],
      runtimeDiscoveredSkillIds: [],
    }],
  };
}

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
    client.listModelProviders.mockResolvedValue({
      currentProviderId: "deepseek",
      currentModelId: "deepseek-v4-flash",
      data: [{
        id: "deepseek",
        name: "DeepSeek",
        kind: "custom",
        supportsFunctionTools: false,
        models: [{ modelId: "deepseek-v4-flash", showInPicker: true }],
      }],
    });
    client.listModels.mockResolvedValue({ data: [] });
    client.writeModelProvider.mockResolvedValue({ data: [] });
    client.selectProviderModel.mockResolvedValue({ data: [] });
    client.updateThreadModelSelection.mockResolvedValue({});
    client.listMcpServerStatus.mockResolvedValue({ data: [] });
    client.getAccountRateLimits.mockResolvedValue({});
    client.getSupervisorOverview.mockResolvedValue(null);
    client.runIdForThread.mockRejectedValue(new Error("Thread context lookup is still hydrating"));
    client.listRunUserInputRequests.mockResolvedValue([]);
    client.listThreadUserInputRequests.mockResolvedValue([]);
    client.listRunMcpFormRequests.mockResolvedValue([]);
    client.listThreadMcpFormRequests.mockResolvedValue([]);
    client.respondToMcpForm.mockResolvedValue(undefined);
    client.startThread.mockImplementation(async (
      _workspaceId: string,
      options: { onRunAccepted?: (value: { taskId: string; runId: string }) => void },
    ) => {
      options.onRunAccepted?.({ taskId: "task-new", runId: "run-new" });
      return { thread: { id: "thread-new" } };
    });
    client.copilotProfileStatus.mockResolvedValue({ packages: [], installations: [] });
    client.resumeThread.mockResolvedValue({ thread: { id: "thread-new", turns: [] } });
    client.listThreadTurns.mockResolvedValue([]);
    client.readThread.mockResolvedValue({ thread: { id: "thread-new", turns: [] } });
    client.sendUserMessage.mockResolvedValue({ turn: { id: "turn-1" } });
    client.interruptTurn.mockResolvedValue({ status: "interrupted" });
    client.respondToServerRequest.mockResolvedValue({});
  });

  it.each([
    ["authentication", "Provider authentication failed. Check its credential configuration."],
    ["not_found", "Provider model catalog was not found. Check the Provider URL."],
    ["rate_limited", "Provider model catalog is rate limited. Try again later."],
    ["upstream", "The provider could not return its model catalog. Try again later."],
    ["timeout", "Provider model catalog timed out. Try again."],
    ["network", "Provider model catalog is unreachable. Check the Provider URL."],
    ["invalid_json", "Provider returned invalid model catalog data. Check the Provider URL."],
    ["incompatible_schema", "Provider model catalog is incompatible. Check the Provider URL and schema."],
    ["empty_catalog", "Provider returned no usable models; the existing catalog was kept."],
  ] as const)("maps Provider catalog cause %s to a safe actionable message", (cause, message) => {
    expect(providerCatalogFailureMessage(new PlatformRequestError("secret runtime body", {
      providerCatalogFailure: cause,
    }))).toBe(message);
    expect(message).not.toContain("secret runtime body");
  });

  it("uses a fixed generic message for an unknown or missing Provider catalog cause", () => {
    expect(providerCatalogFailureMessage(new PlatformRequestError("secret runtime body", {
      providerCatalogFailure: null,
    }))).toBe("Unable to fetch Provider models. Try again.");
    expect(providerCatalogFailureMessage(new Error("secret runtime body"))).toBe(
      "Unable to fetch Provider models. Try again.",
    );
  });

  it("applies a successful Fetch snapshot directly without a stale catalog read", async () => {
    client.listModelProviders.mockResolvedValue({
      currentProviderId: "provider-1",
      currentModelId: "old-model",
      data: [{
        id: "provider-1",
        name: "Provider",
        kind: "custom",
        supportsFunctionTools: false,
        canFetchModels: true,
        models: [{ modelId: "old-model", showInPicker: true }],
      }],
    });
    client.writeModelProvider.mockResolvedValueOnce({
      currentProviderId: "provider-1",
      currentModelId: "new-model",
      data: [{
        id: "provider-1",
        name: "Provider",
        kind: "custom",
        supportsFunctionTools: false,
        canFetchModels: true,
        models: [{ modelId: "new-model", showInPicker: true }],
      }],
    });
    render(<WebApp />);

    fireEvent.click(await screen.findByRole("button", { name: "Provider" }));
    const dialog = await screen.findByRole("dialog", { name: "Providers and models" });
    expect(within(dialog).getAllByText("old-model").length).toBeGreaterThan(0);
    client.listModelProviders.mockClear();
    fireEvent.click(screen.getByRole("button", { name: "Fetch" }));

    await waitFor(() => expect(within(dialog).getAllByText("new-model").length).toBeGreaterThan(0));
    await waitFor(() => expect(client.writeModelProvider).toHaveBeenCalledTimes(1));
    expect(client.writeModelProvider).toHaveBeenCalledWith(
      { action: "fetch", id: "provider-1" },
    );
    expect(client.listModelProviders).not.toHaveBeenCalled();
    expect(screen.queryByText("Unable to fetch Provider models. Try again.")).toBeNull();
  });

  it("rejects a Provider catalog that omits the required function-tool declaration", async () => {
    client.listModelProviders.mockResolvedValue({
      currentProviderId: "provider-1",
      data: [{
        id: "provider-1",
        name: "Provider",
        kind: "custom",
        models: [],
      }],
    });
    render(<WebApp />);

    fireEvent.click(await screen.findByRole("button", { name: "Codex" }));
    await screen.findByText("Unable to load Provider settings. Try again.");
  });

  it("rejects a Provider catalog with a non-boolean function-tool declaration", async () => {
    client.listModelProviders.mockResolvedValue({
      currentProviderId: "provider-1",
      data: [{
        id: "provider-1",
        name: "Provider",
        kind: "custom",
        supportsFunctionTools: "true",
        models: [],
      }],
    });
    render(<WebApp />);

    fireEvent.click(await screen.findByRole("button", { name: "Codex" }));
    await screen.findByText("Unable to load Provider settings. Try again.");
  });

  it("preserves the catalog and hides raw error text when Fetch fails", async () => {
    client.listModelProviders.mockResolvedValue({
      currentProviderId: "provider-1",
      currentModelId: "old-model",
      data: [{
        id: "provider-1",
        name: "Provider",
        kind: "custom",
        supportsFunctionTools: false,
        canFetchModels: true,
        models: [{ modelId: "old-model", showInPicker: true }],
      }],
    });
    client.writeModelProvider.mockRejectedValueOnce(new PlatformRequestError(
      "secret provider response",
      { providerCatalogFailure: "authentication" },
    ));
    render(<WebApp />);

    fireEvent.click(await screen.findByRole("button", { name: "Provider" }));
    const dialog = await screen.findByRole("dialog", { name: "Providers and models" });
    expect(within(dialog).getAllByText("old-model").length).toBeGreaterThan(0);
    client.listModelProviders.mockClear();
    fireEvent.click(screen.getByRole("button", { name: "Fetch" }));

    await screen.findByText("Provider authentication failed. Check its credential configuration.");
    expect(within(dialog).getAllByText("old-model").length).toBeGreaterThan(0);
    expect(screen.getByRole("button", { name: "Provider" })).toBeTruthy();
    expect(screen.queryByText("secret provider response")).toBeNull();
    expect(client.listModelProviders).not.toHaveBeenCalled();
  });

  it("preserves the existing catalog for malformed, duplicate, empty, or stale current data", async () => {
    client.listModelProviders.mockResolvedValue({
      currentProviderId: "provider-1",
      currentModelId: "old-model",
      data: [{
        id: "provider-1",
        name: "Provider",
        kind: "custom",
        supportsFunctionTools: false,
        canFetchModels: true,
        models: [{ modelId: "old-model", showInPicker: true }],
      }],
    });
    client.writeModelProvider
      .mockResolvedValueOnce({
        currentProviderId: "provider-1",
        currentModelId: "new-model",
        data: [
          {
            id: "provider-1",
            name: "Provider",
            kind: "custom",
            supportsFunctionTools: false,
            canFetchModels: true,
            models: [{ modelId: "new-model", showInPicker: true }],
          },
          {
            id: "broken",
            name: "Broken",
            supportsFunctionTools: false,
            models: "not-an-array",
          },
        ],
      })
      .mockResolvedValueOnce({
        currentProviderId: "provider-1",
        currentModelId: "new-model",
        data: [
          {
            id: "provider-1",
            name: "Provider",
            kind: "custom",
            supportsFunctionTools: false,
            canFetchModels: true,
            models: [{ modelId: "new-model", showInPicker: true }],
          },
          {
            id: "provider-1",
            name: "Duplicate",
            kind: "custom",
            supportsFunctionTools: false,
            models: [{ modelId: "new-model", showInPicker: true }],
          },
        ],
      })
      .mockResolvedValueOnce({
        currentProviderId: "provider-1",
        currentModelId: "new-model",
        data: [{
          id: "provider-1",
          name: "Provider",
          kind: "custom",
          supportsFunctionTools: false,
          canFetchModels: true,
          models: [],
        }],
      })
      .mockResolvedValueOnce({
        currentProviderId: "missing-provider",
        currentModelId: "new-model",
        data: [{
          id: "provider-1",
          name: "Provider",
          kind: "custom",
          supportsFunctionTools: false,
          canFetchModels: true,
          models: [{ modelId: "new-model", showInPicker: true }],
        }],
      });
    render(<WebApp />);

    fireEvent.click(await screen.findByRole("button", { name: "Provider" }));
    const dialog = await screen.findByRole("dialog", { name: "Providers and models" });
    expect(within(dialog).getAllByText("old-model").length).toBeGreaterThan(0);
    const fetchButton = screen.getByRole("button", { name: "Fetch" });

    fireEvent.click(fetchButton);
    await screen.findByText("Unable to fetch Provider models. Try again.");
    expect(within(dialog).getAllByText("old-model").length).toBeGreaterThan(0);
    expect(screen.queryByText("new-model")).toBeNull();
    expect(screen.queryByText("Invalid Provider model catalog response")).toBeNull();

    fireEvent.click(fetchButton);
    await screen.findByText("Unable to fetch Provider models. Try again.");
    expect(within(dialog).getAllByText("old-model").length).toBeGreaterThan(0);
    expect(screen.queryByText("new-model")).toBeNull();

    fireEvent.click(fetchButton);
    await screen.findByText("Unable to fetch Provider models. Try again.");
    expect(within(dialog).getAllByText("old-model").length).toBeGreaterThan(0);
    expect(screen.queryByText("new-model")).toBeNull();

    fireEvent.click(fetchButton);
    await screen.findByText("Unable to fetch Provider models. Try again.");
    expect(within(dialog).getAllByText("old-model").length).toBeGreaterThan(0);
    expect(screen.getByRole("button", { name: "Provider" })).toBeTruthy();
  });

  it("preserves the catalog for duplicate models or a missing current model", async () => {
    client.listModelProviders.mockResolvedValue({
      currentProviderId: "provider-1",
      currentModelId: "old-model",
      data: [{
        id: "provider-1",
        name: "Provider",
        kind: "custom",
        supportsFunctionTools: false,
        canFetchModels: true,
        models: [{ modelId: "old-model", showInPicker: true }],
      }],
    });
    client.writeModelProvider
      .mockResolvedValueOnce({
        currentProviderId: "provider-1",
        currentModelId: "new-model",
        data: [{
          id: "provider-1",
          name: "Provider",
          kind: "custom",
          supportsFunctionTools: false,
          canFetchModels: true,
          models: [
            { modelId: "new-model", showInPicker: true },
            { modelId: "new-model", showInPicker: true },
          ],
        }],
      })
      .mockResolvedValueOnce({
        currentProviderId: "provider-1",
        currentModelId: "missing-current-model",
        data: [{
          id: "provider-1",
          name: "Provider",
          kind: "custom",
          supportsFunctionTools: false,
          canFetchModels: true,
          models: [{ modelId: "new-model", showInPicker: true }],
        }],
      });
    render(<WebApp />);

    fireEvent.click(await screen.findByRole("button", { name: "Provider" }));
    const dialog = await screen.findByRole("dialog", { name: "Providers and models" });
    const fetchButton = screen.getByRole("button", { name: "Fetch" });

    fireEvent.click(fetchButton);
    await waitFor(() => expect(client.writeModelProvider).toHaveBeenCalledTimes(1));
    await screen.findByText("Unable to fetch Provider models. Try again.");
    expect(within(dialog).getAllByText("old-model").length).toBeGreaterThan(0);
    expect(screen.queryByText("new-model")).toBeNull();
    expect(screen.getByRole("button", { name: "Provider" })).toBeTruthy();

    fireEvent.click(fetchButton);
    await waitFor(() => expect(client.writeModelProvider).toHaveBeenCalledTimes(2));
    await screen.findByText("Unable to fetch Provider models. Try again.");
    expect(within(dialog).getAllByText("old-model").length).toBeGreaterThan(0);
    expect(screen.queryByText("new-model")).toBeNull();
    expect(screen.getByRole("button", { name: "Provider" })).toBeTruthy();
  });

  it("rejects a Fetch snapshot that drops the active Thread Provider", async () => {
    client.listThreads.mockResolvedValue({
      data: [{
        id: "thread-first",
        name: "First thread",
        cwd: "/tmp/demo",
        updatedAt: Date.now(),
        modelProvider: "missing-provider",
        model: "thread-model",
      }],
    });
    client.listThreadTurns.mockResolvedValue([{
      id: "turn-1",
      status: "completed",
      items: [{ id: "assistant-1", type: "agentMessage", text: "Thread ready" }],
    }]);
    client.listModelProviders.mockResolvedValue({
      currentProviderId: "provider-1",
      currentModelId: "old-model",
      data: [{
        id: "provider-1",
        name: "Provider",
        kind: "custom",
        supportsFunctionTools: false,
        canFetchModels: true,
        models: [{ modelId: "old-model", showInPicker: true }],
      }],
    });
    client.writeModelProvider.mockResolvedValueOnce({
      currentProviderId: "provider-1",
      currentModelId: "new-model",
      data: [{
        id: "provider-1",
        name: "Provider",
        kind: "custom",
        supportsFunctionTools: false,
        canFetchModels: true,
        models: [{ modelId: "new-model", showInPicker: true }],
      }],
    });
    render(<WebApp />);

    fireEvent.click(await screen.findByText("First thread"));
    await screen.findByText("Thread ready");
    fireEvent.click(screen.getByRole("button", { name: "Codex" }));
    const dialog = await screen.findByRole("dialog", { name: "Providers and models" });
    expect(within(dialog).getAllByText("thread-model").length).toBeGreaterThan(0);
    fireEvent.click(screen.getByRole("button", { name: "Fetch" }));

    await screen.findByText("Unable to fetch Provider models. Try again.");
    expect(within(dialog).getAllByText("thread-model").length).toBeGreaterThan(0);
    expect(screen.queryByText("new-model")).toBeNull();
  });

  it("rejects a Fetch snapshot that drops the active Thread model", async () => {
    client.listThreads.mockResolvedValue({
      data: [{
        id: "thread-first",
        name: "First thread",
        cwd: "/tmp/demo",
        updatedAt: Date.now(),
        modelProvider: "provider-1",
        model: "missing-thread-model",
      }],
    });
    client.listThreadTurns.mockResolvedValue([{
      id: "turn-1",
      status: "completed",
      items: [{ id: "assistant-1", type: "agentMessage", text: "Thread ready" }],
    }]);
    client.listModelProviders.mockResolvedValue({
      currentProviderId: "provider-1",
      currentModelId: "old-model",
      data: [{
        id: "provider-1",
        name: "Provider",
        kind: "custom",
        supportsFunctionTools: false,
        canFetchModels: true,
        models: [{ modelId: "old-model", showInPicker: true }],
      }],
    });
    client.writeModelProvider.mockResolvedValueOnce({
      currentProviderId: "provider-1",
      currentModelId: "new-model",
      data: [{
        id: "provider-1",
        name: "Provider",
        kind: "custom",
        supportsFunctionTools: false,
        canFetchModels: true,
        models: [{ modelId: "new-model", showInPicker: true }],
      }],
    });
    render(<WebApp />);

    fireEvent.click(await screen.findByText("First thread"));
    await screen.findByText("Thread ready");
    fireEvent.click(screen.getByRole("button", { name: "Provider" }));
    const dialog = await screen.findByRole("dialog", { name: "Providers and models" });
    expect(within(dialog).getAllByText("old-model").length).toBeGreaterThan(0);
    fireEvent.click(screen.getByRole("button", { name: "Fetch" }));

    await waitFor(() => expect(client.writeModelProvider).toHaveBeenCalledTimes(1));
    await screen.findByText("Unable to fetch Provider models. Try again.");
    expect(within(dialog).getAllByText("old-model").length).toBeGreaterThan(0);
    expect(screen.queryByText("new-model")).toBeNull();
    expect(screen.getByRole("button", { name: "Provider" })).toBeTruthy();
  });

  it("keeps a local send time when Runtime reports a different Turn start", () => {
    expect(resolveTurnStartedAt(1_700_000_120_000, 1_700_000_000)).toBe(1_700_000_120_000);
    expect(resolveTurnStartedAt(null, 1_700_000_000)).toBe(1_700_000_000_000);
    expect(resolveTurnStartedAt(null, null, 1_700_000_120_000)).toBe(1_700_000_120_000);
  });

  it("persists a Profile Provider without requiring a Workspace", async () => {
    client.listWorkspaces.mockResolvedValue([]);
    client.listModelProviders.mockResolvedValue({
      currentProviderId: "openai",
      data: [{
        id: "openai",
        name: "OpenAI",
        kind: "builtIn",
        supportsFunctionTools: true,
        models: [],
      }],
    });
    client.writeModelProvider.mockResolvedValueOnce({
      currentProviderId: "deepseek",
      currentModelId: "deepseek-v4-flash",
      data: [{
        id: "deepseek",
        name: "DeepSeek",
        kind: "custom",
        supportsFunctionTools: false,
        models: [{ modelId: "deepseek-v4-flash", showInPicker: true }],
      }],
    });
    render(<WebApp />);

    const catalog = await screen.findByRole("dialog", { name: "Providers and models" });
    fireEvent.click(within(catalog).getByRole("button", { name: "Add" }));
    const editor = within(screen.getByRole("dialog", { name: "Add provider" }));
    fireEvent.change(editor.getByLabelText("ID"), { target: { value: "deepseek" } });
    fireEvent.change(editor.getByLabelText("Name"), { target: { value: "DeepSeek" } });
    fireEvent.change(editor.getByLabelText("Base URL"), {
      target: { value: "https://api.deepseek.com" },
    });
    fireEvent.change(editor.getByLabelText("Credential source"), {
      target: { value: "none" },
    });
    fireEvent.click(editor.getByRole("button", { name: "Save provider" }));

    await waitFor(() => expect(client.writeModelProvider).toHaveBeenCalledWith({
      action: "upsert",
      id: "deepseek",
      name: "DeepSeek",
      baseUrl: "https://api.deepseek.com",
      credentialMode: "none",
      envKey: "",
      apiKey: "",
      wireApi: "responses",
      select: true,
    }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Add provider" })).toBeNull());
    expect(await screen.findByRole("button", { name: "DeepSeek" })).toBeTruthy();
  });

  it("keeps the Provider editor open and shows a safe error when Profile persistence fails", async () => {
    client.listWorkspaces.mockResolvedValue([]);
    client.listModelProviders.mockResolvedValue({
      currentProviderId: "openai",
      data: [{
        id: "openai",
        name: "OpenAI",
        kind: "builtIn",
        supportsFunctionTools: true,
        models: [],
      }],
    });
    client.writeModelProvider.mockRejectedValueOnce(new Error("credential-canary"));
    render(<WebApp />);

    const catalog = await screen.findByRole("dialog", { name: "Providers and models" });
    fireEvent.click(within(catalog).getByRole("button", { name: "Add" }));
    const dialog = screen.getByRole("dialog", { name: "Add provider" });
    const editor = within(dialog);
    fireEvent.change(editor.getByLabelText("ID"), { target: { value: "deepseek" } });
    fireEvent.change(editor.getByLabelText("Name"), { target: { value: "DeepSeek" } });
    fireEvent.change(editor.getByLabelText("Base URL"), {
      target: { value: "https://api.deepseek.com" },
    });
    fireEvent.change(editor.getByLabelText("Credential source"), {
      target: { value: "none" },
    });
    fireEvent.click(editor.getByRole("button", { name: "Save provider" }));

    await screen.findByText("Unable to save Provider settings. Try again.");
    expect(screen.getByRole("dialog", { name: "Add provider" })).toBe(dialog);
    expect(screen.queryByText("credential-canary")).toBeNull();
  });

  it("opens the Provider catalog and does not create a Thread without a valid model", async () => {
    client.listModelProviders.mockResolvedValue({ data: [] });
    render(<WebApp />);

    const catalog = await screen.findByRole("dialog", { name: "Providers and models" });
    expect(within(catalog).getByText("Select a Provider and model before starting.")).toBeTruthy();
    const send = screen.getByRole("button", { name: "Send" });
    expect((send as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(send);
    fireEvent.click(screen.getByRole("button", { name: "New task in Demo" }));
    expect(client.startThread).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog", { name: "Providers and models" })).toBe(catalog);
  });

  it("creates a thread before sending when only a workspace is selected", async () => {
    client.listModelProviders.mockResolvedValue({
      currentProviderId: "deepseek",
      currentModelId: "deepseek-v4-flash",
      data: [{
        id: "deepseek",
        name: "DeepSeek",
        kind: "custom",
        supportsFunctionTools: false,
        models: [{ modelId: "deepseek-v4-flash", showInPicker: true }],
      }],
    });
    render(<WebApp />);

    const composer = await screen.findByPlaceholderText("Ask Codex to do something...");
    await waitFor(() => expect((composer as HTMLTextAreaElement).disabled).toBe(false));
    fireEvent.change(composer, { target: { value: "Start from this workspace" } });
    fireEvent.click(screen.getByRole("button", { name: "Send" }));

    await waitFor(() => expect(client.startThread).toHaveBeenCalledWith(
      "workspace-1",
      expect.objectContaining({
        operationId: expect.any(String),
        onRunAccepted: expect.any(Function),
      }),
    ));
    await waitFor(() => expect(client.sendUserMessage).toHaveBeenCalledWith(
      "workspace-1",
      "thread-new",
      "Start from this workspace",
      "deepseek-v4-flash",
      "deepseek",
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

  it("keeps the Agents tab available and shows an empty state before collaboration starts", async () => {
    render(<WebApp />);

    fireEvent.click(await screen.findByRole("button", { name: "File manager" }));
    const agentsTab = await screen.findByRole("tab", { name: "Agents" });
    expect((agentsTab as HTMLButtonElement).disabled).toBe(false);

    fireEvent.click(agentsTab);

    await waitFor(() => expect(agentsTab.getAttribute("aria-selected")).toBe("true"));
    expect(screen.getByText("No Agent activity yet")).toBeTruthy();
  });

  it("keeps Agent activity live while Files is selected and marks it unread", async () => {
    const rootAgent = {
      run_id: "run-enterprise",
      thread_id: "thread-new",
      parent_thread_id: null,
      source_kind: "root",
      agent_path: null,
      agent_nickname: null,
      agent_role: null,
      status_type: "active",
      active_flags: [],
      is_root: true,
      first_observed_at: "2026-07-26T00:00:01Z",
      last_observed_at: "2026-07-26T00:00:01Z",
    };
    const dataAgent = {
      ...rootAgent,
      thread_id: "data-thread",
      parent_thread_id: "thread-new",
      source_kind: "thread_spawn",
      agent_path: "/root/data",
      agent_nickname: "Data Analyst",
      agent_role: "data_agent",
      is_root: false,
      first_observed_at: "2026-07-26T00:00:02Z",
    };
    const assignment = {
      run_id: "run-enterprise",
      sequence: 4,
      thread_id: "data-thread",
      turn_id: null,
      item_id: "spawn-data",
      kind: "assignment",
      status: "pending",
      title: "Task assigned",
      detail: "Inspect enterprise planning data.",
      created_at: "2026-07-26T00:00:02Z",
    };
    const baseOverview = {
      taskTitle: "Enterprise network planning",
      policy: null,
      agents: [rootAgent, dataAgent],
      activities: [assignment],
      executions: [{
        id: "execution-data-1",
        run_id: "run-enterprise",
        thread_id: "data-thread",
        turn_id: null,
        ordinal: 1,
        task: "Inspect enterprise planning data.",
        status: "pending",
        current_behavior: "Waiting to start",
        latest_progress: null,
        first_observed_sequence: 4,
        last_observed_sequence: 4,
        started_at: null,
        completed_at: null,
        created_at: "2026-07-26T00:00:02Z",
        updated_at: "2026-07-26T00:00:02Z",
      }],
      artifacts: [],
    };
    client.listThreads.mockResolvedValue({
      data: [{
        id: "thread-new",
        name: "Supervisor case",
        cwd: "/tmp/demo",
        status: "idle",
        updatedAt: "2026-07-26T00:00:02Z",
      }],
    });
    client.getSupervisorOverview.mockResolvedValue(baseOverview);
    render(<WebApp />);

    fireEvent.click(await screen.findByText("Supervisor case"));
    await waitFor(() => expect(client.getSupervisorOverview)
      .toHaveBeenCalledWith("thread-new"));
    const agentButton = await screen.findByRole("button", { name: "Agent activity" });
    await waitFor(() => expect((agentButton as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(screen.getByRole("button", { name: "File manager" }));
    await waitFor(() => expect(
      screen.getByRole("button", { name: "File manager" }).getAttribute("aria-pressed"),
    ).toBe("true"));
    await act(async () => {
      await new Promise((resolve) => window.setTimeout(resolve, 180));
    });

    const previousRefreshes = client.getSupervisorOverview.mock.calls.length;
    client.getSupervisorOverview.mockResolvedValue({
      ...baseOverview,
      activities: [
        assignment,
        {
          ...assignment,
          sequence: 5,
          turn_id: "turn-data",
          item_id: null,
          kind: "turn_started",
          status: "running",
          title: "Started working",
          detail: null,
          created_at: "2026-07-26T00:00:03Z",
        },
        {
          ...assignment,
          sequence: 6,
          turn_id: "turn-data",
          item_id: "tool-data",
          kind: "tool_started",
          status: "running",
          title: "Using planning data · load network",
          detail: null,
          created_at: "2026-07-26T00:00:04Z",
        },
      ],
      executions: [{
        ...baseOverview.executions[0],
        turn_id: "turn-data",
        status: "running",
        current_behavior: "Using planning data · load network",
        last_observed_sequence: 6,
        started_at: "2026-07-26T00:00:03Z",
        updated_at: "2026-07-26T00:00:04Z",
      }],
    });
    act(() => {
      appServerEventHandler?.({
        workspace_id: "workspace-1",
        message: {
          method: "item/started",
          params: {
            threadId: "data-thread",
            turnId: "turn-data",
            item: {
              id: "tool-data",
              type: "mcpToolCall",
              server: "planning_data",
              tool: "load_network",
              status: "inProgress",
            },
          },
        },
      });
    });

    await waitFor(() => expect(
      client.getSupervisorOverview.mock.calls.length,
    ).toBeGreaterThan(previousRefreshes));
    expect(await screen.findAllByLabelText("New Agent activity")).toHaveLength(2);

    fireEvent.click(agentButton);
    expect(await screen.findAllByText("Using planning data · load network")).toHaveLength(2);
    expect(screen.queryByLabelText("New Agent activity")).toBeNull();
  });

  it("renders consecutive redacted command approvals without refreshing", async () => {
    render(<WebApp />);
    const composer = await screen.findByPlaceholderText("Ask Codex to do something...");
    await waitFor(() => expect((composer as HTMLTextAreaElement).disabled).toBe(false));
    fireEvent.change(composer, { target: { value: "Show Jakarta" } });
    fireEvent.click(screen.getByRole("button", { name: "Send" }));
    await waitFor(() => expect(client.sendUserMessage).toHaveBeenCalled());
    for (const id of ["approval-ls", "approval-bash"]) {
      act(() => appServerEventHandler?.({
        workspace_id: "workspace-1",
        message: {
          method: "item/commandExecution/requestApproval",
          id,
          // Production events omit raw command text and cwd.
          params: { threadId: "thread-new", turnId: "turn-1", itemId: id },
        },
      }));
      fireEvent.click(await screen.findByRole("button", { name: "Accept" }));
      await waitFor(() => expect(client.respondToServerRequest).toHaveBeenCalledWith(
        "workspace-1", id, { decision: "accept" },
      ));
      act(() => appServerEventHandler?.({
        workspace_id: "workspace-1",
        message: {
          method: "serverRequest/resolved",
          params: { threadId: "thread-new", requestId: id },
        },
      }));
    }
    act(() => appServerEventHandler?.({
      workspace_id: "workspace-1",
      message: {
        method: "turn/completed",
        params: {
          threadId: "thread-new",
          turn: { id: "turn-1", status: "failed", error: {
            message: "The upstream provider returned an error.",
          } },
        },
      },
    }));
    expect(await screen.findByText("The upstream provider returned an error.")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Stop" })).toBeNull();
  });

  it("surfaces a child Agent approval in the root task and Agent activity", async () => {
    const rootAgent = {
      run_id: "run-enterprise",
      thread_id: "thread-root",
      parent_thread_id: null,
      source_kind: "root",
      agent_path: null,
      agent_nickname: null,
      agent_role: null,
      status_type: "active",
      active_flags: [],
      is_root: true,
      first_observed_at: "2026-07-26T00:00:01Z",
      last_observed_at: "2026-07-26T00:00:01Z",
    };
    const dataAgent = {
      ...rootAgent,
      thread_id: "thread-data",
      parent_thread_id: "thread-root",
      source_kind: "thread_spawn",
      agent_nickname: "Data Analyst",
      agent_role: "data_agent",
      is_root: false,
    };
    client.listThreads.mockResolvedValue({
      data: [{
        id: "thread-root",
        name: "Supervisor case",
        cwd: "/tmp/demo",
        status: "active",
        updatedAt: "2026-07-26T00:00:02Z",
      }],
    });
    client.getSupervisorOverview.mockResolvedValue({
      taskTitle: "Enterprise network planning",
      policy: null,
      agents: [rootAgent, dataAgent],
      activities: [],
      executions: [],
      artifacts: [],
    });
    render(<WebApp />);

    fireEvent.click(await screen.findByText("Supervisor case"));
    await waitFor(() => expect(client.getSupervisorOverview)
      .toHaveBeenCalledWith("thread-root"));

    act(() => {
      appServerEventHandler?.({
        workspace_id: "workspace-1",
        message: {
          method: "item/commandExecution/requestApproval",
          id: "approval-child-1",
          params: {
            threadId: "thread-data",
            turnId: "turn-data",
            serverName: "supply_chain_data",
          },
        },
      });
    });

    const taskQueue = await screen.findByRole("region", { name: "Task approvals" });
    expect(taskQueue.textContent).toContain("Data Analyst");
    expect(taskQueue.textContent).toContain("Approval required");
    expect(await screen.findAllByLabelText("New Agent activity")).toHaveLength(2);

    fireEvent.click(within(taskQueue).getByRole("button", { name: "Accept" }));
    await waitFor(() => expect(client.respondToServerRequest).toHaveBeenCalledWith(
      "workspace-1",
      "approval-child-1",
      { decision: "accept" },
    ));

    fireEvent.click(screen.getByRole("button", { name: "Agent activity" }));
    const agentQueue = await screen.findByRole("region", { name: "Agent approvals" });
    expect(agentQueue.textContent).toContain("Data Analyst");
    expect(agentQueue.textContent).toContain("Approval resolved");
    expect(within(agentQueue).queryByRole("button", { name: "Accept" })).toBeNull();
  });

  it("shows a child MCP form from its owning Run without waiting for a Thread lookup", async () => {
    client.listThreads.mockResolvedValue({
      data: [{
        id: "thread-root",
        name: "Supervisor case",
        cwd: "/tmp/demo",
        status: "active",
        updatedAt: "2026-08-10T00:00:02Z",
      }],
    });
    client.listRunMcpFormRequests.mockResolvedValue([{
      id: "approval-mcp-child-1",
      runId: "run-enterprise",
      source: {
        kind: "agent",
        executionId: "execution-data-1",
        displayTitle: "Wanwan",
      },
      serverName: "supply_chain",
      message: "Allow the supply_chain MCP server to run tool discover_workspace_sources?",
      fields: [],
      state: "pending",
      version: 1,
      createdAt: "2026-08-10T00:00:03Z",
    }]);
    render(<WebApp />);

    fireEvent.click(await screen.findByText("Supervisor case"));
    await waitFor(() => expect(client.listThreadMcpFormRequests)
      .toHaveBeenCalledWith("thread-root"));
    await waitFor(() => expect(screen.queryByText("正在加载 Thread…")).toBeNull());

    act(() => {
      appServerEventHandler?.({
        workspace_id: "workspace-1",
        run_id: "run-enterprise",
        root_thread_id: "thread-root",
        message: {
          method: "platform/mcpFormRequested",
          params: {
            threadId: "thread-data",
            turnId: "turn-data",
            runId: "run-enterprise",
            approvalId: "approval-mcp-child-1",
          },
        },
      });
    });

    expect(await screen.findByText(/Allow the supply_chain MCP server/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Approve" })).toBeTruthy();
    expect(client.runIdForThread).not.toHaveBeenCalled();
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
          supportsFunctionTools: true,
          isCurrent: false,
          modelCount: 0,
          models: [],
        },
        {
          id: "deepseek",
          name: "DeepSeek",
          kind: "custom",
          supportsFunctionTools: false,
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
      { action: "select", id: "openai" },
    );
    expect(client.writeModelProvider).toHaveBeenNthCalledWith(
      2,
      { action: "select", id: "deepseek" },
    );
  });

  it("shows a temporary Thread only after the platform accepts the Run", async () => {
    let acceptRun!: () => void;
    let resolveThread!: (
      value: { thread: { id: string; name: string } },
    ) => void;
    client.startThread.mockImplementation((
      _workspaceId: string,
      options: {
        onRunAccepted?: (value: { taskId: string; runId: string }) => void;
      },
    ) => {
      acceptRun = () =>
        options.onRunAccepted?.({ taskId: "task-1", runId: "run-1" });
      return new Promise((resolve) => {
        resolveThread = resolve;
      });
    });
    render(<WebApp />);

    fireEvent.click(
      await screen.findByRole("button", { name: "New task in Demo" }),
    );

    await waitFor(() => expect(client.startThread).toHaveBeenCalledTimes(1));
    expect(screen.queryByText("正在创建 Thread…")).toBeNull();

    act(() => acceptRun());
    expect(screen.getByText("正在创建 Thread…")).toBeTruthy();
    expect(
      (screen.getByPlaceholderText(
        "Ask Codex to do something...",
      ) as HTMLTextAreaElement).disabled,
    ).toBe(true);

    act(() =>
      resolveThread({
        thread: { id: "thread-accepted", name: "Accepted task" },
      }),
    );
    await waitFor(() => expect(screen.queryByText("正在创建 Thread…")).toBeNull());
    await waitFor(() =>
      expect(screen.getAllByText("Accepted task")).toHaveLength(2),
    );
  });

  it("asks for the Copilot before creating the task for a first message", async () => {
    client.copilotProfileStatus.mockResolvedValue(warehouseCopilotStatus());
    render(<WebApp />);

    const composer = await screen.findByPlaceholderText("Ask Codex to do something...");
    await waitFor(() => expect((composer as HTMLTextAreaElement).disabled).toBe(false));
    await waitFor(() => expect(client.copilotProfileStatus).toHaveBeenCalled());
    fireEvent.change(composer, { target: { value: "计算 12 小时时效达标率" } });
    fireEvent.click(screen.getByRole("button", { name: "Send" }));

    const dialog = await screen.findByRole("dialog", { name: "Choose a Copilot" });
    expect(client.startThread).not.toHaveBeenCalled();
    fireEvent.click(within(dialog).getByRole("button", { name: "Create Thread" }));

    await waitFor(() => expect(client.startThread).toHaveBeenCalledWith(
      "workspace-1",
      expect.objectContaining({ copilot: { packageId: "warehouse-network-copilot" } }),
    ));
    await waitFor(() => expect(client.sendUserMessage).toHaveBeenCalledWith(
      "workspace-1",
      "thread-new",
      "计算 12 小时时效达标率",
      "deepseek-v4-flash",
      "deepseek",
      null,
    ));
    expect((composer as HTMLTextAreaElement).value).toBe("");
  });

  it("keeps the first message when the Copilot choice is cancelled", async () => {
    client.copilotProfileStatus.mockResolvedValue(warehouseCopilotStatus());
    render(<WebApp />);

    const composer = await screen.findByPlaceholderText("Ask Codex to do something...");
    await waitFor(() => expect((composer as HTMLTextAreaElement).disabled).toBe(false));
    await waitFor(() => expect(client.copilotProfileStatus).toHaveBeenCalled());
    fireEvent.change(composer, { target: { value: "计算 12 小时时效达标率" } });
    fireEvent.click(screen.getByRole("button", { name: "Send" }));
    const dialog = await screen.findByRole("dialog", { name: "Choose a Copilot" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));

    expect(screen.queryByRole("dialog", { name: "Choose a Copilot" })).toBeNull();
    expect((composer as HTMLTextAreaElement).value).toBe("计算 12 小时时效达标率");

    fireEvent.click(screen.getByRole("button", { name: "New task in Demo" }));
    fireEvent.click(
      within(await screen.findByRole("dialog", { name: "Choose a Copilot" }))
        .getByRole("button", { name: "Create Thread" }),
    );
    await waitFor(() => expect(client.startThread).toHaveBeenCalledTimes(1));
    expect(client.sendUserMessage).not.toHaveBeenCalled();
  });

  it("retries a failed Thread creation with its original Copilot", async () => {
    client.copilotProfileStatus.mockResolvedValue(warehouseCopilotStatus());
    client.startThread
      .mockImplementationOnce(async (
        _workspaceId: string,
        options: {
          onRunAccepted?: (value: { taskId: string; runId: string }) => void;
        },
      ) => {
        options.onRunAccepted?.({ taskId: "task-failed", runId: "run-failed" });
        throw new Error("Thread startup failed");
      });
    render(<WebApp />);

    await waitFor(() => expect(client.copilotProfileStatus).toHaveBeenCalled());
    fireEvent.click(await screen.findByRole("button", { name: "New task in Demo" }));
    fireEvent.click(
      within(await screen.findByRole("dialog", { name: "Choose a Copilot" }))
        .getByRole("button", { name: "Create Thread" }),
    );
    await waitFor(() => expect(client.startThread).toHaveBeenCalledTimes(1));

    fireEvent.click(await screen.findByRole("button", { name: /重试|Retry/ }));

    await waitFor(() => expect(client.startThread).toHaveBeenCalledTimes(2));
    expect(client.startThread.mock.calls[1][1]).toEqual(expect.objectContaining({
      copilot: { packageId: "warehouse-network-copilot" },
    }));
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
    client.startThread.mockImplementation(async (
      _workspaceId: string,
      options: {
        onRunAccepted?: (value: { taskId: string; runId: string }) => void;
      },
    ) => {
      options.onRunAccepted?.({ taskId: "task-named", runId: "run-named" });
      return {
        thread: { id: "thread-named", name: "Server generated title" },
      };
    });
    render(<WebApp />);

    fireEvent.click(
      await screen.findByRole("button", { name: "New task in Demo" }),
    );

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

  it("allows a direct Standard start to be retried after admission fails", async () => {
    client.startThread
      .mockRejectedValueOnce(new Error("Thread startup failed"))
      .mockImplementationOnce(async (
        _workspaceId: string,
        options: {
          onRunAccepted?: (value: { taskId: string; runId: string }) => void;
        },
      ) => {
        options.onRunAccepted?.({ taskId: "task-retried", runId: "run-retried" });
        return { thread: { id: "thread-retried" } };
      });
    render(<WebApp />);

    fireEvent.click(
      await screen.findByRole("button", { name: "New task in Demo" }),
    );

    await screen.findByText("Thread startup failed");
    expect(screen.queryByText("正在创建 Thread…")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "New task in Demo" }));
    await waitFor(() => expect(client.startThread).toHaveBeenCalledTimes(2));
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
    client.listModelProviders.mockResolvedValue({ data: [] });
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

  it("restores the selected root Thread and its replayed child approval after remount", async () => {
    const rootAgent = {
      run_id: "run-enterprise",
      thread_id: "thread-root",
      parent_thread_id: null,
      source_kind: "root",
      agent_path: null,
      agent_nickname: null,
      agent_role: null,
      status_type: "active",
      active_flags: [],
      is_root: true,
      first_observed_at: "2026-07-26T00:00:01Z",
      last_observed_at: "2026-07-26T00:00:01Z",
    };
    const dataAgent = {
      ...rootAgent,
      thread_id: "thread-data",
      parent_thread_id: "thread-root",
      source_kind: "thread_spawn",
      agent_nickname: "Data Analyst",
      agent_role: "data_agent",
      is_root: false,
    };
    client.listThreads.mockResolvedValue({
      data: [{
        id: "thread-root",
        name: "Supervisor case",
        cwd: "/tmp/demo",
        status: "active",
        updatedAt: "2026-07-26T00:00:02Z",
      }],
    });
    client.getSupervisorOverview.mockResolvedValue({
      taskTitle: "Enterprise network planning",
      policy: null,
      agents: [rootAgent, dataAgent],
      activities: [],
      executions: [],
      artifacts: [],
    });

    const first = render(<WebApp />);
    fireEvent.click(await screen.findByText("Supervisor case"));
    await waitFor(() => expect(window.sessionStorage.getItem(
      "open-web-codex:active-thread:v1:workspace-1",
    )).toBe("thread-root"));
    first.unmount();

    render(<WebApp />);
    act(() => {
      appServerEventHandler?.({
        workspace_id: "workspace-1",
        message: {
          method: "item/commandExecution/requestApproval",
          id: "approval-child-restored",
          params: {
            threadId: "thread-data",
            turnId: "turn-data",
            serverName: "supply_chain_data",
            command: "Allow supply_chain_data to publish the report?",
          },
        },
      });
    });

    await waitFor(() => expect(client.listThreadTurns).toHaveBeenCalledWith(
      "workspace-1",
      "thread-root",
    ));
    await waitFor(() => expect(client.getSupervisorOverview).toHaveBeenCalledWith("thread-root"));
    const taskQueue = await screen.findByRole("region", { name: "Task approvals" });
    expect(taskQueue.textContent).toContain("Data Analyst");
    expect(within(taskQueue).getByRole("button", { name: "Accept" })).toBeTruthy();
  });

  it("clears a stored Thread that is not in the authorized Workspace listing", async () => {
    window.sessionStorage.setItem("open-web-codex:active-workspace:v1", "workspace-1");
    window.sessionStorage.setItem(
      "open-web-codex:active-thread:v1:workspace-1",
      "thread-missing",
    );
    client.listThreads.mockResolvedValue({
      data: [{
        id: "thread-first",
        name: "First thread",
        cwd: "/tmp/demo",
        updatedAt: Date.now(),
      }],
    });

    const view = render(<WebApp />);
    await screen.findByText("First thread");
    await waitFor(() => expect(window.sessionStorage.getItem(
      "open-web-codex:active-thread:v1:workspace-1",
    )).toBeNull());
    expect(view.container.querySelector(".web-ws-thread-active")).toBeNull();
    expect(client.listThreadTurns).not.toHaveBeenCalled();
  });

  it("converges an interrupted replayed Turn to the Runtime Thread idle status", async () => {
    client.listThreads.mockResolvedValue({
      data: [{
        id: "thread-recovery",
        name: "Recovering thread",
        cwd: "/tmp/demo",
        updatedAt: Date.now(),
        status: "idle",
      }],
    });
    render(<WebApp />);

    await screen.findByText("Recovering thread");
    act(() => {
      appServerEventHandler?.({
        workspace_id: "workspace-1",
        message: {
          method: "turn/started",
          params: {
            threadId: "thread-recovery",
            turn: { id: "turn-before-restart" },
          },
        },
      });
    });
    expect(screen.getByRole("status", { name: "Thread is running" })).toBeTruthy();

    act(() => {
      appServerEventHandler?.({
        workspace_id: "workspace-1",
        message: {
          method: "thread/status/changed",
          params: {
            threadId: "thread-recovery",
            status: { type: "idle" },
          },
        },
      });
    });

    await waitFor(() => {
      expect(screen.queryByRole("status", { name: "Thread is running" })).toBeNull();
    });
    expect((screen.getByRole("button", {
      name: "Archive thread Recovering thread",
    }) as HTMLButtonElement).disabled).toBe(false);
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
            turn: { id: "turn-1", status: "completed", durationMs: 48_318 },
          },
        },
      });
    });

    expect(screen.queryByText("I will find the boundary data.")).toBeNull();
    fireEvent.click(screen.getByRole("button", {
      name: "0 tool calls, 1 message · 0:48",
    }));
    expect(screen.getByText("I will find the boundary data.")).toBeTruthy();
  });
});
