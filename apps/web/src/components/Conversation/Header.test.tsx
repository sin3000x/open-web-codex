// @vitest-environment jsdom
import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import Header from "./Header";

describe("Header", () => {
  it("omits context usage and the Codex selector", () => {
    render(
      <Header
        workspaceName="workspace"
        threadTitle={null}
        threadStatus="idle"
        sidebarCollapsed={false}
        onToggleSidebar={vi.fn()}
      />,
    );

    expect(screen.queryByTitle(/Context used/)).toBeNull();
    expect(screen.queryByTitle("Active coding agent")).toBeNull();
    expect(screen.getByLabelText("File manager")).toBeTruthy();
    expect(screen.queryByTitle("Terminal integration is not available in Web mode")).toBeNull();
    expect(screen.queryByTitle("Thread link")).toBeNull();
  });

  it("shows the thread name without exposing its id", () => {
    const { container } = render(
      <Header
        workspaceName="workspace"
        threadTitle="Generated title"
        threadStatus="idle"
        sidebarCollapsed={false}
        onToggleSidebar={vi.fn()}
      />,
    );

    expect(container.querySelector(".web-chat-workspace")?.textContent).toBe("workspace");
    expect(container.querySelector(".web-chat-title")?.textContent).toBe("Generated title");
    expect(container.textContent).not.toContain("CodexMonitor");
    expect(container.querySelectorAll(".web-chat-header-sep")).toHaveLength(1);
    expect(container.querySelector(".web-chat-workspace-chevron")).toBeNull();
  });
});
