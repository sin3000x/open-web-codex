import { describe, expect, it } from "vitest";
import { isWebAppServerRecoveryEvent, parseCodexStderr, parseWebAppServerError } from "./webAppServerError";

describe("isWebAppServerRecoveryEvent", () => {
  it.each([
    "turn/started",
    "thread/status/changed",
    "item/started",
    "item/agentMessage/delta",
    "item/commandExecution/outputDelta",
    "turn/diff/updated",
    "serverRequest/resolved",
    "turn/completed",
  ])("treats %s as proof that the response stream recovered", (method) => {
    expect(isWebAppServerRecoveryEvent(method)).toBe(true);
  });

  it.each([
    "error",
    "thread/tokenUsage/updated",
    "account/rateLimits/updated",
  ])("does not clear the reconnect notice for %s", (method) => {
    expect(isWebAppServerRecoveryEvent(method)).toBe(false);
  });
});

describe("parseWebAppServerError", () => {
  it("turns response stream disconnects into a reconnect status", () => {
    expect(parseWebAppServerError({
      willRetry: true,
      error: {
        additionalDetails: "stream disconnected before completion: failed to send websocket request: Connection closed normally",
        codexErrorInfo: { responseStreamDisconnected: { httpStatusCode: null } },
        message: "Reconnecting... 1/5",
      },
      threadId: "019f5b88-612b-7cc3-8309-45fc1a929d15",
    })).toEqual({ recoverable: true, text: "Connection interrupted. Reconnecting (1/5)…" });
  });

  it("keeps a concise message for terminal errors", () => {
    expect(parseWebAppServerError({ error: { message: "Authentication failed" } })).toEqual({
      recoverable: false,
      text: "Authentication failed",
    });
  });

  it.each([400, 401, 422, 503])("does not hide terminal HTTP %s as reconnecting", (status) => {
    expect(parseWebAppServerError({
      willRetry: false,
      error: {
        message: `Provider request failed (HTTP ${status}).`,
        recoverable: true,
        codexErrorInfo: { responseStreamDisconnected: { httpStatusCode: status } },
      },
    })).toEqual({ recoverable: false, text: `Provider request failed (HTTP ${status}).` });
  });

  it("keeps turn/completed errors terminal even when their text mentions a disconnect", () => {
    expect(parseWebAppServerError({ error: { message: "stream disconnected before completion" } }))
      .toEqual({ recoverable: false, text: "stream disconnected before completion" });
  });
});

describe("parseCodexStderr", () => {
  it("turns structured sampling retry warnings into a reconnect status", () => {
    expect(parseCodexStderr({
      message: JSON.stringify({
        level: "WARN",
        fields: { message: "stream disconnected - retrying sampling request (1/5 in 217ms)..." },
        target: "codex_core::responses_retry",
      }),
    })).toEqual({ recoverable: true, text: "Connection interrupted. Reconnecting (1/5)…" });
  });

  it("ignores unrelated stderr records", () => {
    expect(parseCodexStderr({ message: JSON.stringify({ level: "INFO", fields: { message: "ready" } }) }))
      .toBeNull();
  });
});
