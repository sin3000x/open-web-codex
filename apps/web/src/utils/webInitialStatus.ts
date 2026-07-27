export type WebMcpServer = {
  name: string;
  status: string;
  error?: string | null;
  failureReason?: string | null;
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function unwrapResult(value: unknown): Record<string, unknown> {
  let current = value;
  while (isRecord(current) && isRecord(current.result)) current = current.result;
  return isRecord(current) ? current : {};
}

export function parseInitialMcpServers(value: unknown): Record<string, WebMcpServer> {
  const payload = unwrapResult(value);
  const data = Array.isArray(payload.data) ? payload.data : [];
  const servers: Record<string, WebMcpServer> = {};

  for (const candidate of data) {
    if (!isRecord(candidate) || typeof candidate.name !== "string" || !candidate.name) continue;
    const projectedStatus = typeof candidate.status === "string"
      ? candidate.status
      : null;
    const hasInventory = isRecord(candidate.serverInfo)
      || (isRecord(candidate.tools) && Object.keys(candidate.tools).length > 0)
      || (Array.isArray(candidate.resources) && candidate.resources.length > 0)
      || (Array.isArray(candidate.resourceTemplates) && candidate.resourceTemplates.length > 0);
    servers[candidate.name] = {
      name: candidate.name,
      status: projectedStatus ?? (hasInventory ? "ready" : "unavailable"),
      ...(typeof candidate.error === "string" ? { error: candidate.error } : {}),
      failureReason: typeof candidate.failureReason === "string"
        ? candidate.failureReason
        : candidate.authStatus === "notLoggedIn" ? "Authentication required" : null,
    };
  }

  return servers;
}

export function parseInitialRateLimits(value: unknown): Record<string, unknown> | null {
  const payload = unwrapResult(value);
  return isRecord(payload.rateLimits) ? payload.rateLimits : null;
}

export function mergeRateLimits(
  base: Record<string, unknown> | null,
  update: Record<string, unknown> | null,
): Record<string, unknown> | null {
  if (!base) return update;
  if (!update) return base;
  const merged = { ...base, ...update };
  for (const key of ["primary", "secondary", "credits"] as const) {
    if (isRecord(base[key]) && isRecord(update[key])) {
      merged[key] = { ...base[key], ...update[key] };
    }
  }
  return merged;
}
