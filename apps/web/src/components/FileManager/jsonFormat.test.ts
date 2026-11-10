import { describe, expect, it } from "vitest";
import { formatJsonPreview } from "./jsonFormat";

describe("formatJsonPreview", () => {
  it("pretty-prints valid JSON with two-space indentation", () => {
    expect(formatJsonPreview('{"a":[1,{"b":null}]}', false)).toEqual({
      ok: true,
      text: '{\n  "a": [\n    1,\n    {\n      "b": null\n    }\n  ]\n}',
    });
  });

  it("reports invalid JSON instead of formatting it", () => {
    const result = formatJsonPreview('{"a":', false);
    expect(result.ok).toBe(false);
    expect(!result.ok && result.reason).toMatch(/^The file is not valid JSON \(.+\), so it is shown unformatted\.$/);
  });

  it("does not parse a truncated prefix", () => {
    expect(formatJsonPreview('{"a":1}', true)).toEqual({
      ok: false,
      reason: "The file is larger than the preview limit, so it is shown unformatted.",
    });
  });
});
