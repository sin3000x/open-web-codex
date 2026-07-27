// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import MessageList from "./MessageList";

describe("MessageList", () => {
  afterEach(cleanup);

  it("renders reasoning and a restored command execution without message text", () => {
    render(
      <MessageList
        items={[
          {
            id: "reasoning-1",
            level: "system",
            kind: "reasoning",
            text: "Checking whether the workspace is already a repository.",
          },
          {
            id: "command-1",
            level: "info",
            kind: "command_exec",
            text: "/bin/zsh -lc 'git status'",
            toolStatus: "failed",
            cmdExitCode: 128,
            cmdOutput: "fatal: not a git repository",
          },
        ]}
      />,
    );

    expect(screen.getByText(/Checking whether/)).toBeTruthy();
    expect(screen.getByText("git status")).toBeTruthy();
    expect(screen.getByText("✗ exit 128")).toBeTruthy();
    fireEvent.click(screen.getByText("git status"));
    expect(screen.getByText("fatal: not a git repository")).toBeTruthy();
  });

  it("renders a pending command as running", () => {
    render(
      <MessageList
        items={[
          {
            id: "command-running",
            level: "info",
            kind: "command_exec",
            text: "git init",
            toolStatus: "inProgress",
          },
        ]}
      />,
    );

    expect(screen.getByText("running")).toBeTruthy();
  });

  it("shows a resolved approval on the command card", () => {
    render(
      <MessageList
        items={[{
          id: "command-approved",
          level: "info",
          kind: "command_exec",
          text: "git push",
          toolStatus: "completed",
          cmdExitCode: 0,
          approvalStatus: "accepted",
        }]}
      />,
    );

    expect(screen.getByLabelText("Approved: The user approved this action.")).toBeTruthy();
    expect(screen.queryByText("Approval resolved")).toBeNull();
  });

  it("folds a resolved approval into the MCP call that it authorized", () => {
    const view = render(
      <MessageList
        items={[
          { id: "user-1", level: "user", text: "Geocode these cities" },
          {
            id: "mcp-unrelated",
            level: "info",
            kind: "tool",
            text: "read resource",
            toolType: "MCP",
            toolTitle: "map_utils / read_mcp_resource",
            toolStatus: "completed",
          },
          {
            id: "approval-1",
            level: "info",
            kind: "approval",
            text: "Allow the map_utils MCP server to run tool \"batch_geocode\"?",
            approvalStatus: "accepted",
            approvalServerName: "map_utils",
            approvalTool: "batch_geocode",
          },
          {
            id: "mcp-1",
            level: "info",
            kind: "tool",
            text: "batch geocode",
            toolType: "MCP",
            toolTitle: "map_utils / batch_geocode",
            toolStatus: "completed",
          },
        ]}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "2 tool calls, 0 messages" }));
    expect(view.container.querySelector(".web-approval-card")).toBeNull();
    const approvalIcon = screen.getByLabelText(
      'Approved: Allow the map_utils MCP server to run tool "batch_geocode"?',
    );
    expect(approvalIcon.closest(".web-tool-card")?.textContent).toContain(
      "map_utils / batch_geocode",
    );
    expect(
      screen.getByText("map_utils / read_mcp_resource")
        .closest(".web-tool-card")
        ?.querySelector(".web-approval-status-icon"),
    ).toBeNull();
  });

  it.each([
    ["accepted", "Approved"],
    ["declined", "Denied"],
    ["answered", "Other response"],
  ] as const)("shows the %s outcome on a running MCP call as soon as the user responds", (
    approvalStatus,
    label,
  ) => {
    const items = [
      { id: "user-live", level: "user" as const, text: "Geocode Shanghai" },
      {
        id: "mcp-live",
        level: "info" as const,
        kind: "tool" as const,
        text: "batch geocode",
        toolType: "MCP",
        toolTitle: "map_utils / batch_geocode",
        toolStatus: "running",
        streaming: true,
      },
      {
        id: "approval-live",
        level: "info" as const,
        kind: "approval" as const,
        text: "Allow the map_utils MCP server to run tool \"batch_geocode\"?",
        approvalStatus: "pending" as const,
        approvalServerName: "map_utils",
        approvalTool: "batch_geocode",
      },
    ];
    const view = render(<MessageList items={items} thinking />);

    expect(screen.queryByLabelText(new RegExp(`^${label}:`))).toBeNull();
    view.rerender(
      <MessageList
        thinking
        items={items.map((item) => item.id === "approval-live"
          ? { ...item, approvalStatus }
          : item)}
      />,
    );

    const icon = screen.getByLabelText(
      `${label}: Allow the map_utils MCP server to run tool "batch_geocode"?`,
    );
    expect(icon.closest(".web-tool-card")?.textContent).toContain(
      "map_utils / batch_geocode",
    );
    expect(view.container.querySelector(".web-approval-card")).toBeNull();
  });

  it("renders a recoverable connection error as an active status", () => {
    const view = render(
      <MessageList
        items={[{
          id: "connection-1",
          level: "info",
          kind: "connection",
          text: "Connection interrupted. Reconnecting (1/5)…",
          streaming: true,
        }]}
      />,
    );

    expect(screen.getByRole("status").textContent).toContain("Reconnecting (1/5)");
    expect(view.container.querySelector(".web-thinking-spinner")).toBeTruthy();
  });

  it("groups turn activity and keeps the final answer outside the execution timeline", () => {
    const view = render(
      <MessageList
        items={[
          { id: "user-1", level: "user", text: "Inspect the project" },
          { id: "reasoning-1", level: "system", kind: "reasoning", text: "Reviewing project files" },
          {
            id: "commentary-1",
            level: "assistant",
            text: "I am checking the relevant files.",
            messagePhase: "commentary",
          },
          { id: "command-1", level: "info", kind: "command_exec", text: "rg --files", cmdExitCode: 0 },
          {
            id: "final-1",
            level: "assistant",
            text: "The project is ready.",
            messagePhase: "final_answer",
          },
        ]}
      />,
    );

    const summary = screen.getByRole("button", { name: "1 tool call, 2 messages" });
    expect(summary.getAttribute("aria-expanded")).toBe("false");
    expect(view.container.querySelector(".web-execution-timeline")).toBeNull();
    fireEvent.click(summary);
    const timeline = view.container.querySelector(".web-execution-timeline");
    expect(timeline?.textContent).toContain("I am checking the relevant files.");
    expect(timeline?.textContent).not.toContain("The project is ready.");
  });

  it("uses message phase instead of message position to identify replies", () => {
    const view = render(
      <MessageList
        items={[
          { id: "user-1", level: "user", text: "Inspect the project" },
          {
            id: "first-reply",
            level: "assistant",
            text: "First provider reply.",
            messagePhase: "final_answer",
          },
          {
            id: "commentary-1",
            level: "assistant",
            text: "I am checking one more file.",
            messagePhase: "commentary",
          },
          {
            id: "command-1",
            level: "info",
            kind: "command_exec",
            text: "git status",
            cmdExitCode: 0,
          },
          {
            id: "typed-reply",
            level: "assistant",
            text: "Typed final reply.",
            messagePhase: "final_answer",
          },
        ]}
      />,
    );

    expect(screen.getByText("First provider reply.")).toBeTruthy();
    expect(screen.getByText("Typed final reply.")).toBeTruthy();
    expect(screen.queryByText("I am checking one more file.")).toBeNull();
    const summary = screen.getByRole("button", { name: "1 tool call, 1 message" });
    fireEvent.click(summary);
    expect(screen.getByText("I am checking one more file.")).toBeTruthy();

    const text = view.container.textContent ?? "";
    expect(text.indexOf("First provider reply.")).toBeLessThan(text.indexOf("Typed final reply."));
  });

  it("renders tool calls as timeline siblings and drops empty reasoning wrappers", () => {
    const view = render(
      <MessageList
        items={[
          { id: "user-1", level: "user", text: "Check the weather" },
          { id: "reasoning-empty", level: "system", kind: "reasoning", text: "Reasoning completed" },
          { id: "search-1", level: "info", kind: "tool", text: "Web search", toolType: "Search", toolTitle: "weather Shenzhen", toolStatus: "completed" },
          {
            id: "final-1",
            level: "assistant",
            text: "It is sunny.",
            messagePhase: "final_answer",
          },
        ]}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "1 tool call, 0 messages" }));
    const toolCard = screen.getByText("weather Shenzhen").closest(".web-tool-card");
    expect(toolCard).toBeTruthy();
    expect(toolCard?.closest(".web-reasoning")).toBeNull();
    expect(view.container.querySelector(".web-reasoning")).toBeNull();
  });

  it("keeps a streamed final answer outside the process timeline", () => {
    const liveItems = [
      { id: "user-1", level: "user" as const, text: "Inspect the project" },
      { id: "reasoning-1", level: "system" as const, kind: "reasoning" as const, text: "Reviewing project files" },
      {
        id: "commentary-1",
        level: "assistant" as const,
        text: "I am checking the relevant files.",
        messagePhase: "commentary" as const,
      },
      { id: "command-1", level: "info" as const, kind: "command_exec" as const, text: "rg --files", toolStatus: "running" },
      {
        id: "final-1",
        level: "assistant" as const,
        text: "The project is rea",
        messagePhase: "final_answer" as const,
        streaming: true,
      },
    ];
    const view = render(<MessageList items={liveItems} thinking />);

    const liveSummary = within(view.container).getByRole("button", { name: "1 tool call, 2 messages" });
    expect(liveSummary.getAttribute("aria-expanded")).toBe("false");
    expect(view.container.querySelector(".web-execution-timeline")).toBeNull();
    expect(view.container.querySelector(".web-execution-current")).toBeNull();
    expect(within(view.container).getByText("The project is rea")).toBeTruthy();
    expect(within(view.container).queryByText("Working…")).toBeNull();

    view.rerender(
      <MessageList
        items={liveItems.map((item) => item.id === "command-1"
          ? { ...item, toolStatus: "completed" }
          : item.id === "final-1"
            ? { ...item, text: "The project is ready.", streaming: false }
            : item)}
        thinking={false}
      />,
    );

    const summary = within(view.container).getByRole("button", { name: "1 tool call, 2 messages" });
    expect(summary.getAttribute("aria-expanded")).toBe("false");
    expect(view.container.querySelector(".web-execution-timeline")).toBeNull();
    expect(within(view.container).getByText("The project is ready.")).toBeTruthy();
    expect(within(view.container).queryByText("Working…")).toBeNull();
  });

  it("renders active commentary with process styling", () => {
    const view = render(
      <MessageList
        items={[
          { id: "user-1", level: "user", text: "Inspect the project" },
          {
            id: "commentary-1",
            level: "assistant",
            text: "I am checking the relevant files.",
            messagePhase: "commentary",
            streaming: true,
          },
        ]}
        thinking
      />,
    );

    expect(view.container.querySelector(".web-execution-current .web-msg-commentary-body")).toBeTruthy();
    expect(view.container.querySelector(".web-execution-current .web-msg-commentary")).toBeTruthy();
  });

  it("does not guess a process phase for an unclassified streaming assistant message", () => {
    const items = [
      { id: "user-1", level: "user" as const, text: "Write the script" },
      {
        id: "assistant-1",
        level: "assistant" as const,
        text: "Preparing the Python script",
        streaming: true,
      },
    ];
    const view = render(<MessageList items={items} thinking />);

    expect(view.container.querySelector(".web-execution-current")).toBeNull();
    expect(view.container.querySelector(".web-msg-commentary-body")).toBeNull();
    expect(screen.getByText("Preparing the Python script")).toBeTruthy();

    view.rerender(
      <MessageList
        items={items.map((item) => item.id === "assistant-1"
          ? {
              ...item,
              text: "Script complete",
              streaming: false,
              messagePhase: "final_answer" as const,
            }
          : item)}
      />,
    );

    expect(view.container.querySelector(".web-execution-current")).toBeNull();
    expect(view.container.querySelector(".web-msg-commentary-body")).toBeNull();
    expect(screen.getByText("Script complete")).toBeTruthy();
  });

  it("passes the live reasoning state through to its collapsible block", () => {
    const view = render(
      <MessageList
        items={[{
          id: "reasoning-running",
          level: "system",
          kind: "reasoning",
          text: "Inspecting files",
          streaming: true,
        }]}
      />,
    );

    expect(screen.getByText("Inspecting files")).toBeTruthy();
    expect(view.container.querySelector(".web-reasoning-working")).toBeTruthy();
  });

  it("passes workspace and thread context to approval decisions", () => {
    const onResolve = vi.fn();
    render(
      <MessageList
        workspaceId="workspace-1"
        onResolveApproval={onResolve}
        items={[
          {
            id: "approval-1",
            level: "info",
            kind: "approval",
            text: "git init",
            approvalRequestId: 42,
          },
        ]}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Accept" }));
    expect(onResolve).toHaveBeenCalledWith("workspace-1", 42, "accept");
  });

  it("renders URL-mode MCP elicitation as key configuration instead of ordinary approval", () => {
    const onResolve = vi.fn();
    const view = render(
      <MessageList
        workspaceId="workspace-1"
        onResolveApproval={onResolve}
        thinking
        items={[
          {
            id: "user-url",
            level: "user",
            text: "Geocode the cities.",
          },
          {
            id: "approval-url",
            level: "info",
            kind: "approval",
            text: "Google Maps API key is not configured. Configure it in this app to save it globally and reuse it automatically.",
            approvalRequestId: "approval-url-1",
            approvalStatus: "pending",
            approvalMode: "url",
            approvalUrl: "http://127.0.0.1:43123/one-time-token",
            approvalServerName: "workspace_maps",
          },
        ]}
      />,
    );

    expect(screen.getByText("Map provider and API key required")).toBeTruthy();
    expect(screen.getByRole("button", { name: "配置 Key" })).toBeTruthy();
    expect(screen.queryByRole("link", { name: "Configure key" })).toBeNull();
    expect(screen.getByText("Waiting for API key…")).toBeTruthy();
    expect(screen.queryByText("Working…")).toBeNull();
    expect(within(view.container).queryByRole("button", { name: "Accept" })).toBeNull();
  });

  it("stops waiting for a map key after server-side delivery succeeds", () => {
    render(
      <MessageList
        thinking
        items={[
          {
            id: "user-mapbox",
            level: "user",
            text: "Geocode the cities.",
          },
          {
            id: "approval-mapbox-url",
            level: "info",
            kind: "approval",
            text: "Mapbox access token is not configured. Configure it in this app to save it globally and reuse it automatically.",
            approvalRequestId: "approval-mapbox-url-1",
            approvalStatus: "accepted",
            approvalMode: "url",
            approvalUrl: "http://127.0.0.1:43123/one-time-token",
            approvalServerName: "workspace_maps",
          },
          {
            id: "approval-next-tool",
            level: "info",
            kind: "approval",
            text: "Allow the workspace_maps MCP server to run tool \"batch_geocode\"?",
            approvalRequestId: "approval-next-tool-1",
            approvalStatus: "pending",
          },
        ]}
      />,
    );

    expect(screen.getByText("Waiting for approval…")).toBeTruthy();
    expect(screen.queryByText("Waiting for API key…")).toBeNull();
  });

  it("renders a resolved server request as non-interactive history", () => {
    const onResolve = vi.fn();
    const view = render(
      <MessageList
        workspaceId="workspace-1"
        onResolveApproval={onResolve}
        items={[{
          id: "approval-resolved",
          level: "info",
          kind: "approval",
          text: "git init",
          approvalRequestId: 43,
          approvalStatus: "resolved",
        }]}
      />,
    );

    expect(screen.getByText("Approval resolved")).toBeTruthy();
    expect(screen.getByText("Resolved")).toBeTruthy();
    expect(view.container.querySelector(".web-approval-accept")).toBeNull();
    expect(view.container.querySelector(".web-approval-deny")).toBeNull();
  });

  it("renders typed Artifacts only where one Assistant message references them", async () => {
    const view = render(
      <MessageList
        items={[
          {
            id: "user-map",
            level: "user",
            text: "Show the locations.",
          },
          {
            id: "map-card",
            level: "info",
            kind: "tool",
            text: "create_map_card",
            toolType: "MCP",
            toolTitle: "map_utils / create_map_card",
            toolStatus: "completed",
          },
          {
            id: "assistant-final",
            level: "assistant",
            text: [
              "第一张地图之前。",
              '::codex-inline-vis{artifact="map-card-data"}',
              "两张地图之间。",
              '::codex-inline-vis{artifact="map-card-route"}',
              "第二张地图之后。",
            ].join("\n"),
            messagePhase: "final_answer",
            inlineArtifacts: [
              {
                ref: "map-card-data",
                rendererKind: "map.v3",
                card: {
                  type: "card",
                  kind: "map.v3",
                  id: "map-card-data",
                  title: "Batch geocode",
                  intent: "visualization",
                  status: "ready",
                  viewport: { mode: "fit" },
                  sources: [{
                    id: "locations",
                    data: {
                      type: "inline",
                      format: "geojson",
                      geojson: {
                        type: "FeatureCollection",
                        features: [],
                      },
                    },
                  }],
                  layers: [{
                    id: "points",
                    source: "locations",
                    type: "circle",
                    paint: {},
                  }],
                },
              },
              {
                ref: "map-card-route",
                rendererKind: "map.v3",
                card: {
                  type: "card",
                  kind: "map.v3",
                  id: "map-card-route",
                  title: "Route overview",
                  intent: "route",
                  status: "ready",
                  viewport: {
                    mode: "camera",
                    center: [116.44, 39.92],
                    zoom: 9,
                  },
                  sources: [{
                    id: "route",
                    data: {
                      type: "inline",
                      format: "geojson",
                      geojson: { type: "FeatureCollection", features: [] },
                    },
                  }],
                  layers: [{
                    id: "route-line",
                    source: "route",
                    type: "line",
                    paint: { "line-width": 4, "line-dasharray": [2, 1] },
                  }],
                },
              },
            ],
          },
        ]}
      />,
    );

    expect(screen.getByText("Batch geocode")).toBeTruthy();
    expect(screen.getByText("Route overview")).toBeTruthy();
    expect(screen.getByText("两张地图之间。")).toBeTruthy();
    const executionSummary = screen.getByRole("button", { name: "1 tool call, 0 messages" });
    fireEvent.click(executionSummary);
    expect(screen.getByText("map_utils / create_map_card")).toBeTruthy();
    const text = view.container.textContent ?? "";
    expect(text.indexOf("第一张地图之前。")).toBeLessThan(text.indexOf("Batch geocode"));
    expect(text.indexOf("Batch geocode")).toBeLessThan(text.indexOf("两张地图之间。"));
    expect(text.indexOf("两张地图之间。")).toBeLessThan(text.indexOf("Route overview"));
    expect(text.indexOf("Route overview")).toBeLessThan(text.indexOf("第二张地图之后。"));
    expect(screen.getAllByRole("button", { name: "Open map card fullscreen" })).toHaveLength(2);
    const configureButtons = await screen.findAllByRole("button", { name: "配置 Mapbox Key" });
    fireEvent.click(configureButtons[0]);
    expect(screen.getByRole("dialog", { name: "配置地图服务 Key" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Mapbox" }).getAttribute("aria-pressed"))
      .toBe("true");
    expect(screen.getByRole("button", { name: "Google" })).toBeTruthy();
    const input = screen.getByLabelText("Mapbox public token");
    fireEvent.change(input, { target: { value: "sk.not-a-mapbox-public-token" } });
    fireEvent.click(screen.getByRole("button", { name: "保存配置" }));
    expect(screen.getByText(/请输入以 pk\. 开头/)).toBeTruthy();
  });

  it("does not render a visualization when only the producer Tool completed", () => {
    const view = render(
      <MessageList
        items={[
          { id: "user", level: "user", text: "Prepare a map." },
          {
            id: "tool",
            level: "info",
            kind: "tool",
            text: "create_map_card",
            toolType: "MCP",
            toolTitle: "map_utils / create_map_card",
            toolStatus: "completed",
            toolOutput: '::codex-inline-vis{artifact="map-unreferenced"}',
          },
        ]}
      />,
    );

    expect(view.container.querySelector(".web-map-card")).toBeNull();
  });
});
