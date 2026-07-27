import { useEffect, useMemo, useRef, useState } from "react";
import ChevronRight from "lucide-react/dist/esm/icons/chevron-right";
import Folder from "lucide-react/dist/esm/icons/folder";
import RefreshCw from "lucide-react/dist/esm/icons/refresh-cw";
import Search from "lucide-react/dist/esm/icons/search";
import X from "lucide-react/dist/esm/icons/x";
import type { GitFileStatus } from "../../types";
import { Markdown } from "../../features/messages/components/Markdown";
import { getFileTypeIconUrl } from "../../utils/fileTypeIcons";

type Props = {
  workspaceId: string | null;
  selectedPath: string | null;
  onSelectedPathChange: (path: string | null) => void;
  onClose: () => void;
  panelWidth: number;
  onPanelWidthChange: (width: number) => void;
  listFiles: (workspaceId: string) => Promise<string[]>;
  readFile: (workspaceId: string, path: string) => Promise<{ content: string; truncated: boolean }>;
  loadGitStatus: (workspaceId: string) => Promise<{ files: GitFileStatus[] }>;
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

export default function FileManager({ workspaceId, selectedPath, onSelectedPathChange, onClose, panelWidth, onPanelWidthChange, listFiles, readFile, loadGitStatus }: Props) {
  const [files, setFiles] = useState<string[]>([]);
  const [statuses, setStatuses] = useState<Map<string, string>>(new Map());
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [content, setContent] = useState("");
  const [truncated, setTruncated] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [treeOpen, setTreeOpen] = useState(true);
  const [query, setQuery] = useState("");
  const resizeSession = useRef<ResizeSession | null>(null);
  const refreshRequest = useRef(0);

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
    void refresh();
    return () => { refreshRequest.current += 1; };
  }, [workspaceId, listFiles, loadGitStatus]);

  useEffect(() => {
    let cancelled = false;
    setContent("");
    setTruncated(false);
    setError(null);
    if (!workspaceId || !selectedPath) {
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
  }, [readFile, selectedPath, workspaceId]);

  useEffect(() => () => {
    resizeSession.current?.cleanup(false);
  }, []);

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

  const clampPanelWidth = (width: number) => Math.min(MAX_PANEL_WIDTH, Math.max(MIN_PANEL_WIDTH, width));
  return (
    <aside className="web-file-manager" aria-label="Workspace files">
      <div
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
      />
      <div className="web-file-manager-header">
        <strong>Files</strong>
        <div>
          <button type="button" onClick={() => void refresh()} aria-label="Refresh files"><RefreshCw size={14} /></button>
          <button type="button" onClick={onClose} aria-label="Collapse file manager"><X size={15} /></button>
        </div>
      </div>
      <section className={`web-file-tree-section${treeOpen ? " is-open" : ""}`}>
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
          <div className="web-file-manager-tree">
            {!workspaceId ? <div className="web-file-empty">Select a workspace</div> : rows.length === 0 ? <div className="web-file-empty">No matching files</div> : rows.map((row) => {
              const status = statuses.get(row.path);
              const fileTypeIconUrl = row.folder ? null : getFileTypeIconUrl(row.path);
              return <button type="button" className={`web-file-row${selectedPath === row.path ? " is-active" : ""}`} key={row.path} style={{ paddingLeft: 8 + row.depth * 14 }} onClick={() => row.folder ? setExpanded((current) => { const next = new Set(current); next.has(row.path) ? next.delete(row.path) : next.add(row.path); return next; }) : onSelectedPathChange(row.path)}>
                {row.folder ? <ChevronRight size={13} className={expanded.has(row.path) ? "is-open" : ""} /> : <span className="web-file-spacer" />}
                {row.folder ? <Folder size={15} className="web-folder-icon" /> : <img className="web-file-type-icon" src={fileTypeIconUrl ?? ""} alt="" loading="lazy" decoding="async" />}
                <span className="web-file-name">{row.name}</span>
                {status && <span className={`web-file-status is-${status.includes("?") || status.includes("A") ? "added" : "modified"}`}>{status.includes("?") || status.includes("A") ? "A" : "M"}</span>}
              </button>;
            })}
          </div>
        </> : null}
      </section>
      <div className="web-file-preview">
        {selectedPath && <div className="web-file-preview-header"><span>{selectedPath}</span>{truncated && <em>Truncated</em>}</div>}
        {loading ? <div className="web-file-empty">Loading…</div> : error ? <div className="web-file-error">{error}</div> : selectedPath ? markdownPreview ? (
          <Markdown
            className="web-file-markdown"
            value={content}
            showFilePath={false}
            onOpenFileLink={(location) => onSelectedPathChange(resolveMarkdownLink(selectedPath, location.path))}
          />
        ) : <pre><code>{content}</code></pre> : <div className="web-file-empty">Select a file to preview</div>}
      </div>
    </aside>
  );
}
