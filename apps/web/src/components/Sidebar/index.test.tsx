// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import Sidebar from "./index";

describe("Sidebar settings", () => {
  afterEach(cleanup);
  it("opens settings in a dialog and closes it with Escape", () => {
    render(
      <Sidebar
        gatewayState="online"
        gatewayVersion="1.0.0"
        workspaces={[]}
        activeWorkspaceId={null}
        onSelectWorkspace={vi.fn()}
        onCreateWorkspace={vi.fn()}
        onLoadWorkspaces={vi.fn()}
        onConnectWorkspace={vi.fn()}
        threadsByWorkspace={{}}
        activeThreadId={null}
        onSelectThread={vi.fn()}
        onNewThread={vi.fn()}
        onArchiveThread={vi.fn()}
        onRemoveWorkspace={vi.fn()}
        baseUrl="http://127.0.0.1:4733"
        token=""
        onBaseUrlChange={vi.fn()}
        onTokenChange={vi.fn()}
        onCheckGateway={vi.fn()}
        mcpServers={{}}
        rateLimits={null}
        currentProviderId={null}
        busy={false}
        theme="dark"
        onToggleTheme={vi.fn()}
      />,
    );

    expect(screen.queryByRole("dialog", { name: "Settings" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Open settings" }));
    expect(screen.getByRole("dialog", { name: "Settings" })).toBeTruthy();

    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("dialog", { name: "Settings" })).toBeNull();
  });

  it("toggles between dark and light themes", () => {
    const onToggleTheme = vi.fn();
    const { rerender } = render(
      <Sidebar
        gatewayState="online"
        gatewayVersion="1.0.0"
        workspaces={[]}
        activeWorkspaceId={null}
        onSelectWorkspace={vi.fn()}
        onCreateWorkspace={vi.fn()}
        onLoadWorkspaces={vi.fn()}
        onConnectWorkspace={vi.fn()}
        threadsByWorkspace={{}}
        activeThreadId={null}
        onSelectThread={vi.fn()}
        onNewThread={vi.fn()}
        onArchiveThread={vi.fn()}
        onRemoveWorkspace={vi.fn()}
        baseUrl="http://127.0.0.1:4733"
        token=""
        onBaseUrlChange={vi.fn()}
        onTokenChange={vi.fn()}
        onCheckGateway={vi.fn()}
        mcpServers={{}}
        rateLimits={null}
        currentProviderId={null}
        busy={false}
        theme="dark"
        onToggleTheme={onToggleTheme}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Switch to light theme" }));
    expect(onToggleTheme).toHaveBeenCalledTimes(1);

    rerender(
      <Sidebar
        gatewayState="online"
        gatewayVersion="1.0.0"
        workspaces={[]}
        activeWorkspaceId={null}
        onSelectWorkspace={vi.fn()}
        onCreateWorkspace={vi.fn()}
        onLoadWorkspaces={vi.fn()}
        onConnectWorkspace={vi.fn()}
        threadsByWorkspace={{}}
        activeThreadId={null}
        onSelectThread={vi.fn()}
        onNewThread={vi.fn()}
        onArchiveThread={vi.fn()}
        onRemoveWorkspace={vi.fn()}
        baseUrl="http://127.0.0.1:4733"
        token=""
        onBaseUrlChange={vi.fn()}
        onTokenChange={vi.fn()}
        onCheckGateway={vi.fn()}
        mcpServers={{}}
        rateLimits={null}
        currentProviderId={null}
        busy={false}
        theme="light"
        onToggleTheme={onToggleTheme}
      />,
    );
    expect(screen.getByRole("button", { name: "Switch to dark theme" })).toBeTruthy();
  });

  it("renders Codex quota windows above the settings control", () => {
    const view = render(
      <Sidebar
        gatewayState="online"
        gatewayVersion="1.0.0"
        workspaces={[]}
        activeWorkspaceId={null}
        onSelectWorkspace={vi.fn()}
        onCreateWorkspace={vi.fn()}
        onLoadWorkspaces={vi.fn()}
        onConnectWorkspace={vi.fn()}
        threadsByWorkspace={{}}
        activeThreadId={null}
        onSelectThread={vi.fn()}
        onNewThread={vi.fn()}
        onArchiveThread={vi.fn()}
        onRemoveWorkspace={vi.fn()}
        baseUrl="http://127.0.0.1:4733"
        token=""
        onBaseUrlChange={vi.fn()}
        onTokenChange={vi.fn()}
        onCheckGateway={vi.fn()}
        mcpServers={{}}
        rateLimits={{
          primary: { usedPercent: 12, resetsAt: Date.now() + 3_600_000 },
          secondary: { usedPercent: 67, resetsAt: Date.now() + 86_400_000 },
          credits: { hasCredits: true, unlimited: false, balance: "40" },
        }}
        currentProviderId={"openai"}
        busy={false}
        theme="dark"
        onToggleTheme={vi.fn()}
      />,
    );

    expect(screen.getByLabelText("Codex usage limits")).toBeTruthy();
    expect(screen.getByText("Session")).toBeTruthy();
    expect(screen.getByText("Weekly")).toBeTruthy();
    expect(screen.getByText("Credits: 40 credits")).toBeTruthy();
    const bottom = view.container.querySelector(".web-sidebar-bottom");
    expect(bottom?.firstElementChild?.classList.contains("web-quota-card")).toBe(true);
  });
  it("hides quota windows when provider is not OpenAI", () => {
    render(
      <Sidebar
        gatewayState="online"
        gatewayVersion="1.0.0"
        workspaces={[]}
        activeWorkspaceId={null}
        onSelectWorkspace={vi.fn()}
        onCreateWorkspace={vi.fn()}
        onLoadWorkspaces={vi.fn()}
        onConnectWorkspace={vi.fn()}
        threadsByWorkspace={{}}
        activeThreadId={null}
        onSelectThread={vi.fn()}
        onNewThread={vi.fn()}
        onArchiveThread={vi.fn()}
        onRemoveWorkspace={vi.fn()}
        baseUrl="http://127.0.0.1:4733"
        token=""
        onBaseUrlChange={vi.fn()}
        onTokenChange={vi.fn()}
        onCheckGateway={vi.fn()}
        mcpServers={{}}
        rateLimits={{
          primary: { usedPercent: 12, resetsAt: Date.now() + 3_600_000 },
          secondary: { usedPercent: 67, resetsAt: Date.now() + 86_400_000 },
        }}
        currentProviderId={"deepseek"}
        busy={false}
        theme="dark"
        onToggleTheme={vi.fn()}
      />,
    );

    expect(screen.queryByLabelText("Codex usage limits")).toBeNull();
    expect(screen.queryByText("Session")).toBeNull();
  });

});
