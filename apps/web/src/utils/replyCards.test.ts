import { describe, expect, it } from "vitest";
import { parseInlineVisualizationArtifact } from "./replyCards";

function artifact(overrides: Record<string, unknown> = {}) {
  return {
    ref: "map-guangdong",
    renderer: {
      kind: "map.v3",
      payload: {
        title: "深圳到广东各地级市",
        intent: "展示驾车路线",
        status: "ready",
        sources: {
          routes: {
            type: "geojson",
            lineMetrics: true,
            data: {
              type: "inline",
              format: "geojson",
              geojson: { type: "FeatureCollection", features: [] },
            },
          },
        },
        layers: [
          {
            id: "routes",
            type: "line",
            source: "routes",
            minzoom: 4,
            layout: { "line-cap": "round" },
            paint: {
              "line-color": [
                "interpolate",
                ["linear"],
                ["zoom"],
                4,
                "#2563eb",
                12,
                "#ef4444",
              ],
              "line-width": 4,
            },
          },
        ],
        extensions: {
          hover: {
            layers: [
              {
                layer: "routes",
                title_property: "label",
                fields: ["distance", { property: "duration", label: "时长" }],
              },
            ],
          },
          legend: {
            title: "路线",
            items: [{ label: "驾车路线", color: "#2563eb", type: "line" }],
          },
        },
        ...overrides,
      },
    },
  };
}

describe("parseInlineVisualizationArtifact", () => {
  it("preserves Mapbox layer JSON and normalizes managed source data", () => {
    expect(parseInlineVisualizationArtifact(artifact())).toEqual({
      ref: "map-guangdong",
      rendererKind: "map.v3",
      card: {
        type: "card",
        kind: "map.v3",
        id: "map-guangdong",
        title: "深圳到广东各地级市",
        intent: "展示驾车路线",
        status: "ready",
        viewport: { mode: "fit" },
        sources: [
          {
            id: "routes",
            options: { lineMetrics: true },
            data: {
              type: "inline",
              format: "geojson",
              geojson: { type: "FeatureCollection", features: [] },
            },
          },
        ],
        layers: [
          {
            id: "routes",
            type: "line",
            source: "routes",
            minzoom: 4,
            layout: { "line-cap": "round" },
            paint: {
              "line-color": [
                "interpolate",
                ["linear"],
                ["zoom"],
                4,
                "#2563eb",
                12,
                "#ef4444",
              ],
              "line-width": 4,
            },
          },
        ],
        extensions: {
          hover: {
            layers: [
              {
                layer: "routes",
                titleProperty: "label",
                fields: [
                  { property: "distance" },
                  { property: "duration", label: "时长" },
                ],
              },
            ],
          },
          legend: {
            title: "路线",
            items: [{ label: "驾车路线", color: "#2563eb", type: "line" }],
          },
        },
      },
    });
  });

  it("normalizes standard camera fields", () => {
    expect(
      parseInlineVisualizationArtifact(
        artifact({ center: [114.0579, 22.5431], zoom: 8, bearing: 15, pitch: 30 }),
      )?.card.viewport,
    ).toEqual({
      mode: "camera",
      center: [114.0579, 22.5431],
      zoom: 8,
      bearing: 15,
      pitch: 30,
    });
  });

  it("rejects unknown source references", () => {
    expect(
      parseInlineVisualizationArtifact(
        artifact({
          layers: [{ id: "bad", type: "line", source: "missing" }],
        }),
      ),
    ).toBeNull();
  });
});
