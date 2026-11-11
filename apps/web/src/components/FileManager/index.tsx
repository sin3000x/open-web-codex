import { Suspense, lazy, useEffect, useId, useMemo, useRef, useState } from "react";
import type { ChangeEvent, DragEvent } from "react";
import { createPortal } from "react-dom";
import ChevronRight from "lucide-react/dist/esm/icons/chevron-right";
import Download from "lucide-react/dist/esm/icons/download";
import Folder from "lucide-react/dist/esm/icons/folder";
import RefreshCw from "lucide-react/dist/esm/icons/refresh-cw";
import Search from "lucide-react/dist/esm/icons/search";
import Trash2 from "lucide-react/dist/esm/icons/trash-2";
import Upload from "lucide-react/dist/esm/icons/upload";
import X from "lucide-react/dist/esm/icons/x";
import type { GitFileStatus } from "../../types";
import { Markdown } from "../../features/messages/components/Markdown";
import { getFileTypeIconUrl } from "../../utils/fileTypeIcons";
import { languageFromPath } from "../../utils/languageFromPath";
import { DelimitedTablePreview } from "./DelimitedTablePreview";
import { delimiterForPath } from "./delimitedText";

// Syntax highlighting pulls in Prism and its grammars; load it only when a
// code file is actually previewed.
const CodePreview = lazy(() => import("./CodePreview"));

type Props = {
  workspaceId: string | null;
  selectedPath: string | null;
  onSelectedPathChange: (path: string | null) => void;
  onClose: () => void;
  panelWidth: number;
  onPanelWidthChange: (width: number) => void;
  listFiles: (workspaceId: string) => Promise<string[]>;
  uploadFiles?: (
    workspaceId: string,
    files: File[],
    options?: { overwrite?: boolean; paths?: string[] },
  ) => Promise<unknown>;
  readFile: (workspaceId: string, path: string) => Promise<{ content: string; truncated: boolean }>;
  downloadFile?: (workspaceId: string, path: string) => Promise<{ blob: Blob; filename: string }>;
  deleteFile?: (workspaceId: string, path: string) => Promise<unknown>;
  loadGitStatus: (workspaceId: string) => Promise<{ files: GitFileStatus[] }>;
  embedded?: boolean;
  enabled?: boolean;
};

type Row = { path: string; name: string; depth: number; folder: boolean };
type ResizeSession = {
  x: number;
  width: number;
  currentWidth: number;
  shell: HTMLElement;
  manager: HTMLElement;
  cleanup: (commit: boolean) => void;
};

const MIN_PANEL_WIDTH = 260;
const MAX_PANEL_WIDTH = 720;
const MARKDOWN_FILE_PATTERN = /\.(?:md|markdown|mdown|mkd|mdx)$/i;

function resolveMarkdownLink(currentPath: string, targetPath: string) {
  const sourceDirectory = currentPath.split("/").slice(0, -1);
  const targetParts = targetPath.startsWith("/")
    ? targetPath.slice(1).split("/")
    : [...sourceDirectory, ...targetPath.split("/")];
  const resolved: string[] = [];
  for (const part of targetParts) {
    if (!part || part === ".") continue;
    if (part === "..") resolved.pop();
    else resolved.push(part);
  }
  return resolved.join("/");
}

export default function FileManager({ workspaceId, selectedPath, onSelectedPathChange, onClose, panelWidth, onPanelWidthChange, listFiles, uploadFiles, readFile, downloadFile, deleteFile, loadGitStatus, embedded = false, enabled = true }: Props) {
  const [files, setFiles] = useState<string[]>([]);
  const [statuses, setStatuses] = useState<Map<string, string>>(new Map());
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [content, setContent] = useState("");
  const [truncated, setTruncated] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [treeOpen, setTreeOpen] = useState(true);
  const [formattedView, setFormattedView] = useState(true);
  const [query, setQuery] = useState("");
  const [fileActionPath, setFileActionPath] = useState<string | null>(null);
  const [pendingDeletePath, setPendingDeletePath] = useState<string | null>(null);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const [isDragOver, setIsDragOver] = useState(false);
  const [uploadingFiles, setUploadingFiles] = useState(false);
  const uploadInput = useRef<HTMLInputElement | null>(null);
  const resizeSession = useRef<ResizeSession | null>(null);
  const refreshRequest = useRef(0);
  const dragDepth = useRef(0);
  const deleteTitleId = useId();
  const deleteDescriptionId = useId();

  const refresh = async () => {
    if (!workspaceId) return;
    const request = ++refreshRequest.current;
    setLoading(true);
    setError(null);
    try {
      const [nextFiles, git] = await Promise.all([listFiles(workspaceId), loadGitStatus(workspaceId).catch(() => ({ files: [] }))]);
      if (request !== refreshRequest.current) return;
      setFiles(nextFiles);
      setStatuses(new Map(git.files.map((file) => [file.path, file.status])));
    } catch (reason) {
      if (request !== refreshRequest.current) return;
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      if (request === refreshRequest.current) setLoading(false);
    }
  };

  useEffect(() => {
    if (!enabled) return;
    void refresh();
    return () => { refreshRequest.current += 1; };
  }, [enabled, workspaceId, listFiles, loadGitStatus]);

  useEffect(() => {
    let cancelled = false;
    setContent("");
    setTruncated(false);
    setError(null);
    if (!enabled || !workspaceId || !selectedPath) {
      setLoading(false);
      return () => { cancelled = true; };
    }
    const parents = selectedPath.split("/").slice(0, -1);
    setExpanded((current) => {
      const next = new Set(current);
      parents.forEach((_, index) => next.add(parents.slice(0, index + 1).join("/")));
      return next;
    });
    setLoading(true);
    readFile(workspaceId, selectedPath).then((result) => {
      if (cancelled) return;
      setContent(result.content);
      setTruncated(result.truncated);
    }).catch((reason) => {
      if (!cancelled) setError(reason instanceof Error ? reason.message : String(reason));
    }).finally(() => {
      if (!cancelled) setLoading(false);
    });
    return () => { cancelled = true; };
  }, [enabled, readFile, selectedPath, workspaceId]);

  useEffect(() => () => {
    resizeSession.current?.cleanup(false);
  }, []);

  useEffect(() => {
    setPendingDeletePath(null);
    setDeleteError(null);
  }, [workspaceId]);

  useEffect(() => {
    if (!pendingDeletePath) return;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || fileActionPath === pendingDeletePath) return;
      setPendingDeletePath(null);
      setDeleteError(null);
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [fileActionPath, pendingDeletePath]);

  const handleDownload = async (path: string) => {
    if (!workspaceId || !downloadFile) return;
    setFileActionPath(path);
    setError(null);
    try {
      const { blob, filename } = await downloadFile(workspaceId, path);
      if (typeof URL.createObjectURL !== "function") {
        throw new Error("The browser does not support file downloads.");
      }
      const objectUrl = URL.createObjectURL(blob);
      const anchor = document.createElement("a");
      anchor.href = objectUrl;
      anchor.download = filename;
      anchor.style.display = "none";
      document.body.appendChild(anchor);
      anchor.click();
      anchor.remove();
      window.setTimeout(() => URL.revokeObjectURL(objectUrl), 0);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setFileActionPath(null);
    }
  };

  const handleDelete = async () => {
    const path = pendingDeletePath;
    if (!workspaceId || !deleteFile || !path) return;
    setFileActionPath(path);
    setDeleteError(null);
    try {
      await deleteFile(workspaceId, path);
      if (selectedPath === path) onSelectedPathChange(null);
      setPendingDeletePath(null);
      await refresh();
    } catch (reason) {
      setDeleteError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setFileActionPath(null);
    }
  };

  const handleUpload = async (files: File[]) => {
    if (!workspaceId || !uploadFiles || files.length === 0) return;
    setUploadingFiles(true);
    setError(null);
    let changed = false;
    const failures: string[] = [];
    try {
      for (const file of files) {
        const currentPath = file.webkitRelativePath || file.name;
        try {
          try {
            await uploadFiles(workspaceId, [file]);
          } catch (reason) {
            const platformError = reason as Error & { kind?: string; code?: string };
            if (platformError.kind !== "conflict" || platformError.code !== "workspace_file_exists") {
              throw reason;
            }
            const overwrite = window.confirm(
              `${currentPath} already exists. Select OK to overwrite it, or Cancel to rename this file or skip it.`,
            );
            if (overwrite) {
              await uploadFiles(workspaceId, [file], { overwrite: true });
            } else {
              const renamed = window.prompt(
                `Enter a new Workspace-relative path for ${currentPath}. Leave it blank to skip this file.`,
                currentPath,
              );
              if (!renamed?.trim()) continue;
              await uploadFiles(workspaceId, [file], { paths: [renamed.trim()] });
            }
          }
          changed = true;
        } catch (reason) {
          const message = reason instanceof Error ? reason.message : String(reason);
          failures.push(`${currentPath}: ${message}`);
        }
      }
      if (changed) await refresh();
      if (failures.length > 0) {
        setError(`Some files were not uploaded: ${failures.join("; ")}`);
      }
    } finally {
      setUploadingFiles(false);
    }
  };

  const hasFiles = (event: DragEvent<HTMLElement>) =>
    Array.from(event.dataTransfer.types).some((type) =>
      type === "Files" || type === "public.file-url" || type === "application/x-moz-file",
    );
  const handleDragEnter = (event: DragEvent<HTMLElement>) => {
    if (!uploadFiles || !workspaceId || !hasFiles(event)) return;
    event.preventDefault();
    dragDepth.current += 1;
    setTreeOpen(true);
    setIsDragOver(true);
  };
  const handleDragOver = (event: DragEvent<HTMLElement>) => {
    if (!uploadFiles || !workspaceId || !hasFiles(event)) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "copy";
  };
  const handleDragLeave = (event: DragEvent<HTMLElement>) => {
    if (!uploadFiles || !workspaceId || !hasFiles(event)) return;
    event.preventDefault();
    dragDepth.current = Math.max(0, dragDepth.current - 1);
    if (dragDepth.current === 0) setIsDragOver(false);
  };
  const handleDrop = (event: DragEvent<HTMLElement>) => {
    if (!uploadFiles || !workspaceId || !hasFiles(event)) return;
    event.preventDefault();
    dragDepth.current = 0;
    setIsDragOver(false);
    void handleUpload(Array.from(event.dataTransfer.files));
  };
  const handleFileInputChange = (event: ChangeEvent<HTMLInputElement>) => {
    const files = Array.from(event.currentTarget.files ?? []);
    event.currentTarget.value = "";
    void handleUpload(files);
  };

  const rows = useMemo(() => {
    const folders = new Set<string>();
    files.forEach((path) => path.split("/").slice(0, -1).forEach((_, index, parts) => folders.add(parts.slice(0, index + 1).join("/"))));
    const normalizedQuery = query.trim().toLocaleLowerCase();
    const visibleWhenFiltering = new Set<string>();
    if (normalizedQuery) {
      [...folders, ...files].forEach((path) => {
        if (!path.toLocaleLowerCase().includes(normalizedQuery)) return;
        visibleWhenFiltering.add(path);
        const parts = path.split("/");
        parts.slice(0, -1).forEach((_, index) => visibleWhenFiltering.add(parts.slice(0, index + 1).join("/")));
      });
    }
    const all = [...folders, ...files].sort((a, b) => a.localeCompare(b));
    return all.filter((path) => {
      if (normalizedQuery) return visibleWhenFiltering.has(path);
      const parent = path.split("/").slice(0, -1).join("/");
      return !parent || expanded.has(parent);
    }).map((path): Row => ({ path, name: path.split("/").pop() ?? path, depth: path.split("/").length - 1, folder: folders.has(path) }));
  }, [expanded, files, query]);
  const markdownPreview = Boolean(selectedPath && MARKDOWN_FILE_PATTERN.test(selectedPath));
  const tableDelimiter = selectedPath ? delimiterForPath(selectedPath) : null;
  const formattedViewLabel = tableDelimiter ? "Table" : languageFromPath(selectedPath) === "json" ? "Formatted" : null;
  const clampPanelWidth = (width: number) => Math.min(MAX_PANEL_WIDTH, Math.max(MIN_PANEL_WIDTH, width));
  return <>
    <aside className={`web-file-manager${embedded ? " is-embedded" : ""}`} aria-label="Workspace files">
      {!embedded ? <div
        className="web-file-manager-resizer"
        role="separator"
        aria-label="Resize file manager"
        aria-orientation="vertical"
        aria-valuemin={MIN_PANEL_WIDTH}
        aria-valuemax={MAX_PANEL_WIDTH}
        aria-valuenow={panelWidth}
        tabIndex={0}
        onPointerDown={(event) => {
          const manager = event.currentTarget.closest<HTMLElement>(".web-file-manager");
          const shell = event.currentTarget.closest<HTMLElement>(".web-app-shell");
          if (!manager || !shell) return;
          const session: ResizeSession = {
            x: event.clientX,
            width: panelWidth,
            currentWidth: panelWidth,
            shell,
            manager,
            cleanup: () => undefined,
          };
          const move = (moveEvent: PointerEvent) => {
            const width = clampPanelWidth(session.width + session.x - moveEvent.clientX);
            if (width === session.currentWidth) return;
            session.currentWidth = width;
            session.shell.style.setProperty("--web-file-panel-width", `${width}px`);
          };
          const finish = (commit: boolean) => {
            window.removeEventListener("pointermove", move);
            window.removeEventListener("pointerup", finishPointer);
            window.removeEventListener("pointercancel", cancelPointer);
            session.shell.classList.remove("web-files-resizing");
            session.manager.classList.remove("is-resizing");
            if (resizeSession.current === session) resizeSession.current = null;
            if (commit) onPanelWidthChange(session.currentWidth);
          };
          const finishPointer = () => finish(true);
          const cancelPointer = () => finish(false);
          session.cleanup = finish;
          resizeSession.current = session;
          shell.classList.add("web-files-resizing");
          manager.classList.add("is-resizing");
          window.addEventListener("pointermove", move);
          window.addEventListener("pointerup", finishPointer, { once: true });
          window.addEventListener("pointercancel", cancelPointer, { once: true });
        }}
        onKeyDown={(event) => {
          if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
          event.preventDefault();
          onPanelWidthChange(clampPanelWidth(panelWidth + (event.key === "ArrowLeft" ? 16 : -16)));
        }}
      /> : null}
      <div className="web-file-manager-header">
        <strong>{embedded ? "Workspace" : "Files"}</strong>
        <div>
          {uploadFiles ? <>
            <button
              type="button"
              onClick={() => uploadInput.current?.click()}
              disabled={!workspaceId || uploadingFiles}
              aria-label="Upload files"
              title="Upload files"
            ><Upload size={14} aria-hidden="true" /></button>
            <input
              ref={uploadInput}
              className="web-file-upload-input"
              type="file"
              aria-label="Upload Workspace files"
              multiple
              tabIndex={-1}
              aria-hidden="true"
              onChange={handleFileInputChange}
            />
          </> : null}
          <button type="button" onClick={() => void refresh()} aria-label="Refresh files"><RefreshCw size={14} /></button>
          {!embedded ? <button type="button" onClick={onClose} aria-label="Collapse file manager"><X size={15} /></button> : null}
        </div>
      </div>
      <section
        className={`web-file-tree-section${treeOpen ? " is-open" : ""}${isDragOver ? " is-drag-over" : ""}`}
        data-testid="workspace-file-dropzone"
        onDragEnter={handleDragEnter}
        onDragOver={handleDragOver}
        onDragLeave={handleDragLeave}
        onDrop={handleDrop}
      >
        <button type="button" className="web-file-tree-heading" aria-expanded={treeOpen} onClick={() => setTreeOpen((open) => !open)}>
          <ChevronRight size={13} className={treeOpen ? "is-open" : ""} />
          <span>Workspace</span>
          <span className="web-file-count">{files.length}</span>
        </button>
        {treeOpen ? <>
          <label className="web-file-filter">
            <Search size={13} aria-hidden="true" />
            <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Filter files…" aria-label="Filter files" />
          </label>
          {uploadFiles && workspaceId ? <div className="web-file-drop-caption">
            <Upload size={12} aria-hidden="true" />
            <span>Drag files here to add them to this Workspace</span>
          </div> : null}
          <div className="web-file-manager-tree">
            {isDragOver ? <div className="web-file-drop-overlay" role="status">
              <Upload size={20} aria-hidden="true" />
              <strong>Drop files to upload</strong>
              <span>Files will be added to this Workspace.</span>
            </div> : null}
            {uploadingFiles ? <div className="web-file-upload-status" role="status">Uploading files…</div> : null}
            {!workspaceId ? <div className="web-file-empty">Select a workspace</div> : rows.length === 0 && !query.trim() ? (
              <div className="web-file-empty">
                <strong>No Workspace files yet</strong>
                <span>Upload files to make them available to every Task in this Workspace.</span>
                <button
                  type="button"
                  onClick={() => uploadInput.current?.click()}
                  disabled={!uploadFiles || uploadingFiles}
                  data-testid="workspace-files-empty-upload"
                >
                  Upload files
                </button>
              </div>
            ) : rows.length === 0 ? <div className="web-file-empty">No matching files</div> : rows.map((row) => {
              const status = statuses.get(row.path);
              const statusLabel = status?.includes("D") ? "D" : status?.includes("?") || status?.includes("A") ? "A" : status ? "M" : null;
              const fileTypeIconUrl = row.folder ? null : getFileTypeIconUrl(row.path);
              const actionPending = fileActionPath === row.path;
              return <div className="web-file-row-wrap" key={row.path} style={{ paddingLeft: 8 + row.depth * 14 }}>
                <button type="button" className={`web-file-row${selectedPath === row.path ? " is-active" : ""}`} onClick={() => row.folder ? setExpanded((current) => { const next = new Set(current); next.has(row.path) ? next.delete(row.path) : next.add(row.path); return next; }) : onSelectedPathChange(row.path)}>
                  {row.folder ? <ChevronRight size={13} className={expanded.has(row.path) ? "is-open" : ""} /> : <span className="web-file-spacer" />}
                  {row.folder ? <Folder size={15} className="web-folder-icon" /> : <img className="web-file-type-icon" src={fileTypeIconUrl ?? ""} alt="" loading="lazy" decoding="async" />}
                  <span className="web-file-name">{row.name}</span>
                  {statusLabel && <span className={`web-file-status is-${statusLabel === "A" ? "added" : statusLabel === "D" ? "deleted" : "modified"}`}>{statusLabel}</span>}
                </button>
                {!row.folder && statusLabel !== "D" && (downloadFile || deleteFile) ? <div className="web-file-actions">
                  {downloadFile ? <button
                    type="button"
                    className="web-file-action"
                    aria-label={`Download ${row.path}`}
                    title="Download file"
                    disabled={actionPending}
                    onClick={(event) => { event.stopPropagation(); void handleDownload(row.path); }}
                  ><Download size={13} aria-hidden="true" /></button> : null}
                  {deleteFile ? <button
                    type="button"
                    className="web-file-action is-danger"
                    aria-label={`Delete ${row.path}`}
                    title="Delete file"
                    disabled={actionPending}
                    onClick={(event) => {
                      event.stopPropagation();
                      setDeleteError(null);
                      setPendingDeletePath(row.path);
                    }}
                  ><Trash2 size={13} aria-hidden="true" /></button> : null}
                </div> : null}
              </div>;
            })}
          </div>
        </> : null}
      </section>
      <div className="web-file-preview">
        {selectedPath && <div className="web-file-preview-header">
          <span>{selectedPath}</span>
          {truncated && <em>Truncated</em>}
          {formattedViewLabel ? <div className="web-file-view-toggle" role="group" aria-label="Preview mode">
            <button type="button" aria-pressed={formattedView} onClick={() => setFormattedView(true)}>{formattedViewLabel}</button>
            <button type="button" aria-pressed={!formattedView} onClick={() => setFormattedView(false)}>Raw</button>
          </div> : null}
        </div>}
        {loading ? <div className="web-file-empty">Loading…</div> : error ? <div className="web-file-error">{error}</div> : selectedPath ? tableDelimiter && formattedView ? (
          <DelimitedTablePreview content={content} delimiter={tableDelimiter} truncated={truncated} />
        ) : markdownPreview ? (
          <Markdown
            className="web-file-markdown"
            value={content}
            showFilePath={false}
            onOpenFileLink={(location) => onSelectedPathChange(resolveMarkdownLink(selectedPath, location.path))}
          />
        ) : (
          <Suspense fallback={<div className="web-file-empty">Loading…</div>}>
            <CodePreview path={selectedPath} content={content} truncated={truncated} formatted={formattedView} />
          </Suspense>
        ) : <div className="web-file-empty">Select a file to preview</div>}
      </div>
    </aside>
    {pendingDeletePath ? createPortal(
      <div
        className="web-settings-backdrop"
        onMouseDown={(event) => {
          if (event.target !== event.currentTarget || fileActionPath === pendingDeletePath) return;
          setPendingDeletePath(null);
          setDeleteError(null);
        }}
      >
        <section
          className="web-file-delete-modal"
          role="alertdialog"
          aria-modal="true"
          aria-labelledby={deleteTitleId}
          aria-describedby={deleteDescriptionId}
        >
          <div className="web-file-delete-modal-icon"><Trash2 size={18} aria-hidden="true" /></div>
          <div className="web-file-delete-modal-copy">
            <h2 id={deleteTitleId}>Delete file?</h2>
            <p id={deleteDescriptionId}>
              “{pendingDeletePath}” will be permanently removed from this Workspace.
            </p>
            {deleteError ? <p className="web-file-delete-error" role="alert">{deleteError}</p> : null}
          </div>
          <div className="web-file-delete-modal-actions">
            <button
              type="button"
              disabled={fileActionPath === pendingDeletePath}
              onClick={() => {
                setPendingDeletePath(null);
                setDeleteError(null);
              }}
            >
              Cancel
            </button>
            <button
              type="button"
              className="is-danger"
              autoFocus
              disabled={fileActionPath === pendingDeletePath}
              onClick={() => void handleDelete()}
            >
              {fileActionPath === pendingDeletePath ? "Deleting…" : "Delete"}
            </button>
          </div>
        </section>
      </div>,
      document.body,
    ) : null}
  </>;
}
