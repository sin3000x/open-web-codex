export type WebAppServerError = {
  recoverable: boolean;
  text: string;
};

function reconnectStatus(message: string, details = ""): WebAppServerError | null {
  const combined = `${message} ${details}`;
  const progress = combined.match(/(?:Reconnecting\.\.\.|retrying sampling request)\s*\(?(\d+)\s*\/\s*(\d+)/i);
  const disconnected = /stream disconnected|websocket request/i.test(combined);
  if (!progress && !disconnected) return null;
  const suffix = progress ? ` (${progress[1]}/${progress[2]})` : "";
  return { recoverable: true, text: `Connection interrupted. Reconnecting${suffix}…` };
}

export function isWebAppServerRecoveryEvent(method: string): boolean {
  return method === "turn/started"
    || method === "turn/completed"
    || method === "thread/status/changed"
    || method === "turn/plan/updated"
    || method === "turn/diff/updated"
    || method === "serverRequest/resolved"
    || method.startsWith("item/");
}

export function parseWebAppServerError(params: Record<string, unknown>): WebAppServerError {
  const error = params.error && typeof params.error === "object"
    ? params.error as Record<string, unknown>
    : params;
  const message = typeof error.message === "string" ? error.message : "";
  const details = typeof error.additionalDetails === "string" ? error.additionalDetails : "";
  // A retryable error is not proof that Runtime will retry this Turn. In
  // particular, terminal HTTP errors may also use responseStreamDisconnected.
  if (params.willRetry === true) {
    return reconnectStatus(message, details)
      ?? { recoverable: true, text: message || "Connection interrupted. Reconnecting…" };
  }

  return {
    recoverable: false,
    text: message || details || "The runtime reported an unknown error.",
  };
}

export function parseCodexStderr(params: Record<string, unknown>): WebAppServerError | null {
  const raw = typeof params.message === "string" ? params.message.trim() : "";
  if (!raw) return null;
  let message = raw;
  try {
    const record = JSON.parse(raw) as Record<string, unknown>;
    const fields = record.fields && typeof record.fields === "object"
      ? record.fields as Record<string, unknown>
      : null;
    if (typeof fields?.message === "string") message = fields.message;
  } catch {
    // Plain-text stderr from older runtimes is still supported.
  }
  return reconnectStatus(message);
}
