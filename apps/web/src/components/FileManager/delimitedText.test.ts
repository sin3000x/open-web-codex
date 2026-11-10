import { describe, expect, it } from "vitest";
import { delimiterForPath, parseDelimitedText } from "./delimitedText";

describe("delimiterForPath", () => {
  it("maps CSV and TSV extensions and ignores other files", () => {
    expect(delimiterForPath("data/sites.CSV")).toBe(",");
    expect(delimiterForPath("data/sites.tsv")).toBe("\t");
    expect(delimiterForPath("data/sites.csv.md")).toBeNull();
  });
});

describe("parseDelimitedText", () => {
  it("parses quoted fields, doubled quotes, embedded newlines and CRLF", () => {
    const result = parseDelimitedText('﻿name,note\r\n"Doe, J","said ""hi""\nthen left"\r\nx,\r\n', ",", false);
    expect(result).toEqual({
      ok: true,
      rows: [["name", "note"], ["Doe, J", 'said "hi"\nthen left'], ["x", ""]],
      droppedPartialRow: false,
    });
  });

  it("keeps a final row without a trailing newline", () => {
    expect(parseDelimitedText("a\tb\n1\t2", "\t", false)).toEqual({
      ok: true,
      rows: [["a", "b"], ["1", "2"]],
      droppedPartialRow: false,
    });
  });

  it("drops the incomplete final row of a truncated prefix", () => {
    expect(parseDelimitedText('a,b\n1,2\n3,"unfin', ",", true)).toEqual({
      ok: true,
      rows: [["a", "b"], ["1", "2"]],
      droppedPartialRow: true,
    });
    expect(parseDelimitedText("a,b\n1,2\n3,4", ",", true)).toEqual({
      ok: true,
      rows: [["a", "b"], ["1", "2"]],
      droppedPartialRow: true,
    });
  });

  it("rejects an unterminated quote in a complete file", () => {
    expect(parseDelimitedText('a,b\n1,"open', ",", false)).toEqual({
      ok: false,
      reason: "A quoted field is not closed before the end of the file.",
    });
  });
});
