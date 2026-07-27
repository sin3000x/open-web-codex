// @vitest-environment jsdom

import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import Workspaces from "./Workspaces";

describe("Web workspace actions", () => {
  it("places the remove action before the new-thread action", () => {
    const onRemoveWorkspace = vi.fn();
    render(
      <Workspaces
        workspaces={[{
          id: "ws-1",
          name: "Demo",
          path: "/tmp/demo",
          connected: true,
          settings: { sidebarCollapsed: false },
        }]}
        activeId="ws-1"
        onSelect={vi.fn()}
        onCreate={vi.fn()}
        onConnect={vi.fn()}
        onLoad={vi.fn()}
        busy={false}
        threadsByWorkspace={{ "ws-1": [] }}
        activeThreadId={null}
        onSelectThread={vi.fn()}
        onNewThread={vi.fn()}
        onArchiveThread={vi.fn()}
        onRemoveWorkspace={onRemoveWorkspace}
      />,
    );

    const remove = screen.getByRole("button", { name: "Remove workspace Demo" });
    const addThread = screen.getByRole("button", { name: "New thread in Demo" });
    expect(remove.compareDocumentPosition(addThread) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();

    fireEvent.click(remove);
    expect(onRemoveWorkspace).toHaveBeenCalledWith("ws-1");
  });

  it("shows running state and confirms before archiving a thread", () => {
    const onArchiveThread = vi.fn();
    render(
      <Workspaces
        workspaces={[{
          id: "ws-1",
          name: "Demo",
          path: "/tmp/demo",
          connected: true,
          settings: { sidebarCollapsed: false },
        }]}
        activeId="ws-1"
        onSelect={vi.fn()}
        onCreate={vi.fn()}
        onConnect={vi.fn()}
        onLoad={vi.fn()}
        busy={false}
        threadsByWorkspace={{
          "ws-1": [
            { id: "running-thread", label: "Running", updatedAt: 2, status: "running" },
            { id: "idle-thread", label: "Idle", updatedAt: 1, status: "idle" },
          ],
        }}
        activeThreadId={null}
        onSelectThread={vi.fn()}
        onNewThread={vi.fn()}
        onArchiveThread={onArchiveThread}
        onRemoveWorkspace={vi.fn()}
      />,
    );

    const runningIndicator = screen.getByRole("status", { name: "Thread is running" });
    const runningLabel = screen.getByText("Running");
    expect(runningLabel.compareDocumentPosition(runningIndicator) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(document.querySelector(".web-ws-thread-status.is-running")).toBeNull();
    expect(screen.getByText("Idle").parentElement?.querySelector(".web-ws-thread-running")).toBeNull();
    expect(screen.queryByText("running-…")).toBeNull();
    expect(screen.queryByText("idle-thr…")).toBeNull();
    expect((screen.getByRole("button", { name: "Archive thread Running" }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "Archive thread Idle" }));
    expect(screen.getByRole("alertdialog", { name: "Archive thread?" })).toBeTruthy();
    expect(onArchiveThread).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Archive" }));
    expect(onArchiveThread).toHaveBeenCalledWith("ws-1", "idle-thread");
    expect(screen.queryByRole("alertdialog", { name: "Archive thread?" })).toBeNull();
  });
});
