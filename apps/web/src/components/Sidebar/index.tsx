import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import Settings from "lucide-react/dist/esm/icons/settings";
import Moon from "lucide-react/dist/esm/icons/moon";
import Sun from "lucide-react/dist/esm/icons/sun";
import X from "lucide-react/dist/esm/icons/x";
import type { WorkspaceInfo } from "../../types";
import type { CopilotProfileStatus } from "../../../browser/types";
import Brand from "./Brand";
import CopilotManager from "./CopilotManager";
import Workspaces, { type CopilotOption, type StartTaskRequest } from "./Workspaces";
import McpStatus from "./McpStatus";
import RateLimitCard from "./RateLimitCard";
import MapsSettings from "./MapsSettings";

type ThreadInfo = {
  id: string;
  label: string;
  updatedAt: number;
  turnCount?: number;
  status?: string;
  creationStatus?: "creating" | "failed";
};

const NOOP_ASYNC = async () => undefined;

type Props = {
  gatewayState: "checking" | "online" | "offline";
  gatewayVersion: string | null;
  workspaces: WorkspaceInfo[];
  activeWorkspaceId: string | null;
  onSelectWorkspace: (id: string) => void;
  onCreateWorkspace: (name: string) => void;
  onLoadWorkspaces: () => void;
  onConnectWorkspace: (id: string) => void;
  threadsByWorkspace: Record<string, ThreadInfo[]>;
  activeThreadId: string | null;
  onSelectThread: (id: string) => void;
  copilots: CopilotOption[];
  onNewThread: (
    workspaceId: string,
    copilot: { packageId: string } | null,
  ) => void;
  startTaskRequest?: StartTaskRequest | null;
  onStartTaskCancelled?: () => void;
  onArchiveThread: (workspaceId: string, threadId: string) => void;
  onRemoveWorkspace: (workspaceId: string) => void;
  baseUrl: string;
  token: string;
  onBaseUrlChange: (url: string) => void;
  onTokenChange: (token: string) => void;
  onCheckGateway: () => void;
  mcpServers: Record<string, {name: string; status: string; error?: string | null; failureReason?: string | null}>;
  rateLimits: Record<string, unknown> | null;
  currentProviderId: string | null;
  busy: boolean;
  theme: "light" | "dark";
  onToggleTheme: () => void;
  copilotStatus?: CopilotProfileStatus | null;
  copilotStatusLoading?: boolean;
  copilotStatusError?: string | null;
  onRefreshCopilots?: () => Promise<void>;
  onActivateCopilot?: (packageId: string) => Promise<void>;
  onDeactivateCopilot?: (packageId: string) => Promise<void>;
};

export default function Sidebar({
  gatewayState,
  gatewayVersion,
  workspaces,
  activeWorkspaceId,
  onSelectWorkspace,
  onCreateWorkspace,
  onLoadWorkspaces,
  onConnectWorkspace,
  threadsByWorkspace,
  activeThreadId,
  onSelectThread,
  copilots,
  onNewThread,
  startTaskRequest = null,
  onStartTaskCancelled,
  onArchiveThread,
  onRemoveWorkspace,
  baseUrl,
  token,
  onBaseUrlChange,
  onTokenChange,
  onCheckGateway,
  mcpServers,
  rateLimits,
  currentProviderId,
  busy,
  theme,
  onToggleTheme,
  copilotStatus = null,
  copilotStatusLoading = false,
  copilotStatusError = null,
  onRefreshCopilots = NOOP_ASYNC,
  onActivateCopilot = NOOP_ASYNC,
  onDeactivateCopilot = NOOP_ASYNC,
}: Props) {
  const [showSettings, setShowSettings] = useState(false);

  useEffect(() => {
    if (!showSettings) return;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      setShowSettings(false);
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [showSettings]);

  return (
    <aside className="web-sidebar">
      <div className="web-sidebar-scroll">
        <Brand state={gatewayState} version={gatewayVersion} />

        <Workspaces
          workspaces={workspaces}
          activeId={activeWorkspaceId}
          onSelect={onSelectWorkspace}
          onCreate={onCreateWorkspace}
          onConnect={onConnectWorkspace}
          onLoad={onLoadWorkspaces}
          threadsByWorkspace={threadsByWorkspace}
          activeThreadId={activeThreadId}
          onSelectThread={onSelectThread}
          copilots={copilots}
          onStartTask={onNewThread}
          onArchiveThread={onArchiveThread}
          onRemoveWorkspace={onRemoveWorkspace}
          busy={busy}
          copilotSelectionRequired={Boolean(
            copilotStatus?.packages.some((candidate) => candidate.available),
          )}
          onCopilotUnavailable={() => setShowSettings(true)}
          startTaskRequest={startTaskRequest}
          onStartTaskCancelled={onStartTaskCancelled}
        />

        <McpStatus servers={mcpServers} />


      </div>

      <div className="web-sidebar-bottom">
        {rateLimits && currentProviderId === "openai" ? <RateLimitCard rateLimits={rateLimits} /> : null}
        <div className="web-sidebar-actions">
          <button
            type="button"
            className="web-theme-toggle"
            aria-label={theme === "dark" ? "Switch to light theme" : "Switch to dark theme"}
            title={theme === "dark" ? "Light theme" : "Dark theme"}
            onClick={onToggleTheme}
          >
            {theme === "dark" ? <Sun size={16} aria-hidden="true" /> : <Moon size={16} aria-hidden="true" />}
          </button>
          <button
            type="button"
            className="web-settings-toggle"
            aria-label="Open settings"
            aria-haspopup="dialog"
            aria-expanded={showSettings}
            onClick={() => setShowSettings(true)}
          >
            <span className={`web-settings-toggle-icon${showSettings ? " web-settings-icon-open" : ""}`}>
              <Settings size={16} />
            </span>
            Settings
          </button>
        </div>
      </div>
      {showSettings && createPortal(
        <div
          className="web-settings-backdrop"
          onMouseDown={(event) => {
            if (event.target === event.currentTarget) setShowSettings(false);
          }}
        >
          <section className="web-settings-modal" role="dialog" aria-modal="true" aria-labelledby="web-settings-title">
            <div className="web-settings-modal-header">
              <div>
                <h2 id="web-settings-title">Settings</h2>
                <p>Configure the connection, maps, and Copilots.</p>
              </div>
              <button type="button" className="web-settings-close" aria-label="Close settings" onClick={() => setShowSettings(false)}>
                <X size={17} />
              </button>
            </div>
            <div className="web-settings-panel">
              <label>
                Gateway URL
                <input
                  autoFocus
                  value={baseUrl}
                  onChange={(e) => onBaseUrlChange(e.target.value)}
                  placeholder="http://127.0.0.1:4733"
                />
              </label>
              <label>
                Token
                <input
                  value={token}
                  onChange={(e) => onTokenChange(e.target.value)}
                  type="password"
                  placeholder="Optional auth token"
                />
              </label>
              <div className="web-settings-actions">
                <button
                  type="button"
                  className="web-settings-btn web-settings-btn-primary"
                  onClick={onCheckGateway}
                  disabled={busy}
                >
                  Save & Check
                </button>
              </div>
              {gatewayVersion && (
                <div className="web-version-info">Gateway v{gatewayVersion}</div>
              )}
            </div>
            <MapsSettings />
            <CopilotManager
              status={copilotStatus}
              loading={copilotStatusLoading}
              error={copilotStatusError}
              onRefresh={onRefreshCopilots}
              onActivate={onActivateCopilot}
              onDeactivate={onDeactivateCopilot}
            />
          </section>
        </div>,
        document.body,
      )}
    </aside>
  );
}
