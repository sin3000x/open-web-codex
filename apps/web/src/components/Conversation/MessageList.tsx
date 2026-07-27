import type { LogEntry } from "../../WebApp";
import UserMessage from "./messages/UserMessage";
import AssistantMessage from "./messages/AssistantMessage";
import ReasoningBlock from "./messages/ReasoningBlock";
import ToolCallCard from "./messages/ToolCallCard";
import DiffBlock from "./messages/DiffBlock";
import ApprovalCard from "./messages/ApprovalCard";
import CommandExecutionCard from "./messages/CommandExecutionCard";
import SystemNotice from "./messages/SystemNotice";
import ExecutionGroup from "./messages/ExecutionGroup";

type DiffLine = {
  type: "add" | "del" | "ctx";
  text: string;
};

// Re-export types used by the event handler
export type { DiffLine };

export type MessageEntry = LogEntry & {
  kind?: "reasoning" | "tool" | "diff" | "approval" | "command_exec" | "connection";
  toolType?: string;
  toolTitle?: string;
  toolStatus?: string;
  toolDetail?: string;
  toolOutput?: string;
  filePath?: string;
  diffTitle?: string;
  diffLines?: DiffLine[];
  meta?: string;
  streaming?: boolean;
  approvalDetail?: string;
};

type Props = {
  items: MessageEntry[];
  thinking?: boolean;
  turnStartedAt?: number | null;
  onOpenFile?: (path: string) => void;
  workspaceId?: string;
  onResolveApproval?: (workspaceId: string, requestId: number | string, decision: "accept" | "decline") => void;
};

function isAssistantProcessEntry(entry: MessageEntry) {
  return entry.level === "assistant" && entry.messagePhase === "commentary";
}

function parsedApprovalTool(entry: MessageEntry) {
  return entry.approvalTool?.trim()
    || entry.text.match(/\btool\s+["“']([^"”']+)["”']/i)?.[1]?.trim()
    || "";
}

function parsedMcpIdentity(entry: MessageEntry) {
  if (entry.kind !== "tool" || !entry.toolType?.toLowerCase().includes("mcp")) {
    return null;
  }
  const parts = (entry.toolTitle ?? "")
    .split(/\s*\/\s*/)
    .map((part) => part.trim())
    .filter(Boolean);
  if (parts.length < 2) return null;
  return {
    server: parts[0],
    tool: parts[parts.length - 1],
  };
}

function isTerminalApproval(entry: MessageEntry) {
  return entry.kind === "approval"
    && entry.approvalMode !== "url"
    && entry.approvalStatus !== undefined
    && entry.approvalStatus !== "pending";
}

export function foldTerminalApprovals(items: MessageEntry[]) {
  const folded = items.map((item) => ({ ...item }));
  const consumedApprovals = new Set<number>();
  const claimedTargets = new Set<number>();

  folded.forEach((approval, approvalIndex) => {
    if (!isTerminalApproval(approval)) return;

    const approvalTool = parsedApprovalTool(approval);
    const approvalServer = approval.approvalServerName?.trim() ?? "";
    const exactCandidates: number[] = [];
    const mcpCandidates: number[] = [];

    folded.forEach((candidate, candidateIndex) => {
      if (candidateIndex === approvalIndex || claimedTargets.has(candidateIndex)) return;
      if (
        approval.approvalId
        && candidate.approvalId === approval.approvalId
        && (candidate.kind === "tool" || candidate.kind === "command_exec")
      ) {
        exactCandidates.push(candidateIndex);
        return;
      }
      if (!approvalTool) return;
      const identity = parsedMcpIdentity(candidate);
      if (
        identity
        && identity.tool === approvalTool
        && (!approvalServer || identity.server === approvalServer)
      ) {
        mcpCandidates.push(candidateIndex);
      }
    });

    const matchingCandidates = exactCandidates.length > 0 ? exactCandidates : mcpCandidates;
    let targetIndex = matchingCandidates
      .sort((left, right) => {
        const distance = Math.abs(left - approvalIndex) - Math.abs(right - approvalIndex);
        if (distance !== 0) return distance;
        return left < approvalIndex ? -1 : 1;
      })[0] ?? -1;

    if (targetIndex < 0 && !approvalTool) {
      for (let cursor = approvalIndex - 1; cursor >= 0; cursor -= 1) {
        const candidate = folded[cursor];
        if (
          !claimedTargets.has(cursor)
          && (candidate.kind === "tool" || candidate.kind === "command_exec")
        ) {
          targetIndex = cursor;
          break;
        }
      }
    }

    if (targetIndex < 0) return;
    folded[targetIndex] = {
      ...folded[targetIndex],
      approvalStatus: approval.approvalStatus,
      approvalDetail: approval.text,
    };
    claimedTargets.add(targetIndex);
    consumedApprovals.add(approvalIndex);
  });

  return folded.filter((_, index) => !consumedApprovals.has(index));
}

export default function MessageList({ items, thinking = false, turnStartedAt, onOpenFile, workspaceId, onResolveApproval }: Props) {
  if (items.length === 0) {
    return (
      <div className="web-empty">
        <div className="web-empty-inner">
          <div className="web-brand-sub">Ready when you are</div>
          <h2>Start a Codex task from the browser</h2>
          <p>
            Select a workspace, start a thread, and describe the task.
            Events and messages will appear here in real time.
          </p>
        </div>
      </div>
    );
  }

  const renderEntry = (entry: MessageEntry) => {
        if (entry.kind === "connection") {
          return (
            <div className="web-connection-notice" role="status" key={entry.id}>
              <span className="web-thinking-spinner" aria-hidden="true" />
              <span>{entry.text}</span>
            </div>
          );
        }

        if (entry.kind === "approval") {
          return (
            <ApprovalCard
              key={entry.id}
              command={entry.text}
              workspaceId={workspaceId}
              requestId={entry.approvalRequestId}
              status={entry.approvalStatus}
              mode={entry.approvalMode}
              url={entry.approvalUrl}
              serverName={entry.approvalServerName}
              onResolve={onResolveApproval}
            />
          );
        }

        if (entry.kind === "command_exec") {
          return (
            <CommandExecutionCard
              key={entry.id}
              command={entry.text}
              output={entry.cmdOutput ?? void 0}
              exitCode={entry.cmdExitCode}
              status={entry.toolStatus}
              durationMs={entry.cmdDurationMs}
              cwd={entry.cmdCwd}
              commandActions={entry.cmdActions}
              approvalStatus={entry.approvalStatus === "pending" ? undefined : entry.approvalStatus}
              approvalDetail={entry.approvalDetail}
            />
          );
        }

        if (entry.kind === "reasoning") {
          return (
            <ReasoningBlock
              key={entry.id}
              text={entry.text}
              summary={entry.reasoningSummary}
              meta={entry.meta}
              streaming={entry.streaming}
            />
          );
        }
        if (entry.kind === "tool") {
          return (
            <ToolCallCard
              key={entry.id}
              toolType={entry.toolType ?? ""}
              title={entry.toolTitle ?? ""}
              status={entry.toolStatus ?? ""}
              filePath={entry.filePath}
              detail={entry.toolDetail}
              output={entry.toolOutput}
              approvalStatus={entry.approvalStatus === "pending" ? undefined : entry.approvalStatus}
              approvalDetail={entry.approvalDetail}
            />
          );
        }
        if (entry.kind === "diff") {
          return (
            <DiffBlock
              key={entry.id}
              title={entry.diffTitle ?? ""}
              lines={entry.diffLines ?? []}
              updating={entry.streaming}
            />
          );
        }
        switch (entry.level) {
          case "user":
            return <UserMessage key={entry.id} text={entry.text} />;
          case "assistant":
            return (
              <AssistantMessage
                key={entry.id}
                text={entry.text}
                streaming={entry.streaming}
                onOpenFile={onOpenFile}
                variant={isAssistantProcessEntry(entry) ? "commentary" : "reply"}
                inlineArtifacts={entry.inlineArtifacts}
              />
            );
          case "system":
            return <SystemNotice key={entry.id} text={entry.text} variant="default" />;
          case "error":
            return <SystemNotice key={entry.id} text={entry.text} variant="error" />;
          default:
            return <SystemNotice key={entry.id} text={entry.text} variant="neutral" />;
        }
  };

  const rendered: React.ReactNode[] = [];
  const isLiveEntry = (entry: MessageEntry) =>
    entry.streaming
    || entry.toolStatus === "inProgress"
    || entry.toolStatus === "running";
  const isAssistantReply = (entry: MessageEntry) =>
    entry.level === "assistant" && !isAssistantProcessEntry(entry);
  for (let index = 0; index < items.length;) {
    const entry = items[index];
    if (entry.level !== "user") {
      rendered.push(renderEntry(entry));
      index += 1;
      continue;
    }
    rendered.push(renderEntry(entry));
    let end = index + 1;
    while (end < items.length && items[end].level !== "user") end += 1;
    const turnItems = items.slice(index + 1, end);
    const hasLiveItem = turnItems.some(isLiveEntry);
    const pendingApproval = turnItems.find((item) => {
      if (item.kind !== "approval") return false;
      if (item.approvalStatus === "pending") return true;
      const mapsCredentialWasDelivered = item.approvalMode === "url"
        && (
          item.approvalServerName === "map_utils"
          || item.approvalServerName === "workspace_maps"
        )
        && (
          /maps provider and api key/i.test(item.text)
          || /(?:google maps|mapbox(?: maps)?)\s+(?:api key|access token)/i.test(item.text)
        );
      return item.approvalMode === "url"
        && item.approvalStatus === "accepted"
        && !mapsCredentialWasDelivered;
    });
    const activityLabel = pendingApproval?.approvalMode === "url"
      ? "Waiting for API key…"
      : pendingApproval
        ? "Waiting for approval…"
        : "Working…";
    const isActiveTurn = end === items.length && (thinking || hasLiveItem);

    let executionSegment: MessageEntry[] = [];
    let executionSegmentIndex = 0;
    const flushExecutionSegment = (active: boolean) => {
      if (executionSegment.length === 0 && !active) return;
      const displaySegment = foldTerminalApprovals(executionSegment);
      let activeIndex = -1;
      if (active) {
        for (let cursor = displaySegment.length - 1; cursor >= 0; cursor -= 1) {
          if (isLiveEntry(displaySegment[cursor])) {
            activeIndex = cursor;
            break;
          }
        }
        if (activeIndex < 0 && displaySegment.length > 0) {
          activeIndex = displaySegment.length - 1;
        }
      }
      const timelineItems = displaySegment.filter((_, cursor) => cursor !== activeIndex);
      const activeItem = activeIndex >= 0 ? displaySegment[activeIndex] : null;
      rendered.push(
        <ExecutionGroup
          key={`execution-${entry.id}-${executionSegmentIndex}`}
          items={displaySegment}
          active={active}
          startedAt={turnStartedAt}
          timelineItemCount={timelineItems.length}
          activeItem={activeItem ? renderEntry(activeItem) : null}
          activityLabel={activityLabel}
        >
          {timelineItems.map(renderEntry)}
        </ExecutionGroup>,
      );
      executionSegment = [];
      executionSegmentIndex += 1;
    };

    for (const item of turnItems) {
      if (isAssistantReply(item)) {
        flushExecutionSegment(false);
        rendered.push(renderEntry(item));
        continue;
      }
      executionSegment.push(item);
    }

    if (executionSegment.length > 0) {
      flushExecutionSegment(isActiveTurn);
    } else {
      const lastItem = turnItems[turnItems.length - 1];
      if (isActiveTurn && (!lastItem || !isAssistantReply(lastItem))) {
        flushExecutionSegment(true);
      }
    }
    index = end;
  }
  return <>{rendered}</>;
}
