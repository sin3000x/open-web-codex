import { describe, expect, it } from "vitest";
import { agentMessagePhase, appendTerminalInteractionOutput, buildWebThreadHistory, isUserThreadItem, mergeWebThreadHistory, unwrapWebRpcResult } from "./webThreadHistory";

describe("isUserThreadItem", () => {
  it("recognizes both live and persisted user message shapes", () => {
    expect(isUserThreadItem({ type: "userMessage" })).toBe(true);
    expect(isUserThreadItem({ type: "message", role: "user" })).toBe(true);
    expect(isUserThreadItem({ type: "agentMessage" })).toBe(false);
  });
});

describe("agentMessagePhase", () => {
  it("accepts only phases from the generated Codex contract", () => {
    expect(agentMessagePhase("commentary")).toBe("commentary");
    expect(agentMessagePhase("final_answer")).toBe("final_answer");
    expect(agentMessagePhase("analysis")).toBeUndefined();
    expect(agentMessagePhase(null)).toBeUndefined();
  });
});

describe("mergeWebThreadHistory", () => {
  it("preserves messages sent while historical turns are loading", () => {
    expect(mergeWebThreadHistory(
      [{
        id: "old",
        level: "assistant",
        text: "Earlier response",
        messagePhase: "final_answer",
      }],
      [{ id: "optimistic", level: "user", text: "New request" }],
    )).toEqual([
      {
        id: "old",
        level: "assistant",
        text: "Earlier response",
        messagePhase: "final_answer",
      },
      { id: "optimistic", level: "user", text: "New request" },
    ]);
  });

  it("does not duplicate an optimistic user message already persisted", () => {
    expect(mergeWebThreadHistory(
      [{ id: "persisted", level: "user", text: "New request" }],
      [{ id: "optimistic", level: "user", text: "New request" }],
    )).toHaveLength(1);
  });

  it("merges a live assistant projection with persisted history by runtime item id", () => {
    expect(mergeWebThreadHistory(
      [{
        id: "item-8",
        level: "assistant",
        text: "Inspecting Shanghai boundaries",
        messagePhase: "commentary",
      }],
      [{ id: "item-8", level: "assistant", text: "Inspecting Shanghai boundaries", streaming: true }],
    )).toEqual([
      {
        id: "item-8",
        level: "assistant",
        text: "Inspecting Shanghai boundaries",
        messagePhase: "commentary",
        streaming: true,
      },
    ]);
  });

  it("does not let a stale started Tool event overwrite authoritative completed history", () => {
    expect(mergeWebThreadHistory(
      [{
        id: "call-map",
        level: "info",
        text: "create_map_card",
        kind: "tool",
        toolStatus: "completed",
      }],
      [{
        id: "call-map",
        level: "info",
        text: "create_map_card",
        kind: "tool",
        toolStatus: "inProgress",
        streaming: true,
      }],
    )).toEqual([{
      id: "call-map",
      level: "info",
      text: "create_map_card",
      kind: "tool",
      toolStatus: "completed",
    }]);
  });

  it("merges a live approval with its Server-projected historical approval", () => {
    expect(mergeWebThreadHistory(
      [{
        id: "approval-history",
        level: "info",
        text: "Allow map tool?",
        kind: "approval",
        approvalRequestId: "approval-1",
        approvalStatus: "resolved",
      }],
      [{
        id: "approval-live",
        level: "info",
        text: "Allow map tool?",
        kind: "approval",
        approvalRequestId: "approval-1",
        approvalStatus: "pending",
      }],
    )).toEqual([{
      id: "approval-history",
      level: "info",
      text: "Allow map tool?",
      kind: "approval",
      approvalRequestId: "approval-1",
      approvalStatus: "resolved",
    }]);
  });
});

describe("appendTerminalInteractionOutput", () => {
  it("ignores empty terminal polls", () => {
    expect(appendTerminalInteractionOutput("waiting\n", "")).toBe("waiting\n");
  });

  it("records terminal interaction without persisting sensitive stdin", () => {
    expect(appendTerminalInteractionOutput("Password:", "secret\r\n")).toBe(
      "Password:\n[stdin supplied]\n",
    );
  });
});

describe("buildWebThreadHistory", () => {
  it("drops whitespace-only restored agent messages", () => {
    expect(buildWebThreadHistory({
      turns: [{
        items: [{
          id: "blank-commentary",
          type: "agentMessage",
          text: "\n\n  ",
          phase: "commentary",
        }],
      }],
    }, () => "unused")).toEqual([]);
  });

  it("restores Server-projected approvals in Turn item order", () => {
    const result = buildWebThreadHistory({
      turns: [{
        items: [
          {
            id: "tool-1",
            type: "mcpToolCall",
            server: "map_utils",
            tool: "batch_geocode",
            status: "completed",
          },
          {
            id: "approval-1",
            type: "platformApproval",
            text: "Allow batch_geocode?",
            approvalRequestId: "request-1",
            approvalStatus: "resolved",
            approvalServerName: "map_utils",
            approvalTool: "batch_geocode",
          },
          {
            id: "reply-1",
            type: "agentMessage",
            text: "Coordinates loaded.",
            phase: "commentary",
          },
        ],
      }],
    }, () => "unused");

    expect(result.map((entry) => entry.id)).toEqual([
      "tool-1",
      "approval-1",
      "reply-1",
    ]);
    expect(result[1]).toMatchObject({
      kind: "approval",
      approvalRequestId: "request-1",
      approvalStatus: "resolved",
      approvalTool: "batch_geocode",
    });
  });

  it.each([
    ["accepted", "accepted"],
    ["declined", "declined"],
    ["answered", "answered"],
  ] as const)("restores the %s approval outcome without collapsing it to resolved", (status, expected) => {
    const [entry] = buildWebThreadHistory({
      turns: [{
        items: [{
          id: `approval-${status}`,
          type: "platformApproval",
          text: "Approval response",
          approvalStatus: status,
        }],
      }],
    }, () => "unused");

    expect(entry.approvalStatus).toBe(expected);
  });

  it("unwraps nested gateway and app-server result envelopes", () => {
    const thread = {
      id: "019f5b75-2266-7910-bc60-b4470041c4e7",
      status: { type: "systemError" },
      turns: [{
        items: [{
          id: "item-1",
          type: "userMessage",
          content: [{ type: "text", text: "1" }],
        }],
      }],
    };

    expect(unwrapWebRpcResult({
      result: {
        id: 84,
        result: { thread },
      },
    })).toEqual({ thread });
  });

  it("keeps reasoning and command execution items from a full turn payload", () => {
    let id = 0;
    const result = buildWebThreadHistory({
      turns: [{
        items: [
          { id: "item-1", type: "userMessage", content: [{ type: "text", text: "初始化 git" }] },
          { id: "item-2", type: "reasoning", content: [], summary: ["Checking the repository."] },
          {
            id: "call-1",
            type: "commandExecution",
            command: "/bin/zsh -lc 'git status'",
            aggregatedOutput: "fatal: not a git repository\n",
            cwd: "/tmp/test",
            exitCode: 128,
            durationMs: 0,
            status: "failed",
          },
        ],
      }],
    }, () => `log-${++id}`);

    expect(result).toMatchObject([
      { level: "user", text: "初始化 git" },
      { kind: "reasoning", text: "Checking the repository." },
      {
        kind: "command_exec",
        text: "/bin/zsh -lc 'git status'",
        cmdOutput: "fatal: not a git repository\n",
        cmdExitCode: 128,
        toolStatus: "failed",
      },
    ]);
  });

  it("restores reasoning content when the persisted summary is generic", () => {
    const [entry] = buildWebThreadHistory({ turns: [{ items: [{
      id: "reasoning-content",
      type: "reasoning",
      summary: ["Reasoning completed"],
      content: ["Comparing the current layout with the target screenshot."],
    }] }] }, () => "log-1");

    expect(entry).toMatchObject({
      kind: "reasoning",
      text: "Comparing the current layout with the target screenshot.",
    });
    expect(entry.reasoningSummary).toBeUndefined();
  });

  it("strips provider sentinels from restored assistant history", () => {
    const result = buildWebThreadHistory({
      turns: [{
        items: [
          {
            id: "assistant",
            type: "agentMessage",
            text: "<｜begin▁of▁sentence｜># Route unavailable",
            phase: "final_answer",
          },
        ],
      }],
    }, () => "fallback-id");

    expect(result).toMatchObject([{
      level: "assistant",
      text: "# Route unavailable",
      messagePhase: "final_answer",
    }]);
  });

  it("restores persisted dynamic tools as commands, diffs, and expandable tool cards", () => {
    let id = 0;
    const result = buildWebThreadHistory({
      turns: [{
        items: [
          { id: "user-1", type: "userMessage", content: [{ type: "text", text: "检查项目" }] },
          {
            id: "call-command",
            type: "dynamicToolCall",
            tool: "exec_command",
            arguments: { cmd: "git status" },
            status: "completed",
            contentItems: [{ type: "inputText", text: "Process exited with code 0\nOutput:\nclean" }],
          },
          {
            id: "call-patch",
            type: "dynamicToolCall",
            tool: "apply_patch",
            arguments: "*** Begin Patch\n*** Update File: src/a.ts\n-old\n+new\n*** End Patch",
            status: "completed",
            contentItems: [],
          },
          {
            id: "call-goal",
            type: "dynamicToolCall",
            tool: "update_goal",
            arguments: { status: "complete" },
            status: "completed",
            contentItems: [{ type: "inputText", text: "Goal updated" }],
          },
          {
            id: "call-code-mode",
            type: "dynamicToolCall",
            tool: "exec",
            arguments: "const result = await tools.exec_command({ cmd: 'git status' });",
            status: "completed",
            contentItems: [{ type: "inputText", text: "Nested tool result" }],
          },
          { id: "final", type: "agentMessage", text: "完成。", phase: "final_answer" },
        ],
      }],
    }, () => `log-${++id}`);

    expect(result).toMatchObject([
      { level: "user", text: "检查项目" },
      { kind: "command_exec", text: "git status", cmdExitCode: 0 },
      { kind: "diff", diffTitle: "File changes" },
      { kind: "tool", toolTitle: "update_goal", toolOutput: "Goal updated" },
      { kind: "tool", toolTitle: "exec", toolOutput: "Nested tool result" },
      { level: "assistant", text: "完成。", messagePhase: "final_answer" },
    ]);
  });

  it("keeps every known thread item type visible in restored history", () => {
    let id = 0;
    const result = buildWebThreadHistory({
      turns: [{ items: [
        { id: "mcp", type: "mcpToolCall", server: "docs", tool: "search", status: "completed", arguments: { q: "api" }, result: { content: [] } },
        { id: "search", type: "webSearch", query: "Codex docs" },
        { id: "plan", type: "plan", text: "1. Inspect" },
        { id: "image", type: "imageView", path: "diagram.png" },
        { id: "sleep", type: "sleep", durationMs: 1000 },
        { id: "compact", type: "contextCompaction" },
        { id: "collab", type: "collabAgentToolCall", tool: "spawnAgent", status: "completed", prompt: "Review" },
        { id: "sub", type: "subAgentActivity", kind: "message", agentPath: "reviewer" },
        { id: "review", type: "enteredReviewMode", review: "Reviewing" },
      ] }],
    }, () => `log-${++id}`);

    expect(result).toHaveLength(9);
    expect(result.every((entry) => entry.kind === "tool")).toBe(true);
  });

  it("restores authorized typed Artifacts on their Agent Message", () => {
    const [entry] = buildWebThreadHistory({
      turns: [{ items: [{
        id: "assistant-map",
        type: "agentMessage",
        text: [
          "Before.",
          '::codex-inline-vis{artifact="map-locations"}',
          "After.",
        ].join("\n"),
        inlineArtifacts: [{
          ref: "map-locations",
          renderer: {
            kind: "map.v3",
            payload: {
            title: "Locations",
            intent: "visualization",
            status: "ready",
            sources: {
              locations: {
                type: "geojson",
                data: {
                type: "artifact",
                format: "geojson",
                artifact_id: "8e98ff2f-82ee-4cc9-a3e6-2974debf8666",
                url: "/api/runs/975f1f1c-4b58-47ad-a12c-c32aeae566e7/artifacts/8e98ff2f-82ee-4cc9-a3e6-2974debf8666",
              },
              },
            },
            layers: [{
              id: "points",
              source: "locations",
              type: "circle",
              paint: { "circle-color": "#ef4444" },
            }],
            },
          },
        }],
      }] }],
    }, () => "generated");

    expect(entry).toMatchObject({
      id: "assistant-map",
      level: "assistant",
      text: 'Before.\n::codex-inline-vis{artifact="map-locations"}\nAfter.',
      inlineArtifacts: [{
        ref: "map-locations",
        rendererKind: "map.v3",
        card: {
          kind: "map.v3",
          title: "Locations",
          sources: [{
            id: "locations",
            data: {
              type: "artifact",
              artifactId: "8e98ff2f-82ee-4cc9-a3e6-2974debf8666",
            },
          }],
        },
      }],
    });
  });

  it("redacts sensitive dynamic tool arguments", () => {
    let id = 0;
    const [entry] = buildWebThreadHistory({ turns: [{ items: [{
      id: "stdin",
      type: "dynamicToolCall",
      tool: "write_stdin",
      arguments: { session_id: 1, chars: "password" },
      status: "completed",
      contentItems: [],
    }] }] }, () => `log-${++id}`);

    expect(entry.toolDetail).toContain("[redacted]");
    expect(entry.toolDetail).not.toContain("password");
  });

  it("shows a visible fallback notice for a systemError without details", () => {
    let id = 0;
    const result = buildWebThreadHistory({
      status: { type: "systemError" },
      turns: [{
        items: [{
          id: "item-1",
          type: "userMessage",
          content: [{ type: "text", text: "1" }],
        }],
      }],
    }, () => `log-${++id}`);

    expect(result).toMatchObject([
      { level: "user", text: "1" },
      {
        level: "error",
        text: "System error. The runtime did not provide any additional error details.",
      },
    ]);
  });
});
