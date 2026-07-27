// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import ApprovalStatusIcon from "./ApprovalStatusIcon";

describe("ApprovalStatusIcon", () => {
  afterEach(cleanup);

  it.each([
    ["accepted", "Approved"],
    ["declined", "Denied"],
    ["answered", "Other response"],
  ] as const)("renders the %s state with a visible hover explanation", (status, label) => {
    render(<ApprovalStatusIcon status={status} detail="Allow map_utils to continue?" />);

    const icon = screen.getByRole("img", {
      name: `${label}: Allow map_utils to continue?`,
    });
    expect(icon.getAttribute("title")).toBeNull();

    fireEvent.mouseEnter(icon);
    const tooltip = screen.getByRole("tooltip");
    expect(tooltip.textContent).toContain(label);
    expect(tooltip.textContent).toContain("Allow map_utils to continue?");

    fireEvent.mouseLeave(icon);
    expect(screen.queryByRole("tooltip")).toBeNull();

    fireEvent.focus(icon);
    expect(screen.getByRole("tooltip").textContent).toContain(label);
    fireEvent.blur(icon);
    expect(screen.queryByRole("tooltip")).toBeNull();
  });
});
