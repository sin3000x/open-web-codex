import { describe, expect, it } from "vitest";
import { segmentInlineVisualizations } from "./inlineVisualizations";

describe("segmentInlineVisualizations", () => {
  it("keeps Markdown and typed Artifact references in source order", () => {
    expect(segmentInlineVisualizations([
      "Before.",
      '::codex-inline-vis{artifact="map-one"}',
      "Between.",
      '::codex-inline-vis{artifact="map-two"}',
      "After.",
    ].join("\n"))).toEqual([
      { kind: "markdown", text: "Before.\n" },
      { kind: "artifact", ref: "map-one" },
      { kind: "markdown", text: "Between.\n" },
      { kind: "artifact", ref: "map-two" },
      { kind: "markdown", text: "After." },
    ]);
  });

  it("preserves the official file form without treating it as a typed Artifact", () => {
    expect(segmentInlineVisualizations(
      '::codex-inline-vis{file="chart.html"}',
    )).toEqual([{ kind: "file", file: "chart.html" }]);
  });

  it("does not parse directives inside fenced or indented code", () => {
    const markdown = [
      "```text",
      '::codex-inline-vis{artifact="map-code"}',
      "```",
      '    ::codex-inline-vis{artifact="map-indented"}',
    ].join("\n");
    expect(segmentInlineVisualizations(markdown)).toEqual([
      { kind: "markdown", text: markdown },
    ]);
  });

  it("buffers an incomplete final directive while streaming", () => {
    expect(segmentInlineVisualizations(
      'Before.\n::codex-inline-vis{artifact="map',
      true,
    )).toEqual([{ kind: "markdown", text: "Before.\n" }]);
  });

  it("returns an explicit unavailable segment for a closed invalid directive", () => {
    expect(segmentInlineVisualizations(
      '::codex-inline-vis{artifact="../unsafe"}',
    )).toEqual([{ kind: "unavailable", label: "Visualization unavailable" }]);
  });
});
