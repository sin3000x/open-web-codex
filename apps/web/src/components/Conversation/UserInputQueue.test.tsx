// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import UserInputQueue from "./UserInputQueue";
import type { PendingUserInputSummary } from "../../../browser/types";

afterEach(cleanup);

const request = (overrides: Partial<PendingUserInputSummary> = {}): PendingUserInputSummary => ({
  id: "approval-1",
  runId: "run-1",
  source: { kind: "agent", executionId: "execution-1", displayTitle: "Data Agent" },
  questions: [{
    id: "route_method",
    header: "Route method",
    question: "How should distance be calculated?",
    isOther: true,
    isSecret: false,
    options: [{ label: "Haversine", description: "Use a deterministic estimate" }],
  }],
  state: "pending",
  version: 3,
  autoResolutionMs: null,
  createdAt: "2026-08-07T00:00:00Z",
  ...overrides,
});

describe("UserInputQueue", () => {
  it("renders multiple agent requests without collapsing them", () => {
    render(
      <UserInputQueue
        requests={[
          request(),
          request({
            id: "approval-2",
            source: { kind: "agent", executionId: "execution-2", displayTitle: "Network Agent" },
          }),
        ]}
        submittingIds={new Set()}
        onSubmit={vi.fn()}
      />,
    );

    expect(screen.getByRole("group", { name: "Data Agent needs your input" })).toBeTruthy();
    expect(screen.getByRole("group", { name: "Network Agent needs your input" })).toBeTruthy();
  });

  it("labels a root Thread request without naming a role", () => {
    render(
      <UserInputQueue
        requests={[request({ source: { kind: "root" } })]}
        submittingIds={new Set()}
        onSubmit={vi.fn()}
      />,
    );

    expect(screen.getByRole("group", { name: "Your input is needed" })).toBeTruthy();
    expect(screen.queryByText(/Supervisor/)).toBeNull();
  });

  it("keeps preset and custom answers mutually exclusive", () => {
    const onSubmit = vi.fn();
    render(<UserInputQueue requests={[request()]} submittingIds={new Set()} onSubmit={onSubmit} />);
    const card = screen.getByRole("group", { name: "Data Agent needs your input" });
    const option = within(card).getByRole("button", { name: /Haversine/ });
    const custom = within(card).getByRole("button", { name: "Custom input" });
    const submit = within(card).getByRole("button", { name: "Submit" });

    fireEvent.click(custom);
    expect(within(card).getByPlaceholderText("Type your answer")).toBeTruthy();
    fireEvent.click(option);
    expect(within(card).queryByPlaceholderText("Type your answer")).toBeNull();
    expect((submit as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(submit);

    expect(onSubmit).toHaveBeenCalledWith("approval-1", 3, {
      route_method: { answers: ["Haversine"] },
    });
  });

  it("uses a password field for secret answers", () => {
    const onSubmit = vi.fn();
    render(
      <UserInputQueue
        requests={[request({
          questions: [{
            id: "api_key",
            header: "API key",
            question: "Enter the API key",
            isOther: false,
            isSecret: true,
            options: [],
          }],
        })]}
        submittingIds={new Set()}
        onSubmit={onSubmit}
      />,
    );

    const card = screen.getByRole("group", { name: "Data Agent needs your input" });
    const input = within(card).getByPlaceholderText("Type your answer");
    expect(input.getAttribute("type")).toBe("password");
    fireEvent.change(input, { target: { value: "secret-value" } });
    fireEvent.click(within(card).getByRole("button", { name: "Submit" }));

    expect(onSubmit).toHaveBeenCalledWith("approval-1", 3, {
      api_key: { answers: ["secret-value"] },
    });
  });
});
