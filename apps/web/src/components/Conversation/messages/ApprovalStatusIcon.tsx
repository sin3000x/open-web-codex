import { useCallback, useId, useState } from "react";
import { createPortal } from "react-dom";
import CircleCheck from "lucide-react/dist/esm/icons/circle-check";
import CircleX from "lucide-react/dist/esm/icons/circle-x";
import MessageSquareMore from "lucide-react/dist/esm/icons/message-square-more";
import type { ApprovalOutcome } from "../../../utils/approvalStatus";

type Props = {
  status: ApprovalOutcome;
  detail?: string;
};

type TooltipPosition = {
  left: number;
  top: number;
  placement: "top" | "bottom";
};

const metadata: Record<ApprovalOutcome, {
  label: string;
  fallbackDetail: string;
  Icon: typeof CircleCheck;
}> = {
  accepted: {
    label: "Approved",
    fallbackDetail: "The user approved this action.",
    Icon: CircleCheck,
  },
  declined: {
    label: "Denied",
    fallbackDetail: "The user denied this action.",
    Icon: CircleX,
  },
  answered: {
    label: "Other response",
    fallbackDetail: "The user provided additional information.",
    Icon: MessageSquareMore,
  },
};

export default function ApprovalStatusIcon({ status, detail }: Props) {
  const tooltipId = useId();
  const [tooltipPosition, setTooltipPosition] = useState<TooltipPosition | null>(null);
  const { label, fallbackDetail, Icon } = metadata[status];
  const description = detail?.trim() || fallbackDetail;
  const accessibleLabel = `${label}: ${description}`;

  const showTooltip = useCallback((target: HTMLElement) => {
    const rect = target.getBoundingClientRect();
    const placement = rect.top >= 96 ? "top" : "bottom";
    setTooltipPosition({
      left: Math.min(
        Math.max(rect.left + rect.width / 2, 176),
        Math.max(176, window.innerWidth - 176),
      ),
      top: placement === "top" ? rect.top - 10 : rect.bottom + 10,
      placement,
    });
  }, []);

  return (
    <>
      <span
        className={`web-approval-status-icon is-${status}`}
        role="img"
        tabIndex={0}
        aria-label={accessibleLabel}
        aria-describedby={tooltipPosition ? tooltipId : undefined}
        onMouseEnter={(event) => showTooltip(event.currentTarget)}
        onMouseLeave={() => setTooltipPosition(null)}
        onFocus={(event) => showTooltip(event.currentTarget)}
        onBlur={() => setTooltipPosition(null)}
      >
        <Icon size={16} aria-hidden="true" />
      </span>
      {tooltipPosition && createPortal(
        <span
          id={tooltipId}
          className="web-approval-status-tooltip"
          data-placement={tooltipPosition.placement}
          role="tooltip"
          style={{
            left: tooltipPosition.left,
            top: tooltipPosition.top,
          }}
        >
          <strong>{label}</strong>
          <span>{description}</span>
        </span>,
        document.querySelector(".web-app-shell") ?? document.body,
      )}
    </>
  );
}
