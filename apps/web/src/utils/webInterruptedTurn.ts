type InterruptibleEntry = {
  level: string;
  kind?: string;
  streaming?: boolean;
  toolStatus?: string;
};

const ACTIVE_TOOL_STATUSES = new Set(["inProgress", "running"]);

export function finalizeInterruptedTurnEntries<T extends InterruptibleEntry>(entries: T[]): T[] {
  let latestUserIndex = -1;
  for (let index = entries.length - 1; index >= 0; index -= 1) {
    if (entries[index].level === "user") {
      latestUserIndex = index;
      break;
    }
  }
  let changed = false;
  const next = entries.flatMap((entry, index) => {
    if (index <= latestUserIndex) return [entry];
    if (entry.kind === "connection") {
      changed = true;
      return [];
    }

    const toolWasRunning = entry.toolStatus != null
      && ACTIVE_TOOL_STATUSES.has(entry.toolStatus);
    if (!entry.streaming && !toolWasRunning) return [entry];

    changed = true;
    return [{
      ...entry,
      streaming: false,
      ...(toolWasRunning ? { toolStatus: "interrupted" } : {}),
    }];
  });

  return changed ? next : entries;
}
