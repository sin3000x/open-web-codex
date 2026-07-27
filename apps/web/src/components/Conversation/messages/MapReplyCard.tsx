import { memo, useEffect, useMemo, useRef, useState } from "react";
import Expand from "lucide-react/dist/esm/icons/expand";
import KeyRound from "lucide-react/dist/esm/icons/key-round";
import MapPinned from "lucide-react/dist/esm/icons/map-pinned";
import type { Map as MapboxMap } from "mapbox-gl";
import "mapbox-gl/dist/mapbox-gl.css";
import { platformClient } from "../../../../browser/session";
import type {
  GeoJson,
  MapBounds,
  MapHoverLayer,
  MapReplyCard as MapReplyCardData,
} from "../../../utils/replyCards";
import { useMapsConfiguration } from "../../../services/mapsConfiguration";
import MapsConfigurationModal from "./MapsConfigurationModal";

type Props = {
  card: MapReplyCardData;
};

type LoadedSource = {
  id: string;
  data: GeoJson;
};

type MapLoadState = "loading" | "ready" | "error";
type MapboxModule = typeof import("mapbox-gl");

export const MAP_CARD_PROJECTION = "mercator" as const;

export function sameMapReplyCard(
  left: MapReplyCardData,
  right: MapReplyCardData,
): boolean {
  return left === right || JSON.stringify(left) === JSON.stringify(right);
}

function statusLabel(status: MapReplyCardData["status"]) {
  if (status === "ready") return "Ready";
  if (status === "error") return "Failed";
  return "Loading data";
}

function extendBounds(
  bounds: MapBounds | null,
  longitude: number,
  latitude: number,
): MapBounds {
  if (!bounds) return [longitude, latitude, longitude, latitude];
  return [
    Math.min(bounds[0], longitude),
    Math.min(bounds[1], latitude),
    Math.max(bounds[2], longitude),
    Math.max(bounds[3], latitude),
  ];
}

function collectCoordinates(
  value: unknown,
  visit: (longitude: number, latitude: number) => void,
) {
  if (Array.isArray(value)) {
    if (
      value.length >= 2 &&
      typeof value[0] === "number" &&
      Number.isFinite(value[0]) &&
      typeof value[1] === "number" &&
      Number.isFinite(value[1])
    ) {
      visit(value[0], value[1]);
      return;
    }
    for (const entry of value) collectCoordinates(entry, visit);
    return;
  }
  if (!value || typeof value !== "object") return;
  const record = value as Record<string, unknown>;
  if (record.type === "FeatureCollection")
    collectCoordinates(record.features, visit);
  else if (record.type === "Feature")
    collectCoordinates(record.geometry, visit);
  else if (record.type === "GeometryCollection")
    collectCoordinates(record.geometries, visit);
  else collectCoordinates(record.coordinates, visit);
}

export function dataBoundsForSources(
  sources: LoadedSource[],
): MapBounds | null {
  let bounds: MapBounds | null = null;
  for (const source of sources) {
    collectCoordinates(source.data, (longitude, latitude) => {
      if (
        longitude >= -180 &&
        longitude <= 180 &&
        latitude >= -90 &&
        latitude <= 90
      ) {
        bounds = extendBounds(bounds, longitude, latitude);
      }
    });
  }
  if (!bounds) return null;
  if (bounds[0] === bounds[2] && bounds[1] === bounds[3]) {
    return [
      bounds[0] - 0.08,
      bounds[1] - 0.08,
      bounds[2] + 0.08,
      bounds[3] + 0.08,
    ];
  }
  return bounds;
}

export function mapStyleForToken(token: string): string | null {
  return token ? "mapbox://styles/mapbox/streets-v12" : null;
}

export function mapboxSourceForCard(source: LoadedSource, card: MapReplyCardData) {
  return {
    ...card.sources.find((entry) => entry.id === source.id)?.options,
    type: "geojson" as const,
    data: source.data,
  };
}

export function mapboxLayerForCard(layer: MapReplyCardData["layers"][number]) {
  return {
    ...layer,
    id: `reply-layer-${layer.id}`,
    ...(layer.source ? { source: `reply-source-${layer.source}` } : {}),
  };
}

function fitOptions(card: MapReplyCardData, fullscreen: boolean) {
  if (card.viewport.mode !== "fit") return undefined;
  return {
    padding: card.viewport.padding ?? (fullscreen ? 72 : 40),
    maxZoom: card.viewport.maxZoom ?? 14,
    duration: 0,
  };
}

function hoverValue(value: unknown): string {
  if (value == null || value === "") return "—";
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean")
    return String(value);
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

function hoverExtensionContent(
  hover: MapHoverLayer,
  properties: Record<string, unknown>,
): HTMLElement {
  const root = document.createElement("div");
  root.className = "web-map-card-hover-content";
  if (hover.titleProperty) {
    const title = document.createElement("strong");
    title.textContent = hoverValue(properties[hover.titleProperty]);
    root.append(title);
  }
  if (hover.fields.length) {
    const details = document.createElement("dl");
    for (const field of hover.fields) {
      const label = document.createElement("dt");
      label.textContent = field.label ?? field.property;
      const value = document.createElement("dd");
      value.textContent = hoverValue(properties[field.property]);
      details.append(label, value);
    }
    root.append(details);
  }
  return root;
}

function attachHoverExtension(
  map: MapboxMap,
  mapboxgl: MapboxModule["default"],
  hover: MapHoverLayer | undefined,
  layerId: string,
): () => void {
  if (!hover) return () => {};
  const popup = new mapboxgl.Popup({
    closeButton: false,
    closeOnClick: false,
    offset: 12,
    className: "web-map-card-hover",
  });
  const canvas = map.getCanvas();
  const onMove = (event: {
    features?: Array<{ properties?: Record<string, unknown> | null }>;
    lngLat: import("mapbox-gl").LngLatLike;
  }) => {
    canvas.style.cursor = "pointer";
    popup
      .setLngLat(event.lngLat)
      .setDOMContent(
        hoverExtensionContent(
          hover,
          event.features?.[0]?.properties ?? {},
        ),
      )
      .addTo(map);
  };
  const onLeave = () => {
    canvas.style.cursor = "";
    popup.remove();
  };
  map.on("mousemove", layerId, onMove as never);
  map.on("mouseleave", layerId, onLeave);
  return () => {
    map.off("mousemove", layerId, onMove as never);
    map.off("mouseleave", layerId, onLeave);
    onLeave();
  };
}

function useLoadedSources(card: MapReplyCardData) {
  const [sources, setSources] = useState<LoadedSource[]>([]);
  const [error, setError] = useState("");
  useEffect(() => {
    let disposed = false;
    setSources([]);
    setError("");
    void Promise.all(
      card.sources.map(async (source): Promise<LoadedSource> => {
      if (source.data.type === "inline") {
        return { id: source.id, data: source.data.geojson };
      }
      const data = await platformClient.readReplyArtifact(source.data.url);
      if (!data || typeof data.type !== "string") {
        throw new Error("Reply Artifact did not contain GeoJSON.");
      }
      return { id: source.id, data: data as GeoJson };
      }),
    )
      .then((loaded) => {
        if (!disposed) setSources(loaded);
      })
      .catch((reason: unknown) => {
        if (!disposed) {
          setError(
            reason instanceof Error
              ? reason.message
              : "Map data failed to load.",
          );
        }
      });
    return () => {
      disposed = true;
    };
  }, [card.sources]);
  return { sources, error };
}

function MapCanvas({
  card,
  fullscreen = false,
  accessToken,
  configurationLoading,
  canConfigure,
  onConfigure,
}: {
  card: MapReplyCardData;
  fullscreen?: boolean;
  accessToken: string;
  configurationLoading: boolean;
  canConfigure: boolean;
  onConfigure: () => void;
}) {
  const mapElement = useRef<HTMLDivElement | null>(null);
  const mapInstance = useRef<MapboxMap | null>(null);
  const [loadState, setLoadState] = useState<MapLoadState>("loading");
  const [loadError, setLoadError] = useState("");
  const loaded = useLoadedSources(card);
  const bounds = useMemo(
    () => dataBoundsForSources(loaded.sources),
    [loaded.sources],
  );
  const mapStyle = mapStyleForToken(accessToken);

  useEffect(() => {
    const container = mapElement.current;
    if (!container || !loaded.sources.length || !mapStyle || loaded.error)
      return;
    if (card.viewport.mode === "fit" && !bounds) {
      setLoadState("error");
      setLoadError("GeoJSON does not contain valid coordinates.");
      return;
    }
    if (import.meta.env.MODE === "test") {
      setLoadState("ready");
      return;
    }

    let disposed = false;
    let resizeObserver: ResizeObserver | null = null;
    let fittedAfterLayout = false;
    const hoverCleanups: Array<() => void> = [];
    setLoadState("loading");
    setLoadError("");
    const applyViewport = (map: MapboxMap) => {
      map.resize();
      if (card.viewport.mode === "camera") {
        map.jumpTo({
          center: card.viewport.center,
          zoom: card.viewport.zoom,
          bearing: card.viewport.bearing ?? 0,
          pitch: card.viewport.pitch ?? 0,
        });
      } else if (bounds) {
        map.fitBounds(bounds, fitOptions(card, fullscreen));
        if (
          card.viewport.minZoom != null &&
          map.getZoom() < card.viewport.minZoom
        ) {
          map.setZoom(card.viewport.minZoom);
        }
      }
    };

    void import("mapbox-gl")
      .then((module) => {
        if (disposed || !mapElement.current) return;
        const mapboxgl = module.default;
        mapboxgl.accessToken = accessToken;
        const camera =
          card.viewport.mode === "camera"
          ? {
            center: card.viewport.center,
            zoom: card.viewport.zoom,
            bearing: card.viewport.bearing ?? 0,
            pitch: card.viewport.pitch ?? 0,
          }
          : { center: [0, 0] as [number, number], zoom: 0 };
        const map = new mapboxgl.Map({
          container: mapElement.current,
          style: mapStyle,
          projection: MAP_CARD_PROJECTION,
          ...camera,
          attributionControl: true,
        });
        mapInstance.current = map;
        map.addControl(new mapboxgl.NavigationControl(), "top-right");
        map.once("load", () => {
          void (async () => {
            if (disposed) return;
            for (const source of loaded.sources) {
              map.addSource(
                `reply-source-${source.id}`,
                mapboxSourceForCard(source, card) as never,
              );
            }
            for (const layer of card.layers) {
              const layerId = `reply-layer-${layer.id}`;
              map.addLayer(mapboxLayerForCard(layer) as never);
              if (disposed) return;
              const hover = card.extensions?.hover?.layers.find(
                (entry) => entry.layer === layer.id,
              );
              hoverCleanups.push(
                attachHoverExtension(map, mapboxgl, hover, layerId),
              );
            }
            applyViewport(map);
            setLoadState("ready");
          })().catch((reason: unknown) => {
            if (disposed) return;
            setLoadState("error");
            setLoadError(
              reason instanceof Error
                ? reason.message
                : "Map layers failed to load.",
            );
          });
        });
        map.on("error", (event) => {
          if (disposed) return;
          setLoadState("error");
          setLoadError(event.error?.message ?? "Mapbox GL failed to load.");
        });
        if (typeof ResizeObserver !== "undefined") {
          resizeObserver = new ResizeObserver(() => {
            map.resize();
            if (
              !fittedAfterLayout &&
              container.clientWidth > 0 &&
              container.clientHeight > 0
            ) {
              fittedAfterLayout = true;
              applyViewport(map);
            }
          });
          resizeObserver.observe(container);
        }
      })
      .catch((reason: unknown) => {
        if (disposed) return;
        setLoadState("error");
        setLoadError(
          reason instanceof Error
            ? reason.message
            : "Mapbox GL failed to load.",
        );
      });

    return () => {
      disposed = true;
      for (const cleanup of hoverCleanups) cleanup();
      resizeObserver?.disconnect();
      mapInstance.current?.remove();
      mapInstance.current = null;
    };
  }, [
    accessToken,
    bounds,
    card,
    fullscreen,
    loaded.error,
    loaded.sources,
    mapStyle,
  ]);

  if (loaded.error) {
    return (
      <div className="web-map-card-canvas" role="alert">
        <MapPinned size={28} aria-hidden="true" />
        <strong>地图数据加载失败</strong>
        <span>{loaded.error}</span>
      </div>
    );
  }
  if (!loaded.sources.length) {
    return (
      <div
        className="web-map-card-canvas"
        role="status"
        aria-label="Map data loading"
      >
        <MapPinned size={28} aria-hidden="true" />
        <strong>正在读取地图数据</strong>
      </div>
    );
  }
  if (!mapStyle) {
    return (
      <div className="web-map-card-map-frame" data-map-state="token-required">
        <div className="web-map-card-map-state is-token-required" role="alert">
          <div className="web-map-card-token-prompt">
            <KeyRound size={24} aria-hidden="true" />
            <strong>
              {configurationLoading
                ? "正在读取 Mapbox 配置"
                : "需要公开 Mapbox Token"}
            </strong>
            <button
              type="button"
              className="web-map-card-configure"
              onClick={onConfigure}
              disabled={!canConfigure}
            >
              {canConfigure ? "配置 Mapbox Key" : "请联系管理员配置"}
            </button>
          </div>
        </div>
      </div>
    );
  }
  return (
    <div className="web-map-card-map-frame" data-map-state={loadState}>
      <div
        ref={mapElement}
        className="web-map-card-mapbox"
        role="region"
        aria-label="Interactive Mapbox map"
      />
      {loadState === "loading" ? (
        <div className="web-map-card-map-state" role="status">
          正在加载交互地图…
        </div>
      ) : null}
      {loadState === "error" ? (
        <div className="web-map-card-map-state is-error" role="alert">
          <strong>地图加载失败</strong>
          <span>{loadError}</span>
        </div>
      ) : null}
    </div>
  );
}

const MapReplyCard = memo(
  function MapReplyCard({ card }: Props) {
  const [fullscreen, setFullscreen] = useState(false);
  const [configurationOpen, setConfigurationOpen] = useState(false);
  const mapsConfiguration = useMapsConfiguration();
  const detail = card.summary ?? card.fallbackText ?? "地图数据已就绪。";
  const body = (fullscreenBody = false) => (
    <div
      className={`web-map-card is-${card.status}`}
      role="group"
      aria-label={`Map card: ${card.title}`}
    >
      <div className="web-map-card-header">
        <div className="web-map-card-title">
          <MapPinned size={16} aria-hidden="true" />
          <span>{card.title}</span>
        </div>
        <span className="web-map-card-status">{statusLabel(card.status)}</span>
        {!fullscreenBody ? (
          <button
            type="button"
            className="web-map-card-fullscreen"
            onClick={() => setFullscreen(true)}
            aria-label="Open map card fullscreen"
          >
            <Expand size={16} aria-hidden="true" />
            <span>全屏</span>
          </button>
        ) : null}
      </div>
      <MapCanvas
        card={card}
        fullscreen={fullscreenBody}
        accessToken={mapsConfiguration.mapboxAccessToken ?? ""}
        configurationLoading={mapsConfiguration.loading}
        canConfigure={mapsConfiguration.canConfigure}
        onConfigure={() => setConfigurationOpen(true)}
      />
      <div className="web-map-card-body">
        <p>{detail}</p>
        {card.extensions?.legend ? (
          <div
            className="web-map-card-legend"
            aria-label={card.extensions.legend.title ?? "Map legend"}
          >
            {card.extensions.legend.title ? (
              <strong>{card.extensions.legend.title}</strong>
            ) : null}
            {card.extensions.legend.items.map((item) => (
              <span key={`${item.label}-${item.color}-${item.type ?? "circle"}`}>
                <i
                  data-type={item.type ?? "circle"}
                  style={{ backgroundColor: item.color }}
                />
                {item.label}
              </span>
            ))}
          </div>
        ) : null}
      </div>
    </div>
  );
  return (
    <>
      {body(false)}
      {fullscreen ? (
        <div
          className="web-map-card-modal"
          role="dialog"
          aria-modal="true"
          aria-label={`Fullscreen map card: ${card.title}`}
        >
            <div
              className="web-map-card-modal-backdrop"
              onClick={() => setFullscreen(false)}
            />
          <div className="web-map-card-modal-panel">
            <button
              type="button"
              className="web-map-card-modal-close"
              onClick={() => setFullscreen(false)}
            >
              Close
            </button>
            {body(true)}
          </div>
        </div>
      ) : null}
      {configurationOpen ? (
        <MapsConfigurationModal
          initialProvider="mapbox"
          onClose={() => setConfigurationOpen(false)}
          onSaved={() => setConfigurationOpen(false)}
        />
      ) : null}
    </>
  );
  },
  (previous, next) => sameMapReplyCard(previous.card, next.card),
);

export default MapReplyCard;
