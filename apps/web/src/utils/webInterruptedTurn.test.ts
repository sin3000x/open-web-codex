import { describe, expect, it } from "vitest";
import { finalizeInterruptedTurnEntries } from "./webInterruptedTurn";

describe("finalizeInterruptedTurnEntries", () => {
  it("stops only the latest turn's live entries", () => {
    const previous = [
      { id: "user-old", level: "user", text: "old" },
      { id: "tool-old", level: "info", text: "old tool", toolStatus: "running", streaming: true },
      { id: "user-current", level: "user", text: "current" },
      { id: "reasoning-current", level: "system", text: "thinking", streaming: true },
      { id: "tool-current", level: "info", text: "tool", toolStatus: "inProgress", streaming: true },
      { id: "connection-current", level: "info", text: "reconnecting", kind: "connection", streaming: true },
    ];

    expect(finalizeInterruptedTurnEntries(previous)).toEqual([
      previous[0],
      previous[1],
      previous[2],
      { ...previous[3], streaming: false },
      { ...previous[4], streaming: false, toolStatus: "interrupted" },
    ]);
  });

  it("preserves the original array when no live state needs clearing", () => {
    const entries = [
      { id: "user-current", level: "user", text: "current" },
      { id: "tool-current", level: "info", text: "tool", toolStatus: "completed" },
    ];

    expect(finalizeInterruptedTurnEntries(entries)).toBe(entries);
  });
});
