// @vitest-environment jsdom
import { fireEvent, render, screen } from "@testing-library/react";
import { cleanup } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import ApprovalCard from "./ApprovalCard";

describe("ApprovalCard", () => {
  afterEach(cleanup);

  it("opens one in-app provider dialog for map_utils URL elicitation", () => {
    const onResolve = vi.fn();
    render(
      <ApprovalCard
        command="A maps provider and API key are required. Configure Mapbox or Google in this app; the selected provider will be saved globally and reused."
        workspaceId="workspace-1"
        requestId="approval-1"
        mode="url"
        url="http://127.0.0.1:43123/one-time-token"
        serverName="map_utils"
        onResolve={onResolve}
      />,
    );

    expect(screen.queryByRole("link", { name: "Configure key" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Accept" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "配置 Key" }));
    expect(
      screen.getByRole("dialog", { name: "配置地图服务 Key" }),
    ).toBeTruthy();
    expect(screen.getByRole("button", { name: "Mapbox" }).getAttribute("aria-pressed"))
      .toBe("true");
    expect(screen.getByRole("button", { name: "Google" })).toBeTruthy();
    expect(onResolve).not.toHaveBeenCalled();
  });

  it("keeps legacy workspace_maps requests compatible", () => {
    render(
      <ApprovalCard
        command="Mapbox access token is not configured. Configure it in this app to save it globally and reuse it automatically."
        workspaceId="workspace-1"
        requestId="approval-mapbox"
        mode="url"
        url="http://127.0.0.1:43123/mapbox-token"
        serverName="workspace_maps"
        onResolve={vi.fn()}
      />,
    );

    expect(screen.queryByRole("link", { name: "Configure key" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "配置 Key" }));
    expect(
      screen.getByRole("dialog", { name: "配置地图服务 Key" }),
    ).toBeTruthy();
    expect(screen.getByRole("button", { name: "Mapbox" }).getAttribute("aria-pressed"))
      .toBe("true");
    expect(screen.getByRole("button", { name: "Google" })).toBeTruthy();
  });

  it("does not expose an external URL as an MCP credential link", () => {
    const onResolve = vi.fn();
    render(
      <ApprovalCard
        command="Configure a key."
        workspaceId="workspace-1"
        requestId="approval-2"
        mode="url"
        url="https://example.com/credential"
        onResolve={onResolve}
      />,
    );

    expect(screen.queryByRole("link", { name: "Configure key" })).toBeNull();
    expect(screen.getByText(/secure configuration link is unavailable/i)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onResolve).toHaveBeenCalledWith("workspace-1", "approval-2", "decline");
  });
});
