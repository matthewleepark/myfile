import { useCallback, useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import {
  api,
  folderApi,
  toAppError,
  type ActionId,
  type AppError,
  type FolderSummary,
  type AiTask,
  type RouteDecision,
  type SelectedFile,
} from "./api";
import { baseName } from "./format";
import { useFileDrop, type DropPoint } from "./useFileDrop";
import { DropZone } from "./components/DropZone";
import { FileCard } from "./components/FileCard";
import { ActionList } from "./components/ActionList";
import { QuickMenu } from "./components/QuickMenu";
import { ErrorPanel, Spinner } from "./components/common";
import { TextPreviewPanel } from "./components/TextPreviewPanel";
import { PdfTextPanel } from "./components/PdfTextPanel";
import { ImagePanel } from "./components/ImagePanel";
import { AiPanel } from "./components/AiPanel";
import { ComparePanel } from "./components/ComparePanel";
import { MonitorPanel } from "./components/MonitorPanel";
import { TemplatePanel } from "./components/TemplatePanel";
import { TranscribePanel } from "./components/TranscribePanel";
import { AskBar } from "./components/AskBar";
import { FolderView } from "./components/FolderView";
import { ConvertPanel } from "./components/ConvertPanel";

type FileState =
  | { status: "idle" }
  | { status: "validating"; name: string }
  | { status: "selected"; file: SelectedFile; key: number }
  | { status: "folder"; folder: FolderSummary; key: number }
  | { status: "error"; name: string; error: AppError };

const PANEL_TITLES: Record<ActionId, string> = {
  "preview-text": "Preview",
  "extract-pdf-text": "Extracted text",
  "read-image-text": "Text in this image (OCR)",
  "compare-document": "Compare versions",
  "monitor-document": "Monitor for changes",
  "extract-fields": "Fill a template",
  transcribe: "Transcribe to text",
  "compress-image": "Compress or resize",
  "ai-assistant": "AI document assistant",
};

export default function App() {
  const [fileState, setFileState] = useState<FileState>({ status: "idle" });
  const [active, setActive] = useState<ActionId | null>(null);
  // The latest typed-request decision; n remounts the panel for each new one.
  const [routed, setRouted] = useState<{ d: RouteDecision; n: number } | null>(null);
  // A task started from another panel (e.g. "Summarize it" on a transcript).
  const [launch, setLaunch] = useState<AiTask | null>(null);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [thumbnail, setThumbnail] = useState<string | null>(null);
  const [quickMenu, setQuickMenu] = useState<DropPoint | null>(null);
  // Only the most recent selection may update the screen, even if an older
  // validation finishes later.
  const latestRequest = useRef(0);
  // Bumped whenever the user changes file or picks an action themselves, so a
  // routing decision that arrives afterwards is ignored instead of hijacking
  // the screen.
  const routeToken = useRef(0);
  const panelRef = useRef<HTMLElement>(null);

  async function selectPath(path: string, menuAt: DropPoint | null = null) {
    const request = ++latestRequest.current;
    routeToken.current++;
    setActive(null);
    setRouted(null);
    setBusy(false);
    setThumbnail(null);
    setQuickMenu(null);
    setFileState({ status: "validating", name: baseName(path) });
    try {
      const file = await api.selectFile(path);
      if (request !== latestRequest.current) return;
      setFileState({ status: "selected", file, key: request });
      if (menuAt) setQuickMenu(menuAt);
      if (file.kind === "image") {
        api.imageThumbnail().then(
          (url) => request === latestRequest.current && setThumbnail(url),
          () => {}, // a missing preview isn't worth an error message
        );
      }
    } catch (e) {
      const err = toAppError(e);
      if (err.code === "is_folder") return openFolder(path, request);
      if (request === latestRequest.current) {
        setFileState({ status: "error", name: baseName(path), error: err });
      }
    }
  }

  /** Semester brain: every PDF and text file in a folder at once. */
  async function openFolder(path: string, request = ++latestRequest.current) {
    routeToken.current++;
    setActive(null);
    setRouted(null);
    setThumbnail(null);
    setQuickMenu(null);
    setFileState({ status: "validating", name: baseName(path) + " (reading every document locally)" });
    try {
      const folder = await folderApi.open(path);
      if (request === latestRequest.current) setFileState({ status: "folder", folder, key: request });
    } catch (e) {
      if (request === latestRequest.current) setFileState({ status: "error", name: baseName(path), error: toAppError(e) });
    }
  }

  const dragging = useFileDrop(async (paths, at) => {
    const shift = await api.shiftHeld().catch(() => false);
    setNotice(
      paths.length > 1
        ? `MyFile works with one file at a time, so it opened “${baseName(paths[0])}” and ignored the other ${paths.length - 1}.`
        : null,
    );
    selectPath(paths[0], shift ? at : null);
  });

  async function browse() {
    const picked = await open({ multiple: false, directory: false, title: "Choose a file" });
    if (typeof picked === "string") {
      setNotice(null);
      selectPath(picked);
    }
  }

  async function remove() {
    latestRequest.current++;
    routeToken.current++;
    setFileState({ status: "idle" });
    setActive(null);
    setRouted(null);
    setBusy(false);
    setThumbnail(null);
    setQuickMenu(null);
    setNotice(null);
    await api.clearFile();
    await folderApi.close();
  }

  function pick(id: ActionId) {
    routeToken.current++;
    setQuickMenu(null);
    setRouted(null);
    setLaunch(null);
    setActive(id);
  }

  // Bring the panel into view when an action is chosen.
  useEffect(() => {
    if (active) panelRef.current?.scrollIntoView({ behavior: "smooth", block: "start" });
  }, [active]);

  const closeQuickMenu = useCallback(() => setQuickMenu(null), []);

  return (
    <div className={`app${dragging ? " is-dragging" : ""}`}>
      <header className="topbar">
        <div className="brand">
          <span className="brand-mark" aria-hidden="true">M</span>
          <h1 className="brand-name">MyFile</h1>
        </div>
        <span className="brand-tag">Convert any file</span>
      </header>

      <main className="content">
        {notice && (
          <p className="notice" role="status">
            {notice}
          </p>
        )}

        {fileState.status === "idle" && <DropZone dragging={dragging} onBrowse={browse} />}

        {fileState.status === "folder" && (
          <FolderView key={fileState.key} folder={fileState.folder} onClose={remove} />
        )}

        {fileState.status === "validating" && (
          <section className="file-card is-loading" aria-busy="true">
            <Spinner />
            <p role="status">Checking “{fileState.name}”…</p>
          </section>
        )}

        {fileState.status === "error" && (
          <>
            <ErrorPanel title={`MyFile couldn't open “${fileState.name}”`} error={fileState.error} />
            <DropZone dragging={dragging} onBrowse={browse} />
          </>
        )}

        {fileState.status === "selected" && (
          <>
            {dragging && (
              <p className="notice" role="status">
                Release to replace “{fileState.file.name}” with the new file.
              </p>
            )}
            <FileCard file={fileState.file} thumbnail={thumbnail} onChange={browse} onRemove={remove} />
            <ConvertPanel key={fileState.key} selectionId={fileState.file.selectionId} onBusy={setBusy} />
            {fileState.file.actions.length > 0 && (
            <details className="more-tools" open={active !== null}>
            <summary>More tools for this file</summary>
            <AskBar
              key={fileState.key}
              kind={fileState.file.kind}
              selectionId={fileState.file.selectionId}
              disabled={busy}
              onStart={() => ++routeToken.current}
              onDecision={(d, token) => {
                if (token !== routeToken.current) return; // the user moved on
                setQuickMenu(null);
                setRouted((prev) => ({ d, n: (prev?.n ?? 0) + 1 }));
                if (d.action !== "none") setActive(d.action);
              }}
            />
            <ActionList
              kind={fileState.file.kind}
              actions={fileState.file.actions}
              activeId={active}
              busy={busy}
              onPick={pick}
            />
            {active && (
              <section className="panel" ref={panelRef} aria-label={PANEL_TITLES[active]} key={`${fileState.key}-${active}-${routed?.n ?? 0}`}>
                <h3 className="section-title">{PANEL_TITLES[active]}</h3>
                {active === "preview-text" && <TextPreviewPanel />}
                {(active === "extract-pdf-text" || active === "read-image-text") && <PdfTextPanel onBusy={setBusy} />}
                {active === "extract-fields" && <TemplatePanel scope="file" selectionId={fileState.file.selectionId} />}
                {active === "transcribe" && (
                  <TranscribePanel
                    selectionId={fileState.file.selectionId}
                    onBusy={setBusy}
                    onActions={(actions) =>
                      setFileState((s) => (s.status === "selected" ? { ...s, file: { ...s.file, actions } } : s))
                    }
                    onAsk={(task) => {
                      pick("ai-assistant");
                      setLaunch(task);
                    }}
                    onTemplate={() => pick("extract-fields")}
                  />
                )}
                {active === "monitor-document" && (
                  <MonitorPanel selectionId={fileState.file.selectionId} onAdded={() => {}} />
                )}
                {active === "compare-document" && (
                  <ComparePanel selectionId={fileState.file.selectionId} fileName={fileState.file.name} onBusy={setBusy} />
                )}
                {active === "compress-image" && (
                  <ImagePanel onBusy={setBusy} initial={routed?.d.imageOptions} autoRun={!!routed} />
                )}
                {active === "ai-assistant" && (
                  <AiPanel
                    fileName={fileState.file.name}
                    selectionId={fileState.file.selectionId}
                    onBusy={setBusy}
                    initialTask={routed?.d.aiTask ?? launch ?? undefined}
                    initialQuestion={routed?.d.question ?? undefined}
                    autoRun={!!routed || !!launch}
                  />
                )}
              </section>
            )}
            </details>
            )}
            {quickMenu && (
              <QuickMenu
                fileName={fileState.file.name}
                actions={fileState.file.actions}
                x={quickMenu.x}
                y={quickMenu.y}
                onPick={pick}
                onClose={closeQuickMenu}
              />
            )}
          </>
        )}
      </main>
    </div>
  );
}
