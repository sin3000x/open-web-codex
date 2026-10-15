// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import MapsSettings from "./MapsSettings";

const { saveMaps, configuration } = vi.hoisted(() => ({
  saveMaps: vi.fn(),
  configuration: {
    configured: true, provider: "mapbox", mapboxAccessToken: "pk.old-token",
    canConfigure: true, loading: false, error: null as string | null,
  },
}));
vi.mock("../../services/mapsConfiguration", () => ({
  useMapsConfiguration: () => configuration,
  saveMapsConfiguration: (...args: unknown[]) => saveMaps(...args),
}));

describe("map settings", () => {
  beforeEach(() => { configuration.canConfigure = true; configuration.error = null; saveMaps.mockReset(); });
  afterEach(cleanup);

  it("lets a configured Token be replaced without an active conversation approval", async () => {
    saveMaps.mockResolvedValue({});
    render(<MapsSettings />);
    expect(screen.queryByText("pk.old-token")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "配置 / 更换地图 Token" }));
    fireEvent.change(screen.getByLabelText("Mapbox public token"), { target: { value: "pk.replacement" } });
    fireEvent.click(screen.getByRole("button", { name: "保存配置" }));
    await waitFor(() => expect(saveMaps).toHaveBeenCalledWith("mapbox", "pk.replacement", undefined));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.getByRole("status").textContent).toContain("地图配置已保存");
    expect(screen.queryByText("pk.replacement")).toBeNull();
  });

  it("keeps failed saves editable and does not report success", async () => {
    saveMaps.mockRejectedValue(new Error("保存失败"));
    render(<MapsSettings />);
    fireEvent.click(screen.getByRole("button", { name: "配置 / 更换地图 Token" }));
    fireEvent.click(screen.getByRole("button", { name: "保存配置" }));
    await waitFor(() => expect(screen.getByRole("alert").textContent).toBe("保存失败"));
    expect(screen.getByRole("dialog")).toBeTruthy();
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("respects the platform's configuration permission and read failure", () => {
    configuration.canConfigure = false;
    const view = render(<MapsSettings />);
    expect((screen.getByRole("button") as HTMLButtonElement).disabled).toBe(true);
    configuration.canConfigure = true;
    configuration.error = "配置读取失败";
    view.rerender(<MapsSettings />);
    expect((screen.getByRole("button") as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByRole("alert").textContent).toBe("配置读取失败");
  });
});
