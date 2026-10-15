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
import ReplyCard from "./messages/ReplyCard";
import AgentWaitCard from "./messages/AgentWaitCard";
import type { InlineVisualizationArtifact } from "../../utils/replyCards";
import type { ArtifactSummary } from "../../../browser/types";
import type { AgentWaitHistoryUpdate } from "../../utils/agentWaitUpdates";

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
  suppressInlineArtifacts?: boolean;
  hiddenInlineArtifactRefs?: string[];
};

type Props = {
  items: MessageEntry[];
  thinking?: boolean;
  turnStartedAt?: number | null;
  onOpenFile?: (path: string) => void;
  workspaceId?: string;
  onResolveApproval?: (workspaceId: string, requestId: number | string, decision: "accept" | "decline") => void;
  inlineVisualizationThreadId?: string | null;
  finalArtifacts?: ArtifactSummary[];
  agentWaitUpdates?: AgentWaitHistoryUpdate[];
  onShowAgentActivity?: () => void;
  onReviseMapCard?: (cardId: string, title: string) => void;
};

function isAssistantProcessEntry(entry: MessageEntry) {
  return entry.level === "assistant" && entry.messagePhase === "commentary";
}

function typedArtifactDeliveries(entry: MessageEntry) {
  return entry.level === "assistant" && Array.isArray(entry.inlineArtifacts)
    ? entry.inlineArtifacts
    : [];
}

function ArtifactDeliveries({
  artifacts,
  entryId,
  onReviseMapCard,
}: {
  artifacts: InlineVisualizationArtifact[];
  entryId: string;
  onReviseMapCard?: (cardId: string, title: string) => void;
}) {
  return (
    <div className="web-msg-assistant has-inline-visualization">
      <div className="web-msg-assistant-body">
        {artifacts.map((artifact, index) => (
          <ReplyCard
            key={`${entryId}-${artifact.ref}-${index}`}
            card={artifact.card}
            onRevise={onReviseMapCard}
          />
        ))}
      </div>
    </div>
  );
}

function FinalArtifactLinks({ artifacts }: { artifacts: ArtifactSummary[] }) {
  if (artifacts.length === 0) return null;
  return (
    <section className="web-final-artifact-links" aria-label="生成的文件">
      {artifacts.map((artifact) => (
        <p key={artifact.id}>
          <span>{artifact.mime_type === "text/markdown" ? "正式简报" : "生成文件"}：</span>
          <a href={artifact.download_url ?? undefined} download>
            {artifact.mime_type === "text/markdown" ? "下载 Markdown 文件" : "下载文件"}
          </a>
        </p>
      ))}
    </section>
  );
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
    const approvalServer = "";
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

export default function MessageList({ items, thinking = false, turnStartedAt, onOpenFile, workspaceId, onResolveApproval, inlineVisualizationThreadId, finalArtifacts = [], agentWaitUpdates = [], onShowAgentActivity, onReviseMapCard }: Props) {
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
              credentialKind={entry.approvalCredentialKind}
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
          if (entry.agentAction === "wait") {
            return (
              <AgentWaitCard
                key={entry.id}
                status={entry.toolStatus ?? ""}
                onShowAgentActivity={onShowAgentActivity}
              />
            );
          }
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
                showInlineArtifacts={!entry.suppressInlineArtifacts}
                hiddenInlineArtifactRefs={entry.hiddenInlineArtifactRefs}
                inlineVisualizationThreadId={inlineVisualizationThreadId}
                onReviseMapCard={onReviseMapCard}
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
  const finalArtifactsByProducerItem = new Map<string, ArtifactSummary[]>();
  for (const artifact of finalArtifacts) {
    if (
      artifact.state !== "ready"
      || !artifact.download_url
      || artifact.producer_thread_id !== inlineVisualizationThreadId
    ) {
      continue;
    }
    const existing = finalArtifactsByProducerItem.get(artifact.producer_item_id) ?? [];
    existing.push(artifact);
    finalArtifactsByProducerItem.set(artifact.producer_item_id, existing);
  }
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
      const entryArtifacts = finalArtifactsByProducerItem.get(entry.id) ?? [];
      if (entryArtifacts.length > 0) {
        rendered.push(
          <FinalArtifactLinks key={`final-artifacts-${entry.id}`} artifacts={entryArtifacts} />,
        );
      }
      index += 1;
      continue;
    }
    rendered.push(renderEntry(entry));
    let end = index + 1;
    while (end < items.length && items[end].level !== "user") end += 1;
    const turnItems = items.slice(index + 1, end);
    const surfacedArtifactRefs = new Set<string>();
    const hasLiveItem = turnItems.some(isLiveEntry);
    const pendingApproval = turnItems.find((item) => {
      if (item.kind !== "approval") return false;
      if (item.approvalStatus === "pending") return true;
      const mapsCredentialWasDelivered = item.approvalMode === "url"
        && item.approvalCredentialKind === "maps";
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
    const finalReplyIndex = turnItems.length > 0
      && isAssistantReply(turnItems[turnItems.length - 1])
      ? turnItems.length - 1
      : -1;
    const completedTurnDurationMs = entry.turnDurationMs
      ?? turnItems.find((item) => item.turnDurationMs !== undefined)?.turnDurationMs;

    // One Runtime Turn has one execution summary. This keeps its counts and
    // official duration on the same boundary even when the Turn emits several
    // intermediate Agent messages.
    const executionSegment: MessageEntry[] = [];
    const errors: MessageEntry[] = [];
    const artifactDeliveries: React.ReactNode[] = [];
    const finalArtifactDeliveries: ArtifactSummary[] = [];
    let visibleReply: MessageEntry | null = null;
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
      const activeAgentWait = active && activeItem?.agentAction === "wait"
        ? activeItem
        : null;
      rendered.push(
        <ExecutionGroup
          key={`execution-${entry.id}`}
          items={displaySegment}
          active={active}
          startedAt={turnStartedAt}
          durationMs={active ? undefined : completedTurnDurationMs}
          timelineItemCount={timelineItems.length}
          activeItem={activeItem && !activeAgentWait ? renderEntry(activeItem) : null}
          agentWaitStatus={activeAgentWait?.toolStatus}
          agentWaitUpdates={agentWaitUpdates}
          onShowAgentActivity={onShowAgentActivity}
          activityLabel={activityLabel}
        >
          {timelineItems.map(renderEntry)}
        </ExecutionGroup>,
      );
    };

    for (const [turnItemIndex, item] of turnItems.entries()) {
      // Failures require attention even when the execution timeline is folded.
      if (item.level === "error") {
        errors.push(item);
        continue;
      }
      finalArtifactDeliveries.push(...(finalArtifactsByProducerItem.get(item.id) ?? []));
      const typedArtifacts = typedArtifactDeliveries(item);
      const hiddenInlineArtifactRefs: string[] = [];
      const newlySurfacedArtifacts = typedArtifacts.filter((artifact) => {
        if (surfacedArtifactRefs.has(artifact.ref)) {
          hiddenInlineArtifactRefs.push(artifact.ref);
          return false;
        }
        surfacedArtifactRefs.add(artifact.ref);
        return true;
      });
      if (turnItemIndex === finalReplyIndex) {
        visibleReply = hiddenInlineArtifactRefs.length
          ? { ...item, hiddenInlineArtifactRefs }
          : item;
        continue;
      }
      const executionItem = isAssistantReply(item)
        ? { ...item, messagePhase: "commentary" as const }
        : item;
      executionSegment.push(typedArtifacts.length === 0
        ? executionItem
        : { ...executionItem, suppressInlineArtifacts: true });
      if (newlySurfacedArtifacts.length === 0) continue;
      artifactDeliveries.push(
        <ArtifactDeliveries
          key={`artifact-deliveries-${item.id}`}
          artifacts={newlySurfacedArtifacts}
          entryId={item.id}
          onReviseMapCard={onReviseMapCard}
        />,
      );
    }

    if (executionSegment.length > 0) {
      flushExecutionSegment(isActiveTurn && visibleReply === null);
    } else if (isActiveTurn && visibleReply === null) {
      flushExecutionSegment(true);
    }
    rendered.push(...artifactDeliveries);
    if (visibleReply) rendered.push(renderEntry(visibleReply));
    rendered.push(...errors.map(renderEntry));
    if (finalArtifactDeliveries.length > 0) {
      rendered.push(
        <FinalArtifactLinks
          key={`final-artifacts-${entry.id}`}
          artifacts={finalArtifactDeliveries}
        />,
      );
    }
    index = end;
  }
  return <>{rendered}</>;
}
