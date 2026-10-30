// @vitest-environment jsdom

import {
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import Workspaces, { type CopilotOption } from "./Workspaces";

const workspace = {
  id: "ws-1",
  name: "Demo",
  path: "/tmp/demo",
  connected: true,
  settings: { sidebarCollapsed: false },
};

function baseProps() {
  return {
    workspaces: [workspace],
    activeId: "ws-1",
    onSelect: vi.fn(),
    onCreate: vi.fn(),
    onConnect: vi.fn(),
    onLoad: vi.fn(),
    busy: false,
    threadsByWorkspace: { "ws-1": [] },
    activeThreadId: null,
    onSelectThread: vi.fn(),
    copilots: [] as CopilotOption[],
    onStartTask: vi.fn(),
    onArchiveThread: vi.fn(),
    onRemoveWorkspace: vi.fn(),
  };
}

describe("Web workspace actions", () => {
  afterEach(cleanup);
  it("places the remove action before the single New task action", () => {
    const props = baseProps();
    render(<Workspaces {...props} />);

    const remove = screen.getByRole("button", { name: "Remove workspace Demo" });
    const addThread = screen.getByRole("button", { name: "New task in Demo" });
    expect(
      remove.compareDocumentPosition(addThread) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();

    fireEvent.click(remove);
    expect(props.onRemoveWorkspace).toHaveBeenCalledWith("ws-1");
  });

  it("starts a plain task directly when no Copilot is configured", () => {
    const props = baseProps();
    render(<Workspaces {...props} />);

    fireEvent.click(screen.getByRole("button", { name: "New task in Demo" }));
    expect(props.onStartTask).toHaveBeenCalledWith("ws-1", null);
    expect(screen.queryByRole("dialog", { name: "Choose a Copilot" })).toBeNull();
  });

  it("opens Copilot management instead of creating a plain task when selection is required", () => {
    const props = baseProps();
    const onCopilotUnavailable = vi.fn();
    render(
      <Workspaces
        {...props}
        copilotSelectionRequired
        onCopilotUnavailable={onCopilotUnavailable}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "New task in Demo" }));

    expect(onCopilotUnavailable).toHaveBeenCalledTimes(1);
    expect(props.onStartTask).not.toHaveBeenCalled();
  });

  it("lists independent Copilot packages and starts the selected package", () => {
    const props = baseProps();
    props.copilots = [
      { packageId: "warehouse-network", displayName: "Warehouse Network · Multi-Agent" },
      {
        packageId: "warehouse-network-single-agent",
        displayName: "Warehouse Network · Single Agent",
      },
    ];
    render(<Workspaces {...props} />);

    fireEvent.click(screen.getByRole("button", { name: "New task in Demo" }));
    expect(screen.getByRole("dialog", { name: "Choose a Copilot" })).toBeTruthy();
    fireEvent.click(screen.getByRole("radio", { name: /Warehouse Network · Single Agent/ }));
    fireEvent.click(screen.getByRole("button", { name: "Create Thread" }));

    expect(props.onStartTask).toHaveBeenCalledWith("ws-1", {
      packageId: "warehouse-network-single-agent",
    });
  });

  it("starts the only active Copilot without asking", () => {
    const props = baseProps();
    props.copilots = [
      {
        packageId: "warehouse-network-single-agent",
        displayName: "Warehouse Network · Single Agent",
      },
    ];
    render(<Workspaces {...props} />);

    fireEvent.click(screen.getByRole("button", { name: "New task" }));

    expect(screen.queryByRole("dialog", { name: "Choose a Copilot" })).toBeNull();
    expect(props.onStartTask).toHaveBeenCalledWith("ws-1", {
      packageId: "warehouse-network-single-agent",
    });
  });

  it("disables the primary New task action without an active workspace", () => {
    const props = baseProps();
    render(<Workspaces {...props} activeId={null} />);

    const newTask = screen.getByRole("button", { name: "New task" }) as HTMLButtonElement;
    expect(newTask.disabled).toBe(true);
  });

  it("opens the Copilot choice for an external start request", () => {
    const props = baseProps();
    props.copilots = [
      { packageId: "warehouse-network", displayName: "Warehouse Network · Multi-Agent" },
      {
        packageId: "warehouse-network-single-agent",
        displayName: "Warehouse Network · Single Agent",
      },
    ];
    const onStartTaskCancelled = vi.fn();
    const { rerender } = render(
      <Workspaces {...props} onStartTaskCancelled={onStartTaskCancelled} />,
    );

    rerender(
      <Workspaces
        {...props}
        startTaskRequest={{ id: 1, workspaceId: "ws-1" }}
        onStartTaskCancelled={onStartTaskCancelled}
      />,
    );
    expect(screen.getByRole("dialog", { name: "Choose a Copilot" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Create Thread" }));

    expect(props.onStartTask).toHaveBeenCalledWith("ws-1", { packageId: "warehouse-network" });
    expect(onStartTaskCancelled).not.toHaveBeenCalled();
  });

  it("reports a cancelled Copilot choice", () => {
    const props = baseProps();
    props.copilots = [
      { packageId: "warehouse-network", displayName: "Warehouse Network · Multi-Agent" },
      {
        packageId: "warehouse-network-single-agent",
        displayName: "Warehouse Network · Single Agent",
      },
    ];
    const onStartTaskCancelled = vi.fn();
    render(
      <Workspaces
        {...props}
        startTaskRequest={{ id: 1, workspaceId: "ws-1" }}
        onStartTaskCancelled={onStartTaskCancelled}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(screen.queryByRole("dialog", { name: "Choose a Copilot" })).toBeNull();
    expect(onStartTaskCancelled).toHaveBeenCalledTimes(1);
    expect(props.onStartTask).not.toHaveBeenCalled();
  });

  it("does not start a plain task from an external request when selection is required", () => {
    const props = baseProps();
    const onCopilotUnavailable = vi.fn();
    const onStartTaskCancelled = vi.fn();
    render(
      <Workspaces
        {...props}
        copilotSelectionRequired
        onCopilotUnavailable={onCopilotUnavailable}
        startTaskRequest={{ id: 1, workspaceId: "ws-1" }}
        onStartTaskCancelled={onStartTaskCancelled}
      />,
    );

    expect(onCopilotUnavailable).toHaveBeenCalledTimes(1);
    expect(onStartTaskCancelled).toHaveBeenCalledTimes(1);
    expect(props.onStartTask).not.toHaveBeenCalled();
  });

  it("shows running state and confirms before archiving a thread", () => {
    const props = baseProps();
    const onArchiveThread = vi.fn();
    render(
      <Workspaces
        {...props}
        threadsByWorkspace={{
          "ws-1": [
            {
              id: "running-thread",
              label: "Running",
              updatedAt: 2,
              status: "running",
            },
            {
              id: "idle-thread",
              label: "Idle",
              updatedAt: 1,
              status: "idle",
            },
          ],
        }}
        onArchiveThread={onArchiveThread}
      />,
    );

    const runningIndicator = screen.getByRole("status", {
      name: "Thread is running",
    });
    const runningLabel = screen.getByText("Running");
    expect(
      runningLabel.compareDocumentPosition(runningIndicator) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    expect(document.querySelector(".web-ws-thread-status.is-running")).toBeNull();
    expect(
      screen.getByText("Idle").parentElement?.querySelector(
        ".web-ws-thread-running",
      ),
    ).toBeNull();
    expect(
      (screen.getByRole("button", {
        name: "Archive thread Running",
      }) as HTMLButtonElement).disabled,
    ).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "Archive thread Idle" }));
    expect(
      screen.getByRole("alertdialog", { name: "Archive thread?" }),
    ).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Archive" }));
    expect(onArchiveThread).toHaveBeenCalledWith("ws-1", "idle-thread");
  });
});
