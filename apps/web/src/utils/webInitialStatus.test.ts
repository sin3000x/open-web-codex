import { describe, expect, it } from "vitest";
import { mergeRateLimits, parseInitialMcpServers, parseInitialRateLimits } from "./webInitialStatus";

describe("initial workspace status parsing", () => {
  it("maps the MCP inventory snapshot to sidebar server states", () => {
    expect(parseInitialMcpServers({
      result: {
        data: [
          { name: "docs", serverInfo: { name: "docs" }, tools: {}, resources: [], resourceTemplates: [], authStatus: "unsupported" },
          { name: "github", serverInfo: null, tools: {}, resources: [], resourceTemplates: [], authStatus: "notLoggedIn" },
        ],
      },
    })).toEqual({
      docs: { name: "docs", status: "ready", failureReason: null },
      github: { name: "github", status: "unavailable", failureReason: "Authentication required" },
    });
  });

  it("uses the durable MCP startup-status projection without requiring inventory", () => {
    expect(parseInitialMcpServers({
      data: [
        { name: "map_utils", status: "ready", error: null, failureReason: null },
        {
          name: "broken",
          status: "failed",
          error: "handshake failed",
          failureReason: "initializeFailed",
        },
      ],
    })).toEqual({
      map_utils: {
        name: "map_utils",
        status: "ready",
        failureReason: null,
      },
      broken: {
        name: "broken",
        status: "failed",
        error: "handshake failed",
        failureReason: "initializeFailed",
      },
    });
  });

  it("extracts the canonical rate-limit snapshot", () => {
    const rateLimits = {
      primary: { usedPercent: 12 },
      secondary: { usedPercent: 34 },
    };
    expect(parseInitialRateLimits({ result: { rateLimits } })).toEqual(rateLimits);
  });

  it("merges sparse usage events without discarding snapshot fields", () => {
    expect(mergeRateLimits(
      { primary: { usedPercent: 12, resetsAt: 100 }, planType: "plus" },
      { primary: { usedPercent: 18 } },
    )).toEqual({
      primary: { usedPercent: 18, resetsAt: 100 },
      planType: "plus",
    });
  });
});
