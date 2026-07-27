import { useEffect, useState } from "react";
import SquareTerminal from "lucide-react/dist/esm/icons/square-terminal";
import ApprovalStatusIcon from "./ApprovalStatusIcon";
import {
  isApprovalOutcome,
  type ApprovalStatus,
} from "../../../utils/approvalStatus";

type CommandAction = {
  type: string;
  path: string;
};

type Props = {
  command: string;
  output?: string;
  exitCode?: number;
  status?: string;
  durationMs?: number;
  cwd?: string;
  commandActions?: CommandAction[];
  approvalStatus?: ApprovalStatus;
  approvalDetail?: string;
};

export default function CommandExecutionCard({ command, output, exitCode, status, durationMs, cwd, commandActions, approvalStatus, approvalDetail }: Props) {
  const [open, setOpen] = useState(false);
  const running = status === "inProgress" || status === "running" || (!status && exitCode == null);
  const ok = !running && exitCode === 0;

  useEffect(() => {
    if (running && output) setOpen(true);
  }, [output, running]);

  const shortCmd = command.replace(/^\/bin\/zsh -lc '/, "").replace(/'$/, "").slice(0, 120);

  return (
    <div className={`web-cmdex-card${running ? " is-running" : ok ? " is-completed" : " is-failed"}`}>
      <div className="web-cmdex-header" onClick={() => setOpen(!open)}>
        <SquareTerminal size={14} className="web-cmdex-icon" aria-hidden="true" />
        <span className={`web-cmdex-status ${running ? "web-cmdex-running" : ok ? "web-cmdex-ok" : "web-cmdex-err"}`} aria-live={running ? "polite" : undefined}>
          {running && <span className="web-cmdex-spinner" aria-hidden="true" />}
          {running ? "running" : ok ? "\u2713 OK" : `\u2717 ${exitCode == null ? status || "failed" : `exit ${exitCode}`}`}
        </span>
        <code className="web-cmdex-cmd">{shortCmd}</code>
        {isApprovalOutcome(approvalStatus) && (
          <ApprovalStatusIcon status={approvalStatus} detail={approvalDetail} />
        )}
        {durationMs != null && durationMs > 0 && <span className="web-cmdex-dur">{durationMs}ms</span>}
        <span className={`web-cmdex-chevron${open ? " web-cmdex-chevron-open" : ""}`}>&#9654;</span>
      </div>
      {open && (
        <div className="web-cmdex-body">
          {commandActions && commandActions.length > 0 && (
            <div className="web-cmdex-actions">
              {commandActions.map((a, i) => (
                <span key={i} className="web-cmdex-action">
                  {a.type}{a.path ? ': ' + a.path : ''}
                </span>
              ))}
            </div>
          )}
          {output ? (
            <pre className="web-cmdex-output"><code>{output}</code></pre>
          ) : (
            <div className="web-cmdex-no-output">(no output)</div>
          )}
          {cwd && <div className="web-cmdex-cwd">cwd: {cwd}</div>}
        </div>
      )}
    </div>
  );
}
