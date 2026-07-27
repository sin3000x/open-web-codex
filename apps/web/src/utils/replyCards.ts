export type MapBounds = [number, number, number, number];

export type MapViewport =
  | {
    mode: "fit";
      padding?:
        number | { top: number; right: number; bottom: number; left: number };
    maxZoom?: number;
    minZoom?: number;
  }
  | {
    mode: "camera";
    center: [number, number];
    zoom: number;
    bearing?: number;
    pitch?: number;
  };

export type MapSourceData =
  | { type: "inline"; format: "geojson"; geojson: GeoJson }
  | {
    type: "artifact";
    format: "geojson";
    artifactId: string;
    mimeType?: string;
    url: string;
  };

export type MapSource = {
  id: string;
  data: MapSourceData;
  options?: Record<string, unknown>;
};

export type MapboxLayer = Record<string, unknown> & {
  id: string;
  type: string;
  source?: string;
};

export type MapHoverLayer = {
  layer: string;
  titleProperty?: string;
  fields: Array<{ property: string; label?: string }>;
};

export type MapReplyCard = {
  type: "card";
  kind: "map.v3";
  id: string;
  title: string;
  intent: string;
  fallbackText?: string;
  summary?: string;
  status: "loading" | "ready" | "error";
  viewport: MapViewport;
  sources: MapSource[];
  layers: MapboxLayer[];
  extensions?: {
    hover?: { layers: MapHoverLayer[] };
    legend?: {
      title?: string;
      items: Array<{
        label: string;
        color: string;
        type?: "circle" | "line" | "fill";
      }>;
    };
  };
};

export type ReplyCard = MapReplyCard;
export type InlineVisualizationArtifact = {
  ref: string;
  rendererKind: "map.v3";
  card: MapReplyCard;
};

export type GeoJson = Record<string, unknown> & { type: string };

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value && typeof value === "object" && !Array.isArray(value));
}

function nonemptyString(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() ? value.trim() : undefined;
}

function finiteNumber(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value)
    ? value
    : undefined;
}

function geoJson(value: unknown): GeoJson | undefined {
  if (!isRecord(value) || !nonemptyString(value.type)) return undefined;
  return value as GeoJson;
}

type MapLegend = NonNullable<
  NonNullable<MapReplyCard["extensions"]>["legend"]
>;

function legend(value: unknown): MapLegend | undefined {
  if (!isRecord(value) || !Array.isArray(value.items)) return undefined;
  const items = value.items.map((item) => {
    if (!isRecord(item)) return undefined;
    const label = nonemptyString(item.label);
    const color = nonemptyString(item.color);
    if (
      item.type != null &&
      item.type !== "circle" &&
      item.type !== "line" &&
      item.type !== "fill"
    ) {
      return undefined;
    }
    const type: "circle" | "line" | "fill" | undefined =
      item.type === "circle" || item.type === "line" || item.type === "fill"
        ? item.type
        : undefined;
    return label && color ? { label, color, type } : undefined;
  });
  if (!items.length || items.some((item) => !item)) return undefined;
  return {
    title: nonemptyString(value.title),
    items: items as MapLegend["items"],
  };
}

function v3Source(id: string, value: unknown): MapSource | undefined {
  if (!isRecord(value) || value.type !== "geojson" || !isRecord(value.data))
    return undefined;
  const options = Object.fromEntries(
    Object.entries(value).filter(([key]) => key !== "data" && key !== "type"),
  );
  const base = { id, options };
  if (value.data.type === "inline" && value.data.format === "geojson") {
    const data = geoJson(value.data.geojson);
    return data
      ? {
        ...base,
        data: { type: "inline", format: "geojson", geojson: data },
      }
      : undefined;
  }
  if (value.data.type !== "artifact" || value.data.format !== "geojson")
    return undefined;
  const artifactId = nonemptyString(value.data.artifact_id);
  const url = nonemptyString(value.data.url);
  if (!artifactId || !url || !url.startsWith("/api/runs/")) return undefined;
  return {
    ...base,
    data: {
      type: "artifact",
      format: "geojson",
      artifactId,
      mimeType: nonemptyString(value.data.mime_type),
      url,
    },
  };
}

function mapboxLayer(value: unknown): MapboxLayer | undefined {
  if (!isRecord(value)) return undefined;
  const id = nonemptyString(value.id);
  const type = nonemptyString(value.type);
  const sourceId =
    value.source == null ? undefined : nonemptyString(value.source);
  if (!id || !type || (value.source != null && !sourceId)) return undefined;
  return value as MapboxLayer;
}

function hoverExtensions(value: unknown): MapHoverLayer[] | undefined {
  if (!isRecord(value) || !Array.isArray(value.layers)) return undefined;
  const layers = value.layers.map((entry) => {
    if (!isRecord(entry) || !Array.isArray(entry.fields)) return undefined;
    const layerId = nonemptyString(entry.layer);
    const titleProperty = nonemptyString(entry.title_property);
    const fields = entry.fields.map((field) => {
      if (typeof field === "string") {
        const property = nonemptyString(field);
        return property ? { property } : undefined;
      }
      if (!isRecord(field)) return undefined;
      const property = nonemptyString(field.property);
      return property
        ? { property, label: nonemptyString(field.label) }
        : undefined;
    });
    if (!layerId || fields.some((field) => !field)) return undefined;
    return {
      layer: layerId,
      titleProperty,
      fields: fields as MapHoverLayer["fields"],
    };
  });
  return layers.some((layer) => !layer)
    ? undefined
    : (layers as MapHoverLayer[]);
}

function parseMapRendererPayload(
  value: unknown,
  artifactRef: string,
): MapReplyCard | null {
  if (!isRecord(value)) {
    return null;
  }
  const title = nonemptyString(value.title);
  const intent = nonemptyString(value.intent);
  const status = value.status;
  let normalizedViewport: MapViewport;
  if (value.center == null && value.zoom == null) {
    normalizedViewport = { mode: "fit" };
  } else {
    if (!Array.isArray(value.center) || value.center.length !== 2) return null;
    const longitude = finiteNumber(value.center[0]);
    const latitude = finiteNumber(value.center[1]);
    const zoom = finiteNumber(value.zoom);
    if (longitude == null || latitude == null || zoom == null) return null;
    normalizedViewport = {
      mode: "camera",
      center: [longitude, latitude],
      zoom,
      bearing: finiteNumber(value.bearing),
      pitch: finiteNumber(value.pitch),
    };
  }
  if (
    !title ||
    !intent ||
    !isRecord(value.sources) ||
    !Array.isArray(value.layers) ||
    !["loading", "ready", "error"].includes(String(status))
  ) {
    return null;
  }
  const sources = Object.entries(value.sources).map(([id, entry]) =>
    v3Source(id, entry),
  );
  const layers = value.layers.map(mapboxLayer);
  const extensions = isRecord(value.extensions) ? value.extensions : undefined;
  const normalizedLegend = legend(extensions?.legend);
  const normalizedHover = hoverExtensions(extensions?.hover);
  if (sources.some((entry) => !entry) || layers.some((entry) => !entry))
    return null;
  if (extensions?.legend != null && !normalizedLegend) return null;
  if (extensions?.hover != null && !normalizedHover) return null;
  const sourceIds = new Set((sources as MapSource[]).map((entry) => entry.id));
  if (
    (layers as MapboxLayer[]).some(
      (entry) => entry.source && !sourceIds.has(entry.source),
    )
  )
    return null;
  return {
    type: "card",
    kind: "map.v3",
    id: artifactRef,
    title,
    intent,
    fallbackText: nonemptyString(value.fallback_text),
    summary: nonemptyString(value.summary),
    status: status as MapReplyCard["status"],
    viewport: normalizedViewport,
    sources: sources as MapSource[],
    layers: layers as MapboxLayer[],
    extensions:
      normalizedLegend || normalizedHover
        ? {
          legend: normalizedLegend,
          hover: normalizedHover ? { layers: normalizedHover } : undefined,
        }
        : undefined,
  };
}

export function parseInlineVisualizationArtifact(
  value: unknown,
): InlineVisualizationArtifact | null {
  if (!isRecord(value) || !isRecord(value.renderer)) return null;
  const ref = nonemptyString(value.ref);
  if (
    !ref ||
    !/^[A-Za-z0-9_.-]{1,128}$/.test(ref) ||
    value.renderer.kind !== "map.v3"
  ) {
    return null;
  }
  const card = parseMapRendererPayload(value.renderer.payload, ref);
  return card ? { ref, rendererKind: "map.v3", card } : null;
}
