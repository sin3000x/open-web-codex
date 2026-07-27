import { useEffect, useMemo, useState, type ReactNode } from "react";
import Brain from "lucide-react/dist/esm/icons/brain";
import ChevronRight from "lucide-react/dist/esm/icons/chevron-right";
import type { MessageEntry } from "../MessageList";

type Props = {
  items: MessageEntry[];
  active: boolean;
  startedAt?: number | null;
  activeItem?: ReactNode;
  timelineItemCount?: number;
  activityLabel?: string;
  children: ReactNode;
};

export default function ExecutionGroup({
  items,
  active,
  startedAt,
  activeItem,
  timelineItemCount = 0,
  activityLabel = "Working…",
  children,
}: Props) {
  const [manuallyOpen, setManuallyOpen] = useState(false);
  const [fallbackStartedAt] = useState(Date.now);
  const [elapsed, setElapsed] = useState(0);
  // Live activity is always visible. Once it completes, derive the collapsed
  // state during that same render so historical details never remain expanded
  // for one paint while an effect catches up.
  const open = active || manuallyOpen;
  const toolCount = useMemo(() => items.filter((item) =>
    item.kind === "tool"
    || item.kind === "command_exec"
    || item.kind === "diff").length, [items]);
  const messageCount = useMemo(() => items.filter((item) => item.level === "assistant"
    || (item.kind === "reasoning"
      && (!/^(reasoning completed|reasoning in progress|reasoning)$/i.test(item.text.trim())
        || Boolean(item.reasoningSummary?.trim())))).length, [items]);

  useEffect(() => {
    if (!active) return;
    const effectiveStartedAt = startedAt ?? fallbackStartedAt;
    const update = () => setElapsed(Math.max(0, Math.floor((Date.now() - effectiveStartedAt) / 1000)));
    update();
    const timer = window.setInterval(update, 1000);
    return () => window.clearInterval(timer);
  }, [active, fallbackStartedAt, startedAt]);

  const elapsedLabel = `${Math.floor(elapsed / 60)}:${String(elapsed % 60).padStart(2, "0")}`;
  return (
    <section className={`web-execution-group${active ? " is-active" : ""}`}>
      {(!active || timelineItemCount > 0) && (
        <button
          type="button"
          className="web-execution-summary"
          onClick={() => { if (!active) setManuallyOpen((value) => !value); }}
          aria-expanded={open}
          aria-disabled={active}
        >
          <ChevronRight size={12} className={open ? "is-open" : ""} />
          <span>{toolCount} tool {toolCount === 1 ? "call" : "calls"}, {messageCount} {messageCount === 1 ? "message" : "messages"}</span>
        </button>
      )}
      {((active && timelineItemCount > 0) || (!active && open)) && <div className="web-execution-timeline">{children}</div>}
      {activeItem ? <div className="web-execution-current">{activeItem}</div> : null}
      {active && (
        <div className="web-execution-working" role="status">
          <span className="web-thinking-spinner" aria-hidden="true" />
          <span className="web-execution-elapsed">{elapsedLabel}</span>
          <Brain size={14} aria-hidden="true" />
          <span>{activityLabel}</span>
        </div>
      )}
    </section>
  );
}
