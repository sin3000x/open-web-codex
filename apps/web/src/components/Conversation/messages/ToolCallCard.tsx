import { useState } from "react";
import BrainCircuit from "lucide-react/dist/esm/icons/brain-circuit";
import ChevronRight from "lucide-react/dist/esm/icons/chevron-right";
import Globe from "lucide-react/dist/esm/icons/globe";
import Image from "lucide-react/dist/esm/icons/image";
import Network from "lucide-react/dist/esm/icons/network";
import Puzzle from "lucide-react/dist/esm/icons/puzzle";
import Timer from "lucide-react/dist/esm/icons/timer";
import Wrench from "lucide-react/dist/esm/icons/wrench";
import ApprovalStatusIcon from "./ApprovalStatusIcon";
import {
  isApprovalOutcome,
  type ApprovalStatus,
} from "../../../utils/approvalStatus";

type Props = {
  toolType: string;
  title: string;
  status: string;
  filePath?: string;
  detail?: string;
  output?: string;
  approvalStatus?: ApprovalStatus;
  approvalDetail?: string;
};

const STATUS_CLASS: Record<string, string> = {
  running: "web-tool-status-running",
  completed: "web-tool-status-done",
  done: "web-tool-status-done",
  error: "web-tool-status-error",
  failed: "web-tool-status-error",
  interrupted: "web-tool-status-error",
};

function ToolIcon({ type }: { type: string }) {
  const normalized = type.toLowerCase();
  if (normalized.includes("search")) return <Globe size={15} aria-hidden="true" />;
  if (normalized.includes("image")) return <Image size={15} aria-hidden="true" />;
  if (normalized.includes("collab") || normalized.includes("agent")) return <Network size={15} aria-hidden="true" />;
  if (normalized.includes("mcp") || normalized.includes("extension")) return <Puzzle size={15} aria-hidden="true" />;
  if (normalized.includes("sleep") || normalized.includes("wait")) return <Timer size={15} aria-hidden="true" />;
  if (normalized.includes("reason") || normalized.includes("plan")) return <BrainCircuit size={15} aria-hidden="true" />;
  return <Wrench size={15} aria-hidden="true" />;
}

export default function ToolCallCard({ toolType, title, status, filePath, detail, output, approvalStatus, approvalDetail }: Props) {
  const sc = STATUS_CLASS[status.toLowerCase()] ?? "web-tool-status-running";
  const [open, setOpen] = useState(false);
  const expandable = Boolean(detail || output || filePath);
  return (
    <div className="web-tool-card">
      <button
        type="button"
        className="web-tool-card-header"
        aria-disabled={!expandable}
        aria-expanded={expandable ? open : undefined}
        onClick={() => expandable && setOpen((current) => !current)}
      >
        {expandable ? <ChevronRight size={13} className={open ? "is-open" : ""} /> : <span className="web-tool-chevron-spacer" />}
        <ToolIcon type={toolType} />
        <span className="web-tool-name">{toolType}</span>
        <span>{title}</span>
        {isApprovalOutcome(approvalStatus) && (
          <ApprovalStatusIcon status={approvalStatus} detail={approvalDetail} />
        )}
        <span className={`web-tool-status ${sc}`}>
          <span className="web-tool-status-dot" />
          {status}
        </span>
      </button>
      {open && (
        <div className="web-tool-details">
          {filePath && <div className="web-tool-file">{filePath}</div>}
          {detail && <pre>{detail}</pre>}
          {output && <pre>{output}</pre>}
        </div>
      )}
    </div>
  );
}
