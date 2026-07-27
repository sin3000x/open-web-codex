// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import ExecutionGroup from "./ExecutionGroup";

describe("ExecutionGroup", () => {
  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("shows elapsed time from the actual turn start", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-07-14T00:01:03Z"));
    render(
      <ExecutionGroup items={[]} active startedAt={Date.parse("2026-07-14T00:00:00Z")}>
        <div>Earlier activity</div>
      </ExecutionGroup>,
    );

    expect(screen.getByText("1:03")).toBeTruthy();
    act(() => vi.advanceTimersByTime(2_000));
    expect(screen.getByText("1:05")).toBeTruthy();
  });

  it("collapses completed history in the same render without an expanded frame", () => {
    const items = [{
      id: "tool-1",
      level: "info" as const,
      kind: "tool" as const,
      text: "batch_geocode",
    }];
    const view = render(
      <ExecutionGroup items={items} active timelineItemCount={1}>
        <div>Historical tool details</div>
      </ExecutionGroup>,
    );

    expect(screen.getByText("Historical tool details")).toBeTruthy();
    view.rerender(
      <ExecutionGroup items={items} active={false} timelineItemCount={1}>
        <div>Historical tool details</div>
      </ExecutionGroup>,
    );

    expect(screen.queryByText("Historical tool details")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "1 tool call, 0 messages" }));
    expect(screen.getByText("Historical tool details")).toBeTruthy();
  });

  it("does not count approval cards as tool calls", () => {
    render(
      <ExecutionGroup
        items={[
          {
            id: "tool-1",
            level: "info",
            kind: "tool",
            text: "batch_geocode",
          },
          {
            id: "approval-1",
            level: "info",
            kind: "approval",
            text: "Allow batch_geocode?",
            approvalStatus: "resolved",
          },
        ]}
        active={false}
        timelineItemCount={2}
      >
        <div>Timeline</div>
      </ExecutionGroup>,
    );

    expect(screen.getByRole("button", { name: "1 tool call, 0 messages" })).toBeTruthy();
  });
});
