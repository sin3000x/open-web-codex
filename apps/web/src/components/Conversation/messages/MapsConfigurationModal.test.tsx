// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import MapsConfigurationModal from "./MapsConfigurationModal";

const saveMaps = vi.fn();

vi.mock("../../../services/mapsConfiguration", () => ({
  saveMapsConfiguration: (...args: unknown[]) => saveMaps(...args),
  useMapsConfiguration: () => ({
    configured: true,
    provider: "mapbox",
    mapboxAccessToken: "pk.saved-token",
    canConfigure: true,
    updatedAt: null,
    loading: false,
    error: null,
  }),
}));

describe("MapsConfigurationModal", () => {
  afterEach(() => {
    cleanup();
    vi.clearAllMocks();
  });

  it("selects Google and replaces the active provider through one endpoint", async () => {
    saveMaps.mockResolvedValue({
      configured: true,
      provider: "google",
      mapboxAccessToken: null,
      canConfigure: true,
      updatedAt: "2026-07-23T00:00:00Z",
    });
    const onSaved = vi.fn();
    render(
      <MapsConfigurationModal
        initialProvider="mapbox"
        elicitationUrl="http://127.0.0.1:43123/one-time-token"
        onClose={vi.fn()}
        onSaved={onSaved}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Google" }));
    fireEvent.change(screen.getByLabelText("Google Maps API Key"), {
      target: { value: "google-secret" },
    });
    fireEvent.click(screen.getByRole("button", { name: "保存并继续" }));

    await waitFor(() => expect(saveMaps).toHaveBeenCalledWith(
      "google",
      "google-secret",
      "http://127.0.0.1:43123/one-time-token",
    ));
    expect(onSaved).toHaveBeenCalledWith("google");
  });

  it("keeps an active Mapbox token available for map cards", async () => {
    saveMaps.mockResolvedValue({
      configured: true,
      provider: "mapbox",
      mapboxAccessToken: "pk.saved-token",
      canConfigure: true,
      updatedAt: null,
    });
    const onSaved = vi.fn();
    render(
      <MapsConfigurationModal
        initialProvider="mapbox"
        onClose={vi.fn()}
        onSaved={onSaved}
      />,
    );

    expect((screen.getByLabelText("Mapbox public token") as HTMLInputElement).value)
      .toBe("pk.saved-token");
    fireEvent.click(screen.getByRole("button", { name: "保存配置" }));

    await waitFor(() => expect(saveMaps).toHaveBeenCalledWith(
      "mapbox",
      "pk.saved-token",
      undefined,
    ));
    expect(onSaved).toHaveBeenCalledWith("mapbox");
  });
});
