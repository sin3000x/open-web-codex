// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import Layout from "./Layout";

describe("Layout", () => {
  afterEach(cleanup);

  it("dismisses an expanded sidebar from its narrow-screen scrim", () => {
    const onDismissSidebar = vi.fn();
    render(
      <Layout
        sidebar={<aside>Projects</aside>}
        sidebarCollapsed={false}
        onDismissSidebar={onDismissSidebar}
      >
        <div>Conversation</div>
      </Layout>,
    );

    fireEvent.click(screen.getByRole("button", { name: "Hide projects panel" }));
    expect(onDismissSidebar).toHaveBeenCalledOnce();
  });

  it("does not render the scrim while the sidebar is collapsed", () => {
    render(
      <Layout sidebar={<aside>Projects</aside>} sidebarCollapsed>
        <div>Conversation</div>
      </Layout>,
    );

    expect(screen.queryByRole("button", { name: "Hide projects panel" })).toBeNull();
  });
});
