import { describe, expect, it } from "vitest";
import type { MapReplyCard as MapReplyCardData } from "../../../utils/replyCards";
import {
  dataBoundsForSources,
  MAP_CARD_PROJECTION,
  mapboxLayerForCard,
  mapboxSourceForCard,
  mapStyleForToken,
  sameMapReplyCard,
} from "./MapReplyCard";

function card(overrides: Partial<MapReplyCardData> = {}): MapReplyCardData {
  return {
    type: "card",
    kind: "map.v3",
    id: "map-1",
    title: "Map",
    intent: "visualization",
    status: "ready",
    viewport: { mode: "fit" },
    sources: [
      {
        id: "data",
        options: { cluster: true },
        data: {
          type: "inline",
          format: "geojson",
          geojson: { type: "FeatureCollection", features: [] },
        },
      },
    ],
    layers: [
      {
        id: "points",
        type: "circle",
        source: "data",
        paint: { "circle-color": "#ef4444" },
      },
    ],
    ...overrides,
  };
}

describe("Mapbox map.v3 rendering", () => {
  it("keeps the complete layer and replaces only id/source", () => {
    const layer = card().layers[0];
    expect(mapboxLayerForCard(layer)).toEqual({
      id: "reply-layer-points",
      type: "circle",
      source: "reply-source-data",
      paint: { "circle-color": "#ef4444" },
    });
  });

  it("keeps GeoJSON source options while replacing its data", () => {
    const current = card();
    expect(
      mapboxSourceForCard(
        {
          id: "data",
          data: { type: "FeatureCollection", features: [] },
        },
        current,
      ),
    ).toEqual({
      type: "geojson",
      cluster: true,
      data: { type: "FeatureCollection", features: [] },
    });
  });

  it("keeps an unchanged parsed card stable", () => {
    expect(sameMapReplyCard(card(), card())).toBe(true);
    expect(sameMapReplyCard(card(), card({ title: "Changed" }))).toBe(false);
  });

  it("fits all loaded GeoJSON and expands a single point", () => {
    expect(
      dataBoundsForSources([
        {
          id: "places",
          data: {
            type: "FeatureCollection",
            features: [
              {
                type: "Feature",
                properties: {},
                geometry: { type: "Point", coordinates: [106.8456, -6.2088] },
              },
            ],
          },
        },
      ]),
    ).toEqual([106.7656, -6.2888, 106.9256, -6.1288]);
  });

  it("uses the platform basemap and Mercator projection", () => {
    expect(mapStyleForToken("pk.public-token")).toBe(
      "mapbox://styles/mapbox/streets-v12",
    );
    expect(mapStyleForToken("")).toBeNull();
    expect(MAP_CARD_PROJECTION).toBe("mercator");
  });
});
