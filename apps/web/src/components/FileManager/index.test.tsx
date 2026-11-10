// @vitest-environment jsdom
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import FileManager from "./index";

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("FileManager", () => {
  it("does not load supplementary file data while its tab is inactive", () => {
    const listFiles = vi.fn().mockResolvedValue(["README.md"]);
    const readFile = vi.fn().mockResolvedValue({ content: "# Project", truncated: false });
    render(
      <FileManager
        workspaceId="workspace-1"
        selectedPath="README.md"
        onSelectedPathChange={vi.fn()}
        onClose={vi.fn()}
        panelWidth={360}
        onPanelWidthChange={vi.fn()}
        listFiles={listFiles}
        readFile={readFile}
        loadGitStatus={vi.fn().mockResolvedValue({ files: [] })}
        embedded
        enabled={false}
      />,
    );

    expect(listFiles).not.toHaveBeenCalled();
    expect(readFile).not.toHaveBeenCalled();
  });

  it("shows git states and previews a selected file", async () => {
    const readFile = vi.fn().mockResolvedValue({ content: "export const value = 1;", truncated: false });
    render(
      <FileManager
        workspaceId="workspace-1"
        selectedPath={null}
        onSelectedPathChange={vi.fn()}
        onClose={vi.fn()}
        panelWidth={360}
        onPanelWidthChange={vi.fn()}
        listFiles={vi.fn().mockResolvedValue(["README.md", "src/config.ts"])}
        readFile={readFile}
        loadGitStatus={vi.fn().mockResolvedValue({ files: [{ path: "README.md", status: "??", additions: 1, deletions: 0 }] })}
      />,
    );

    expect(await screen.findByText("README.md")).toBeTruthy();
    expect(screen.getByText("A")).toBeTruthy();
    fireEvent.click(screen.getByText("src"));
    expect(await screen.findByText("config.ts")).toBeTruthy();
  });

  it("downloads and deletes a file from the workspace", async () => {
    const listFiles = vi.fn()
      .mockResolvedValueOnce(["README.md"])
      .mockResolvedValue([]);
    const downloadFile = vi.fn().mockResolvedValue({
      blob: new Blob(["initial\n"], { type: "text/plain" }),
      filename: "README.md",
    });
    const deleteFile = vi.fn().mockResolvedValue({ status: "deleted", path: "README.md" });
    vi.stubGlobal("URL", {
      createObjectURL: vi.fn().mockReturnValue("blob:workspace-file"),
      revokeObjectURL: vi.fn(),
    });
    render(
      <FileManager
        workspaceId="workspace-1"
        selectedPath="README.md"
        onSelectedPathChange={vi.fn()}
        onClose={vi.fn()}
        panelWidth={360}
        onPanelWidthChange={vi.fn()}
        listFiles={listFiles}
        readFile={vi.fn().mockResolvedValue({ content: "initial\n", truncated: false })}
        downloadFile={downloadFile}
        deleteFile={deleteFile}
        loadGitStatus={vi.fn().mockResolvedValue({ files: [] })}
      />,
    );

    await screen.findByText("README.md");
    fireEvent.click(screen.getByRole("button", { name: "Download README.md" }));
    await waitFor(() => expect(downloadFile).toHaveBeenCalledWith("workspace-1", "README.md"));

    fireEvent.click(screen.getByRole("button", { name: "Delete README.md" }));
    expect(deleteFile).not.toHaveBeenCalled();
    expect(screen.getByRole("alertdialog", { name: "Delete file?" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /^Delete$/ }));
    await waitFor(() => expect(deleteFile).toHaveBeenCalledWith("workspace-1", "README.md"));
    await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
    await waitFor(() => expect(screen.getByText("No Workspace files yet")).toBeTruthy());
  });

  it("cancels file deletion without calling the workspace API", async () => {
    const deleteFile = vi.fn().mockResolvedValue({ status: "deleted", path: "README.md" });
    render(
      <FileManager
        workspaceId="workspace-1"
        selectedPath={null}
        onSelectedPathChange={vi.fn()}
        onClose={vi.fn()}
        panelWidth={360}
        onPanelWidthChange={vi.fn()}
        listFiles={vi.fn().mockResolvedValue(["README.md"])}
        readFile={vi.fn().mockResolvedValue({ content: "", truncated: false })}
        deleteFile={deleteFile}
        loadGitStatus={vi.fn().mockResolvedValue({ files: [] })}
      />,
    );

    await screen.findByText("README.md");
    fireEvent.click(screen.getByRole("button", { name: "Delete README.md" }));
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(deleteFile).not.toHaveBeenCalled();
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(screen.getByText("README.md")).toBeTruthy();
  });

  it("keeps the deletion dialog open when the workspace API rejects the request", async () => {
    const deleteFile = vi.fn().mockRejectedValue(new Error("Workspace file could not be deleted"));
    render(
      <FileManager
        workspaceId="workspace-1"
        selectedPath={null}
        onSelectedPathChange={vi.fn()}
        onClose={vi.fn()}
        panelWidth={360}
        onPanelWidthChange={vi.fn()}
        listFiles={vi.fn().mockResolvedValue(["README.md"])}
        readFile={vi.fn().mockResolvedValue({ content: "", truncated: false })}
        deleteFile={deleteFile}
        loadGitStatus={vi.fn().mockResolvedValue({ files: [] })}
      />,
    );

    await screen.findByText("README.md");
    fireEvent.click(screen.getByRole("button", { name: "Delete README.md" }));
    fireEvent.click(screen.getByRole("button", { name: /^Delete$/ }));

    expect((await screen.findByRole("alert")).textContent).toContain("Workspace file could not be deleted");
    expect(screen.getByRole("alertdialog", { name: "Delete file?" })).toBeTruthy();
    expect(screen.getByText("README.md")).toBeTruthy();
  });

  it("uploads dropped files into the Workspace and refreshes the tree", async () => {
    const listFiles = vi.fn()
      .mockResolvedValueOnce([])
      .mockResolvedValue(["planning.csv"]);
    const uploadFiles = vi.fn().mockResolvedValue({
      status: "uploaded",
      paths: ["planning.csv"],
    });
    render(
      <FileManager
        workspaceId="workspace-1"
        selectedPath={null}
        onSelectedPathChange={vi.fn()}
        onClose={vi.fn()}
        panelWidth={360}
        onPanelWidthChange={vi.fn()}
        listFiles={listFiles}
        uploadFiles={uploadFiles}
        readFile={vi.fn().mockResolvedValue({ content: "", truncated: false })}
        loadGitStatus={vi.fn().mockResolvedValue({ files: [] })}
        embedded
      />,
    );

    expect(await screen.findByText("No Workspace files yet")).toBeTruthy();
    const file = new File(["city,demand\nJakarta,10\n"], "planning.csv", { type: "text/csv" });
    const dropzone = screen.getByTestId("workspace-file-dropzone");
    fireEvent.dragEnter(dropzone, { dataTransfer: { types: ["Files"] } });
    expect(screen.getByText("Drop files to upload")).toBeTruthy();
    fireEvent.drop(dropzone, {
      dataTransfer: { types: ["Files"], files: [file] },
    });

    await waitFor(() => expect(uploadFiles).toHaveBeenCalledWith("workspace-1", [file]));
    expect(await screen.findByText("planning.csv")).toBeTruthy();
    expect(screen.queryByText("Drop files to upload")).toBeNull();
  });

  it("uploads selections one file at a time and reports a later failure", async () => {
    const listFiles = vi.fn()
      .mockResolvedValueOnce([])
      .mockResolvedValue(["first.csv"]);
    const uploadFiles = vi.fn()
      .mockResolvedValueOnce({ status: "uploaded", paths: ["first.csv"] })
      .mockRejectedValueOnce(new Error("network unavailable"));
    render(
      <FileManager
        workspaceId="workspace-1"
        selectedPath={null}
        onSelectedPathChange={vi.fn()}
        onClose={vi.fn()}
        panelWidth={360}
        onPanelWidthChange={vi.fn()}
        listFiles={listFiles}
        uploadFiles={uploadFiles}
        readFile={vi.fn().mockResolvedValue({ content: "", truncated: false })}
        loadGitStatus={vi.fn().mockResolvedValue({ files: [] })}
        embedded
      />,
    );
    const first = new File(["first"], "first.csv", { type: "text/csv" });
    const second = new File(["second"], "second.csv", { type: "text/csv" });
    fireEvent.change(screen.getByLabelText("Upload Workspace files"), {
      target: { files: [first, second] },
    });

    await waitFor(() => expect(uploadFiles).toHaveBeenCalledTimes(2));
    expect(uploadFiles).toHaveBeenNthCalledWith(1, "workspace-1", [first]);
    expect(uploadFiles).toHaveBeenNthCalledWith(2, "workspace-1", [second]);
    expect(await screen.findByText(/second\.csv: network unavailable/)).toBeTruthy();
    await waitFor(() => expect(listFiles).toHaveBeenCalledTimes(2));
  });

  it("continues a multi-file selection after the user skips one conflict", async () => {
    const conflict = Object.assign(new Error("workspace_file_exists"), {
      kind: "conflict",
      code: "workspace_file_exists",
    });
    const listFiles = vi.fn()
      .mockResolvedValueOnce([])
      .mockResolvedValue(["first.csv", "third.csv"]);
    const uploadFiles = vi.fn()
      .mockResolvedValueOnce({ status: "uploaded", paths: ["first.csv"] })
      .mockRejectedValueOnce(conflict)
      .mockResolvedValueOnce({ status: "uploaded", paths: ["third.csv"] });
    vi.spyOn(window, "confirm").mockReturnValue(false);
    vi.spyOn(window, "prompt").mockReturnValue("");
    render(
      <FileManager
        workspaceId="workspace-1"
        selectedPath={null}
        onSelectedPathChange={vi.fn()}
        onClose={vi.fn()}
        panelWidth={360}
        onPanelWidthChange={vi.fn()}
        listFiles={listFiles}
        uploadFiles={uploadFiles}
        readFile={vi.fn().mockResolvedValue({ content: "", truncated: false })}
        loadGitStatus={vi.fn().mockResolvedValue({ files: [] })}
        embedded
      />,
    );
    const first = new File(["first"], "first.csv", { type: "text/csv" });
    const second = new File(["second"], "second.csv", { type: "text/csv" });
    const third = new File(["third"], "third.csv", { type: "text/csv" });
    fireEvent.change(screen.getByLabelText("Upload Workspace files"), {
      target: { files: [first, second, third] },
    });

    await waitFor(() => expect(uploadFiles).toHaveBeenCalledTimes(3));
    expect(uploadFiles).toHaveBeenNthCalledWith(1, "workspace-1", [first]);
    expect(uploadFiles).toHaveBeenNthCalledWith(2, "workspace-1", [second]);
    expect(uploadFiles).toHaveBeenNthCalledWith(3, "workspace-1", [third]);
    expect(window.confirm).toHaveBeenCalledTimes(1);
    expect(window.prompt).toHaveBeenCalledTimes(1);
    expect(await screen.findByText("third.csv")).toBeTruthy();
    expect(screen.queryByText(/Some files were not uploaded/)).toBeNull();
  });

  it("requires an explicit overwrite decision when a Workspace path exists", async () => {
    const conflict = Object.assign(new Error("workspace_file_exists"), {
      kind: "conflict",
      code: "workspace_file_exists",
    });
    const uploadFiles = vi.fn()
      .mockRejectedValueOnce(conflict)
      .mockResolvedValueOnce({ status: "uploaded", paths: ["planning.csv"] });
    vi.spyOn(window, "confirm").mockReturnValue(true);
    render(
      <FileManager
        workspaceId="workspace-1"
        selectedPath={null}
        onSelectedPathChange={vi.fn()}
        onClose={vi.fn()}
        panelWidth={360}
        onPanelWidthChange={vi.fn()}
        listFiles={vi.fn().mockResolvedValue(["planning.csv"])}
        uploadFiles={uploadFiles}
        readFile={vi.fn().mockResolvedValue({ content: "", truncated: false })}
        loadGitStatus={vi.fn().mockResolvedValue({ files: [] })}
        embedded
      />,
    );
    const file = new File(["new"], "planning.csv", { type: "text/csv" });
    await screen.findByText("planning.csv");
    fireEvent.change(screen.getByLabelText("Upload Workspace files"), {
      target: { files: [file] },
    });
    await waitFor(() => expect(uploadFiles).toHaveBeenCalledTimes(2));
    expect(uploadFiles).toHaveBeenNthCalledWith(1, "workspace-1", [file]);
    expect(uploadFiles).toHaveBeenNthCalledWith(
      2,
      "workspace-1",
      [file],
      { overwrite: true },
    );
  });

  it("supports rename or cancel without silently overwriting", async () => {
    const conflict = Object.assign(new Error("workspace_file_exists"), {
      kind: "conflict",
      code: "workspace_file_exists",
    });
    const uploadFiles = vi.fn()
      .mockRejectedValueOnce(conflict)
      .mockResolvedValueOnce({ status: "uploaded", paths: ["renamed.csv"] });
    vi.spyOn(window, "confirm").mockReturnValue(false);
    vi.spyOn(window, "prompt").mockReturnValue("inputs/renamed.csv");
    const { rerender } = render(
      <FileManager
        workspaceId="workspace-1"
        selectedPath={null}
        onSelectedPathChange={vi.fn()}
        onClose={vi.fn()}
        panelWidth={360}
        onPanelWidthChange={vi.fn()}
        listFiles={vi.fn().mockResolvedValue([])}
        uploadFiles={uploadFiles}
        readFile={vi.fn().mockResolvedValue({ content: "", truncated: false })}
        loadGitStatus={vi.fn().mockResolvedValue({ files: [] })}
        embedded
      />,
    );
    const file = new File(["new"], "planning.csv", { type: "text/csv" });
    fireEvent.change(screen.getByLabelText("Upload Workspace files"), {
      target: { files: [file] },
    });
    await waitFor(() => expect(uploadFiles).toHaveBeenCalledTimes(2));
    expect(uploadFiles).toHaveBeenNthCalledWith(
      2,
      "workspace-1",
      [file],
      { paths: ["inputs/renamed.csv"] },
    );

    const cancelledUpload = vi.fn().mockRejectedValue(conflict);
    vi.mocked(window.prompt).mockReturnValue("");
    rerender(
      <FileManager
        workspaceId="workspace-1"
        selectedPath={null}
        onSelectedPathChange={vi.fn()}
        onClose={vi.fn()}
        panelWidth={360}
        onPanelWidthChange={vi.fn()}
        listFiles={vi.fn().mockResolvedValue([])}
        uploadFiles={cancelledUpload}
        readFile={vi.fn().mockResolvedValue({ content: "", truncated: false })}
        loadGitStatus={vi.fn().mockResolvedValue({ files: [] })}
        embedded
      />,
    );
    fireEvent.change(screen.getByLabelText("Upload Workspace files"), {
      target: { files: [file] },
    });
    await waitFor(() => expect(cancelledUpload).toHaveBeenCalledTimes(1));
  });

  it("loads a file selected by an external message link", async () => {
    const readFile = vi.fn().mockResolvedValue({ content: "# Project", truncated: false });
    render(
      <FileManager
        workspaceId="workspace-1"
        selectedPath="README.md"
        onSelectedPathChange={vi.fn()}
        onClose={vi.fn()}
        panelWidth={360}
        onPanelWidthChange={vi.fn()}
        listFiles={vi.fn().mockResolvedValue(["README.md"])}
        readFile={readFile}
        loadGitStatus={vi.fn().mockResolvedValue({ files: [] })}
      />,
    );
    expect(await screen.findByRole("heading", { name: "Project" })).toBeTruthy();
    expect(readFile).toHaveBeenCalledWith("workspace-1", "README.md");
  });

  it("renders markdown previews and opens relative file links in the file manager", async () => {
    const onSelectedPathChange = vi.fn();
    render(
      <FileManager
        workspaceId="workspace-1"
        selectedPath="docs/guide.md"
        onSelectedPathChange={onSelectedPathChange}
        onClose={vi.fn()}
        panelWidth={360}
        onPanelWidthChange={vi.fn()}
        listFiles={vi.fn().mockResolvedValue(["docs/guide.md", "src/config.ts"])}
        readFile={vi.fn().mockResolvedValue({
          content: "# Guide\n\n**Ready**\n\n| Item | State |\n| --- | --- |\n| Build | OK |\n\n[Config](../src/config.ts)",
          truncated: false,
        })}
        loadGitStatus={vi.fn().mockResolvedValue({ files: [] })}
      />,
    );

    expect(await screen.findByRole("heading", { name: "Guide" })).toBeTruthy();
    expect(screen.getByText("Ready").tagName).toBe("STRONG");
    expect(screen.getByRole("table")).toBeTruthy();
    fireEvent.click(screen.getByRole("link", { name: "Config" }));
    expect(onSelectedPathChange).toHaveBeenCalledWith("src/config.ts");
  });

  it("renders CSV files as a table and switches to the raw text", async () => {
    render(
      <FileManager
        workspaceId="workspace-1"
        selectedPath="data/sites.csv"
        onSelectedPathChange={vi.fn()}
        onClose={vi.fn()}
        panelWidth={360}
        onPanelWidthChange={vi.fn()}
        listFiles={vi.fn().mockResolvedValue(["data/sites.csv"])}
        readFile={vi.fn().mockResolvedValue({
          content: 'site,city,demand\nS1,"Jakarta, ID",1200\nS2,Bandung,85.5\n',
          truncated: false,
        })}
        loadGitStatus={vi.fn().mockResolvedValue({ files: [] })}
      />,
    );

    const table = await screen.findByRole("table");
    expect(screen.getByRole("columnheader", { name: "city" })).toBeTruthy();
    expect(screen.getByRole("cell", { name: "Jakarta, ID" })).toBeTruthy();
    expect(screen.getByRole("cell", { name: "1200" }).className).toBe("is-numeric");
    expect(screen.getByRole("cell", { name: "Bandung" }).className).toBe("");
    expect(screen.getByText(/2 rows · 3 columns/)).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Raw" }));
    expect(table.isConnected).toBe(false);
    expect(screen.getByText(/S1,"Jakarta, ID",1200/)).toBeTruthy();
  });

  it("formats and highlights JSON files and switches to the raw text", async () => {
    const { container } = render(
      <FileManager
        workspaceId="workspace-1"
        selectedPath="config/areas.json"
        onSelectedPathChange={vi.fn()}
        onClose={vi.fn()}
        panelWidth={360}
        onPanelWidthChange={vi.fn()}
        listFiles={vi.fn().mockResolvedValue(["config/areas.json"])}
        readFile={vi.fn().mockResolvedValue({ content: '{"name":"Jakarta","count":3}', truncated: false })}
        loadGitStatus={vi.fn().mockResolvedValue({ files: [] })}
      />,
    );

    await waitFor(() => expect(container.querySelector(".web-file-code-body .token.property")).toBeTruthy());
    expect(container.querySelector(".web-file-code-body")?.textContent).toBe('{\n  "name": "Jakarta",\n  "count": 3\n}');
    expect(container.querySelector(".web-file-code-gutter")?.textContent).toBe("1\n2\n3\n4");
    expect(screen.getByRole("button", { name: "Formatted" }).getAttribute("aria-pressed")).toBe("true");

    fireEvent.click(screen.getByRole("button", { name: "Raw" }));
    expect(container.querySelector(".web-file-code-body")?.textContent).toBe('{"name":"Jakarta","count":3}');
  });

  it("ignores a stale preview error after switching workspaces", async () => {
    let rejectOldPreview: (reason: Error) => void = () => undefined;
    const oldPreview = new Promise<{ content: string; truncated: boolean }>((_, reject) => {
      rejectOldPreview = reject;
    });
    const readFile = vi.fn().mockImplementation((workspaceId: string) => workspaceId === "workspace-1"
      ? oldPreview
      : Promise.resolve({ content: "", truncated: false }));
    const commonProps = {
      onSelectedPathChange: vi.fn(),
      onClose: vi.fn(),
      panelWidth: 360,
      onPanelWidthChange: vi.fn(),
      listFiles: vi.fn().mockResolvedValue(["README.md"]),
      readFile,
      loadGitStatus: vi.fn().mockResolvedValue({ files: [] }),
    };
    const view = render(
      <FileManager {...commonProps} workspaceId="workspace-1" selectedPath="old-file.md" />,
    );

    expect(readFile).toHaveBeenCalledWith("workspace-1", "old-file.md");
    view.rerender(
      <FileManager {...commonProps} workspaceId="workspace-2" selectedPath={null} />,
    );
    await act(async () => {
      rejectOldPreview(new Error("Failed to open file: No such file or directory (os error 2)"));
      await oldPreview.catch(() => undefined);
    });

    expect(screen.queryByText(/No such file or directory/)).toBeNull();
    expect(screen.getByText("Select a file to preview")).toBeTruthy();
  });

  it("ignores a stale file listing after the active Thread changes", async () => {
    let resolveOldFiles: (files: string[]) => void = () => undefined;
    const oldFiles = new Promise<string[]>((resolve) => {
      resolveOldFiles = resolve;
    });
    const commonProps = {
      workspaceId: "workspace-1",
      selectedPath: null,
      onSelectedPathChange: vi.fn(),
      onClose: vi.fn(),
      panelWidth: 360,
      onPanelWidthChange: vi.fn(),
      readFile: vi.fn().mockResolvedValue({ content: "", truncated: false }),
    };
    const view = render(
      <FileManager
        {...commonProps}
        listFiles={vi.fn().mockReturnValue(oldFiles)}
        loadGitStatus={vi.fn().mockResolvedValue({ files: [] })}
      />,
    );

    view.rerender(
      <FileManager
        {...commonProps}
        listFiles={vi.fn().mockResolvedValue(["new-thread.txt"])}
        loadGitStatus={vi.fn().mockResolvedValue({ files: [] })}
      />,
    );
    expect(await screen.findByText("new-thread.txt")).toBeTruthy();

    await act(async () => {
      resolveOldFiles(["old-thread.txt"]);
      await oldFiles;
    });
    expect(screen.queryByText("old-thread.txt")).toBeNull();
    expect(screen.getByText("new-thread.txt")).toBeTruthy();
  });

  it("collapses the file list independently from the panel", async () => {
    render(
      <FileManager
        workspaceId="workspace-1"
        selectedPath={null}
        onSelectedPathChange={vi.fn()}
        onClose={vi.fn()}
        panelWidth={360}
        onPanelWidthChange={vi.fn()}
        listFiles={vi.fn().mockResolvedValue(["README.md"])}
        readFile={vi.fn().mockResolvedValue({ content: "", truncated: false })}
        loadGitStatus={vi.fn().mockResolvedValue({ files: [] })}
      />,
    );

    expect(await screen.findByText("README.md")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /Workspace/ }));
    expect(screen.queryByText("README.md")).toBeNull();
    expect(screen.getByText("Select a file to preview")).toBeTruthy();
  });

  it("supports keyboard resizing and uses material file icons", async () => {
    const onPanelWidthChange = vi.fn();
    const view = render(
      <FileManager
        workspaceId="workspace-1"
        selectedPath={null}
        onSelectedPathChange={vi.fn()}
        onClose={vi.fn()}
        panelWidth={360}
        onPanelWidthChange={onPanelWidthChange}
        listFiles={vi.fn().mockResolvedValue(["src/config.ts", "README.md"])}
        readFile={vi.fn().mockResolvedValue({ content: "", truncated: false })}
        loadGitStatus={vi.fn().mockResolvedValue({ files: [] })}
      />,
    );

    expect(await screen.findByText("README.md")).toBeTruthy();
    fireEvent.keyDown(screen.getByRole("separator", { name: "Resize file manager" }), { key: "ArrowLeft" });
    expect(onPanelWidthChange).toHaveBeenCalledWith(376);
    expect(view.container.querySelector('img[src*="material-icons"]')).toBeTruthy();
  });

  it("updates the CSS width during pointer dragging and commits React state only on release", async () => {
    const onPanelWidthChange = vi.fn();
    const view = render(
      <div className="web-app-shell">
        <FileManager
          workspaceId="workspace-1"
          selectedPath={null}
          onSelectedPathChange={vi.fn()}
          onClose={vi.fn()}
          panelWidth={360}
          onPanelWidthChange={onPanelWidthChange}
          listFiles={vi.fn().mockResolvedValue([])}
          readFile={vi.fn().mockResolvedValue({ content: "", truncated: false })}
          loadGitStatus={vi.fn().mockResolvedValue({ files: [] })}
        />
      </div>,
    );

    await screen.findByText("No Workspace files yet");
    const separator = screen.getByRole("separator", { name: "Resize file manager" });

    fireEvent.pointerDown(separator, { pointerId: 1, clientX: 500 });
    fireEvent.pointerMove(window, { pointerId: 1, clientX: 450 });

    const shell = view.container.querySelector<HTMLElement>(".web-app-shell");
    expect(shell?.style.getPropertyValue("--web-file-panel-width")).toBe("410px");
    expect(shell?.classList.contains("web-files-resizing")).toBe(true);
    expect(onPanelWidthChange).not.toHaveBeenCalled();

    fireEvent.pointerUp(window, { pointerId: 1, clientX: 450 });
    expect(onPanelWidthChange).toHaveBeenCalledTimes(1);
    expect(onPanelWidthChange).toHaveBeenCalledWith(410);
    expect(shell?.classList.contains("web-files-resizing")).toBe(false);
  });
});
