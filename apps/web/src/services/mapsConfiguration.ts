import { useEffect, useSyncExternalStore } from "react";
import type {
  MapsConfiguration,
  MapsProvider,
} from "../../browser/types";
import { platformClient } from "../../browser/session";

const BUILD_TIME_TOKEN =
  import.meta.env.VITE_MAPBOX_ACCESS_TOKEN
  ?? import.meta.env.VITE_MAPBOX_TOKEN
  ?? "";

export type MapsConfigurationState = MapsConfiguration & {
  loading: boolean;
  error: string | null;
};

let state: MapsConfigurationState = {
  configured: Boolean(BUILD_TIME_TOKEN),
  provider: BUILD_TIME_TOKEN ? "mapbox" : null,
  mapboxAccessToken: BUILD_TIME_TOKEN || null,
  canConfigure: true,
  updatedAt: null,
  loading: false,
  error: null,
};
let loaded = false;
let loadPromise: Promise<MapsConfigurationState> | null = null;
const listeners = new Set<() => void>();

function publish(next: MapsConfigurationState) {
  state = next;
  for (const listener of listeners) listener();
  return state;
}

function serverState(configuration: MapsConfiguration): MapsConfigurationState {
  const fallbackToBuildToken = !configuration.configured && Boolean(BUILD_TIME_TOKEN);
  return {
    ...configuration,
    configured: configuration.configured || fallbackToBuildToken,
    provider: fallbackToBuildToken ? "mapbox" : configuration.provider,
    mapboxAccessToken:
      configuration.mapboxAccessToken || (fallbackToBuildToken ? BUILD_TIME_TOKEN : null),
    loading: false,
    error: null,
  };
}

export function loadMapsConfiguration(force = false) {
  if (loaded && !force) return Promise.resolve(state);
  if (loadPromise && !force) return loadPromise;
  publish({ ...state, loading: true, error: null });
  loadPromise = platformClient.getMapsConfiguration()
    .then((configuration) => {
      loaded = true;
      return publish(serverState(configuration));
    })
    .catch((error: unknown) => {
      loaded = true;
      return publish({
        ...state,
        loading: false,
        error: error instanceof Error ? error.message : "读取地图配置失败",
      });
    })
    .finally(() => {
      loadPromise = null;
    });
  return loadPromise;
}

export async function saveMapsConfiguration(
  provider: MapsProvider,
  apiKey: string,
  elicitationUrl?: string,
) {
  const configuration = await platformClient.updateMapsConfiguration(
    provider,
    apiKey,
    elicitationUrl,
  );
  loaded = true;
  return publish(serverState(configuration));
}

export async function applySavedMapsConfiguration(elicitationUrl: string) {
  const configuration = await platformClient.useMapsConfiguration(elicitationUrl);
  loaded = true;
  return publish(serverState(configuration));
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function getSnapshot() {
  return state;
}

export function useMapsConfiguration() {
  const configuration = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
  useEffect(() => {
    if (import.meta.env.MODE !== "test") {
      void loadMapsConfiguration();
    }
  }, []);
  return configuration;
}

export function resetMapsConfigurationForTests() {
  loaded = false;
  loadPromise = null;
  publish({
    configured: Boolean(BUILD_TIME_TOKEN),
    provider: BUILD_TIME_TOKEN ? "mapbox" : null,
    mapboxAccessToken: BUILD_TIME_TOKEN || null,
    canConfigure: true,
    updatedAt: null,
    loading: false,
    error: null,
  });
}
