import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import Archive from "lucide-react/dist/esm/icons/archive";
import ChevronRight from "lucide-react/dist/esm/icons/chevron-right";
import Folder from "lucide-react/dist/esm/icons/folder";
import LoaderCircle from "lucide-react/dist/esm/icons/loader-circle";
import MessageSquare from "lucide-react/dist/esm/icons/message-square";
import Plus from "lucide-react/dist/esm/icons/plus";
import Sparkles from "lucide-react/dist/esm/icons/sparkles";
import Trash2 from "lucide-react/dist/esm/icons/trash-2";
import type { WorkspaceInfo } from "../../types";

type ThreadInfo = {
  id: string;
  label: string;
  updatedAt: number;
  turnCount?: number;
  status?: string;
  creationStatus?: "creating" | "failed";
};

export type CopilotOption = {
  packageId: string;
  displayName: string;
};

/** A request from outside the sidebar to start a task in one Workspace. */
export type StartTaskRequest = {
  id: number;
  workspaceId: string;
};

type Props = {
  workspaces: WorkspaceInfo[];
  activeId: string | null;
  onSelect: (id: string) => void;
  onCreate: (name: string) => void;
  onConnect: (id: string) => void;
  onLoad: () => void;
  busy: boolean;
  threadsByWorkspace: Record<string, ThreadInfo[]>;
  activeThreadId: string | null;
  onSelectThread: (id: string) => void;
  copilots: CopilotOption[];
  onStartTask: (
    workspaceId: string,
    copilot: { packageId: string } | null,
  ) => void;
  onArchiveThread: (workspaceId: string, threadId: string) => void;
  onRemoveWorkspace: (workspaceId: string) => void;
  copilotSelectionRequired?: boolean;
  onCopilotUnavailable?: () => void;
  startTaskRequest?: StartTaskRequest | null;
  onStartTaskCancelled?: () => void;
};

export default function Workspaces({
  workspaces,
  activeId,
  onSelect,
  onCreate,
  onConnect,
  busy,
  threadsByWorkspace,
  activeThreadId,
  onSelectThread,
  copilots,
  onStartTask,
  onArchiveThread,
  onRemoveWorkspace,
  copilotSelectionRequired = false,
  onCopilotUnavailable = () => undefined,
  startTaskRequest = null,
  onStartTaskCancelled = () => undefined,
}: Props) {
  const [createName, setCreateName] = useState("");
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [pendingArchive, setPendingArchive] = useState<{
    workspaceId: string;
    threadId: string;
    label: string;
  } | null>(null);
  const [pendingTaskWorkspace, setPendingTaskWorkspace] = useState<WorkspaceInfo | null>(null);
  const [selectedPackageId, setSelectedPackageId] = useState<string | null>(null);
  useEffect(() => {
    if (!pendingArchive && !pendingTaskWorkspace) return;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      setPendingArchive(null);
      if (pendingTaskWorkspace) {
        setPendingTaskWorkspace(null);
        onStartTaskCancelled();
      }
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [onStartTaskCancelled, pendingArchive, pendingTaskWorkspace]);

  const openCopilotPicker = (workspace: WorkspaceInfo) => {
    if (copilots.length === 0) {
      if (copilotSelectionRequired) {
        onStartTaskCancelled();
        onCopilotUnavailable();
        return;
      }
      onStartTask(workspace.id, null);
      return;
    }
    // A single active Copilot is already an explicit Profile choice.
    if (copilots.length === 1) {
      onStartTask(workspace.id, { packageId: copilots[0].packageId });
      return;
    }
    setSelectedPackageId(copilots[0]?.packageId ?? null);
    setPendingTaskWorkspace(workspace);
  };

  const closeCopilotPicker = () => {
    setPendingTaskWorkspace(null);
    onStartTaskCancelled();
  };

  // Every task start, including the composer's first message, goes through the
  // same Copilot choice so the server always receives an explicit selection.
  const handleStartTaskRequest = useRef<(request: StartTaskRequest) => void>(() => undefined);
  handleStartTaskRequest.current = (request) => {
    const workspace = workspaces.find((candidate) => candidate.id === request.workspaceId);
    if (!workspace) {
      onStartTaskCancelled();
      return;
    }
    openCopilotPicker(workspace);
  };
  const handledStartTaskRequestId = useRef<number | null>(null);
  useEffect(() => {
    if (!startTaskRequest || handledStartTaskRequestId.current === startTaskRequest.id) return;
    handledStartTaskRequestId.current = startTaskRequest.id;
    handleStartTaskRequest.current(startTaskRequest);
  }, [startTaskRequest]);

  const createTask = () => {
    if (!pendingTaskWorkspace || !selectedPackageId) return;
    const copilot = copilots.find((candidate) => candidate.packageId === selectedPackageId);
    if (!copilot) return;
    onStartTask(pendingTaskWorkspace.id, {
      packageId: copilot.packageId,
    });
    setPendingTaskWorkspace(null);
  };

  const toggleExpand = (wsId: string) => {
    setExpandedId(prev => (prev === wsId ? null : wsId));
  };

  const threadsFor = (wsId: string): ThreadInfo[] => {
    return threadsByWorkspace[wsId] ?? [];
  };

  const activeWorkspace = workspaces.find((candidate) => candidate.id === activeId) ?? null;

  const handleCreate = () => {
    const name = createName.trim();
    if (name) { onCreate(name); setCreateName(""); }
  };

  return (
    <div className="web-ws-section">
      <div className="web-ws-header">
        <span className="web-ws-header-label">Workspaces</span>
        <span className="web-ws-header-count">{workspaces.length}</span>
      </div>

      <button
        type="button"
        className="web-ws-new-task-primary"
        onClick={() => {
          if (!activeWorkspace) return;
          setExpandedId(activeWorkspace.id);
          openCopilotPicker(activeWorkspace);
        }}
        disabled={busy || !activeWorkspace}
        title={activeWorkspace ? `New task in ${activeWorkspace.name}` : "Select a workspace first"}
      >
        <Plus size={14} aria-hidden="true" />
        <span>New task</span>
      </button>

      <div className="web-ws-create">
        <input
          value={createName}
          onChange={(e) => setCreateName(e.target.value)}
          onKeyDown={(e) => { if (e.key === "Enter") handleCreate(); }}
          placeholder="New workspace..."
          disabled={busy}
          className="web-ws-create-input"
        />
        <button
          className="web-ws-create-btn"
          onClick={handleCreate}
          disabled={busy || !createName.trim()}
          title="Create workspace"
        >
          <span className="web-ws-create-plus" aria-hidden="true" />
        </button>
      </div>

      {workspaces.length === 0 && (
        <div className="web-ws-empty">No workspaces</div>
      )}

      <div className="web-ws-list">
        {workspaces.map((ws) => {
          const isExpanded = expandedId === ws.id || (expandedId === null && ws.id === activeId);
          const threads = threadsFor(ws.id);
          const isActive = ws.id === activeId;

          return (
            <div key={ws.id} className="web-ws-tree-node">
              {/* Workspace row */}
              <div
                className={`web-ws-row${isActive ? " web-ws-row-active" : ""}`}
                onClick={() => {
                  toggleExpand(ws.id);
                  if (!ws.connected) onConnect(ws.id);
                  onSelect(ws.id);
                }}
              >
                <span
                  className={`web-ws-arrow${isExpanded ? " web-ws-arrow-open" : ""}`}
                >
                  <ChevronRight size={12} />
                </span>
                <span
                  className={`web-ws-dot ${ws.connected ? "web-ws-dot-on" : "web-ws-dot-off"}`}
                />
                <Folder size={14} className="web-ws-folder-icon" />
                <span className="web-ws-name">{ws.name}</span>
                {threads.length > 0 && (
                  <span className="web-ws-thread-count">{threads.length}</span>
                )}
                <button
                  type="button"
                  className="web-ws-row-action web-ws-remove-btn"
                  onClick={(event) => {
                    event.stopPropagation();
                    onRemoveWorkspace(ws.id);
                  }}
                  disabled={busy}
                  aria-label={`Remove workspace ${ws.name}`}
                  title="Remove workspace"
                >
                  <Trash2 size={13} aria-hidden="true" />
                </button>
                <button
                  type="button"
                  className="web-ws-row-action web-ws-new-thread-btn"
                  onClick={(event) => {
                    event.stopPropagation();
                    setExpandedId(ws.id);
                    openCopilotPicker(ws);
                  }}
                  disabled={busy}
                  aria-label={`New task in ${ws.name}`}
                  title="New task"
                >
                  <Sparkles size={13} aria-hidden="true" />
                </button>
              </div>

              {/* Thread list (collapsible) */}
              {isExpanded && (
              <div className="web-ws-threads">
                {threads.length === 0 && (
                  <div className="web-ws-threads-empty">No threads</div>
                )}
                {threads.map((t) => {
                  const isRunning = t.creationStatus === "creating"
                    || t.status === "active"
                    || t.status === "running"
                    || t.status === "reconnecting";
                  const isFailed = t.creationStatus === "failed";

                  return (
                    <div
                      key={t.id}
                      className={`web-ws-thread${t.id === activeThreadId ? " web-ws-thread-active" : ""}`}
                      onClick={() => onSelectThread(t.id)}
                    >
                      <span
                        className={`web-ws-thread-status${isFailed ? " is-failed" : ""}`}
                        aria-label={isFailed ? "Creation failed" : "Thread"}
                      >
                        <MessageSquare size={12} className="web-ws-thread-icon" />
                      </span>
                      <span className="web-ws-thread-label">{t.label}</span>
                      {isRunning && (
                        <span
                          className="web-ws-thread-running"
                          role="status"
                          aria-label={t.creationStatus === "creating" ? "Creating thread" : "Thread is running"}
                          title={t.creationStatus === "creating" ? "Creating thread" : "Thread is running"}
                        >
                          <LoaderCircle size={13} aria-hidden="true" />
                        </span>
                      )}
                      <button
                        type="button"
                        className="web-ws-thread-archive"
                        aria-label={`Archive thread ${t.label}`}
                        title="Archive thread"
                        disabled={busy || Boolean(t.creationStatus) || isRunning}
                        onClick={(event) => {
                          event.stopPropagation();
                          setPendingArchive({ workspaceId: ws.id, threadId: t.id, label: t.label });
                        }}
                      >
                        <Archive size={12} aria-hidden="true" />
                      </button>
                    </div>
                  );
                })}

              </div>)}
            </div>
          );
        })}
      </div>
      {pendingTaskWorkspace && createPortal(
        <div
          className="web-settings-backdrop"
          onMouseDown={(event) => {
            if (event.target === event.currentTarget) closeCopilotPicker();
          }}
        >
          <section
            className="web-copilot-modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="web-copilot-title"
            aria-describedby="web-copilot-description"
          >
            <div className="web-copilot-modal-copy">
              <h2 id="web-copilot-title">Choose a Copilot</h2>
              <p id="web-copilot-description">
                Select the Copilot for the new Thread in {pendingTaskWorkspace.name}.
              </p>
            </div>
            <div className="web-copilot-options" role="radiogroup" aria-label="Available Copilots">
              {copilots.map((copilot) => {
                const selected = copilot.packageId === selectedPackageId;
                return (
                  <button
                    key={copilot.packageId}
                    type="button"
                    className={`web-copilot-option${selected ? " is-selected" : ""}`}
                    role="radio"
                    aria-checked={selected}
                    onClick={() => setSelectedPackageId(copilot.packageId)}
                  >
                    <strong>{copilot.displayName}</strong>
                    <span>{copilot.packageId}</span>
                  </button>
                );
              })}
            </div>
            <div className="web-copilot-actions">
              <button type="button" onClick={closeCopilotPicker}>
                Cancel
              </button>
              <button type="button" className="is-primary" onClick={createTask}>
                Create Thread
              </button>
            </div>
          </section>
        </div>,
        document.body,
      )}
      {pendingArchive && createPortal(
        <div
          className="web-settings-backdrop"
          onMouseDown={(event) => {
            if (event.target === event.currentTarget) setPendingArchive(null);
          }}
        >
          <section className="web-archive-modal" role="alertdialog" aria-modal="true" aria-labelledby="web-archive-title" aria-describedby="web-archive-description">
            <div className="web-archive-modal-icon"><Archive size={18} aria-hidden="true" /></div>
            <div className="web-archive-modal-copy">
              <h2 id="web-archive-title">Archive thread?</h2>
              <p id="web-archive-description">
                “{pendingArchive.label}” will leave this list, but its history remains recoverable in Codex.
              </p>
            </div>
            <div className="web-archive-modal-actions">
              <button type="button" onClick={() => setPendingArchive(null)}>Cancel</button>
              <button
                type="button"
                className="is-primary"
                autoFocus
                disabled={busy}
                onClick={() => {
                  onArchiveThread(pendingArchive.workspaceId, pendingArchive.threadId);
                  setPendingArchive(null);
                }}
              >
                Archive
              </button>
            </div>
          </section>
        </div>,
        document.body,
      )}
    </div>
  );
}
