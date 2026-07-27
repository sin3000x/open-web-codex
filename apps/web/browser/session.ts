import { PlatformClient } from "./client";

export const PLATFORM_SESSION_KEY = "owc.session";

function storedToken() {
  return typeof sessionStorage === "undefined"
    ? ""
    : sessionStorage.getItem(PLATFORM_SESSION_KEY) ?? "";
}

export const platformClient = new PlatformClient({ token: storedToken() });

export function setPlatformSessionToken(token: string) {
  const normalized = token.trim();
  platformClient.setToken(normalized);
  if (typeof sessionStorage === "undefined") return;
  if (normalized) sessionStorage.setItem(PLATFORM_SESSION_KEY, normalized);
  else sessionStorage.removeItem(PLATFORM_SESSION_KEY);
}

export function getPlatformSessionToken() {
  return storedToken();
}
