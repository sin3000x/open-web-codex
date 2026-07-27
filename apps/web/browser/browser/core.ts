type BridgeHandler = (payload: Record<string, unknown>) => Promise<unknown>;

const handlers = new Map<string, BridgeHandler>();
const workspaceResourceRoots = new Map<string, string>();

const UUID_PATTERN = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;

function normalizeWorkspaceRoot(root: string): string {
  return root.trim().replace(/\\/g, "/").replace(/\/+$/, "");
}

function normalizeRelativeResourcePath(path: string): string {
  const normalized = path.trim().replace(/\\/g, "/");
  const segments = normalized.split("/");
  if (
    !normalized ||
    normalized.startsWith("/") ||
    normalized.includes("\0") ||
    segments.some((segment) => !segment || segment === "." || segment === "..")
  ) {
    throw new Error("Browser workspace resource path is unsafe");
  }
  return normalized;
}

function validateRunId(runId: string): string {
  const normalized = runId.trim();
  if (!UUID_PATTERN.test(normalized)) {
    throw new Error("Browser workspace resource Run id is invalid");
  }
  return normalized;
}

function workspaceAssetUrl(runId: string, relativePath: string): string {
  const query = new URLSearchParams({ path: normalizeRelativeResourcePath(relativePath) });
  return `/api/runs/${encodeURIComponent(validateRunId(runId))}/workspace/assets?${query.toString()}`;
}

function registeredWorkspaceResource(path: string): string | null {
  const normalized = path.trim().replace(/\\/g, "/");
  const roots = Array.from(workspaceResourceRoots.entries())
    .sort(([left], [right]) => right.length - left.length);
  for (const [root, runId] of roots) {
    if (!normalized.startsWith(`${root}/`)) continue;
    return workspaceAssetUrl(runId, normalized.slice(root.length + 1));
  }
  return null;
}

function encodedWorkspaceResource(path: string): string | null {
  const matched = /^owc-run:\/\/([^/]+)\/(.+)$/i.exec(path.trim());
  if (!matched) return null;
  let relativePath: string;
  try {
    relativePath = decodeURIComponent(matched[2]);
  } catch {
    throw new Error("Browser workspace resource reference is invalid");
  }
  return workspaceAssetUrl(matched[1], relativePath);
}

export function isTauri(): boolean {
  return false;
}

export function convertFileSrc(path: string): string {
  const registered = registeredWorkspaceResource(path);
  if (registered) return registered;
  const encoded = encodedWorkspaceResource(path);
  if (encoded) return encoded;
  if (/^(?:data:|blob:|https?:\/\/)/i.test(path)) {
    return path;
  }
  throw new Error("Browser workspace resource is not registered");
}

export function registerWorkspaceResourceRoot(root: string, runId: string): () => void {
  const normalizedRoot = normalizeWorkspaceRoot(root);
  if (!normalizedRoot) {
    throw new Error("Browser workspace resource root is required");
  }
  const normalizedRunId = validateRunId(runId);
  workspaceResourceRoots.set(normalizedRoot, normalizedRunId);
  return () => {
    if (workspaceResourceRoots.get(normalizedRoot) === normalizedRunId) {
      workspaceResourceRoots.delete(normalizedRoot);
    }
  };
}

export function clearWorkspaceResourceRoots(): void {
  workspaceResourceRoots.clear();
}

export function workspaceResourceRef(runId: string, relativePath: string): string {
  const normalizedRunId = validateRunId(runId);
  const normalizedPath = normalizeRelativeResourcePath(relativePath);
  return `owc-run://${normalizedRunId}/${encodeURIComponent(normalizedPath)}`;
}

export function registerBrowserCommand(
  command: string,
  handler: BridgeHandler,
): () => void {
  handlers.set(command, handler);
  return () => {
    if (handlers.get(command) === handler) {
      handlers.delete(command);
    }
  };
}

export async function invoke<T>(
  command: string,
  payload: Record<string, unknown> = {},
): Promise<T> {
  const handler = handlers.get(command);
  if (!handler) {
    throw new Error(`Browser command is not connected: ${command}`);
  }
  return await handler(payload) as T;
}
