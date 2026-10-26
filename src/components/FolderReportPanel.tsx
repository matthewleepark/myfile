import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";

import {
  convertApi,
  reportApi,
  speechApi,
  toAppError,
  type AppError,
  type ReportProgress,
  type ReportSummary,
} from "../api";
import { formatSize } from "../format";
import { AlertIcon, CheckIcon, MicIcon, SaveIcon } from "../icons";
import { ErrorPanel, Spinner } from "./common";

const TARGETS: { id: string; label: string; hint: string }[] = [
  { id: "docx", label: "Word", hint: "one document" },
  { id: "pdf", label: "PDF", hint: "one document" },
  { id: "md", label: "Markdown", hint: "one document" },
  { id: "html", label: "HTML", hint: "a web page" },
  { id: "txt", label: "Text", hint: "plain text" },
  { id: "json", label: "JSON", hint: "for other programs" },
  { id: "csv", label: "File list", hint: "CSV, one row per file" },
];

type Run =
  | { status: "idle" }
  | { status: "running"; transcribing: boolean }
  | { status: "done"; r: ReportSummary }
  | { status: "error"; error: AppError };

/** Folder mode's main screen: read every file, then save it all as one file. */
export function FolderReportPanel({ selectionId }: { selectionId: number }) {
  const [run, setRun] = useState<Run>({ status: "idle" });
  const [last, setLast] = useState<ReportSummary | null>(null);
  const [progress, setProgress] = useState<ReportProgress | null>(null);
  const [saving, setSaving] = useState<string | null>(null);
  const [saved, setSaved] = useState<{ ok: boolean; text: string } | null>(null);
  const [speechReady, setSpeechReady] = useState(false);

  const runningRef = useRef(false);
  runningRef.current = run.status === "running";

  async function read(transcribe: boolean) {
    setProgress(null);
    setSaved(null);
    setRun({ status: "running", transcribing: transcribe });
    try {
      const r = await reportApi.read(selectionId, transcribe);
      setLast(r);
      setRun({ status: "done", r });
    } catch (e) {
      const err = toAppError(e);
      setRun(err.code === "cancelled" ? (last ? { status: "done", r: last } : { status: "idle" }) : { status: "error", error: err });
    }
  }

  useEffect(() => {
    const off = listen<ReportProgress>("report-progress", (e) => setProgress(e.payload));
    speechApi.status().then((s) => setSpeechReady(s.modelInstalled), () => {});
    read(false);
    return () => {
      off.then((f) => f());
      if (runningRef.current) convertApi.cancel();
    };
    // Read once when the folder opens.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectionId]);

  async function save(target: string) {
    setSaving(target);
    setSaved(null);
    try {
      const r = await reportApi.export(selectionId, target);
      const s = await convertApi.save(r.conversionId);
      if (s) setSaved({ ok: true, text: `Saved as “${s.name}” (${formatSize(r.totalBytes)}).` });
    } catch (e) {
      setSaved({ ok: false, text: toAppError(e).message });
    } finally {
      setSaving(null);
    }
  }

  return (
    <section className="panel" aria-label="Everything in this folder">
      <h3 className="section-title">Everything in this folder, as one file</h3>

      {run.status === "running" && (
        <div className="folder-progress" role="status">
          <div className="progress-head">
            <Spinner />
            <span>
              {progress && progress.total > 0
                ? `${run.transcribing ? "Transcribing and reading" : "Reading"} file ${Math.min(progress.done + 1, progress.total)} of ${progress.total}`
                : "Finding the files…"}
              {progress?.current ? ` · ${progress.current}` : ""}
            </span>
            <button type="button" className="btn btn-secondary" onClick={() => convertApi.cancel()}>
              Cancel
            </button>
          </div>
          <div className="progress-bar" aria-hidden="true">
            <span style={{ width: `${progress ? (100 * progress.done) / Math.max(1, progress.total) : 0}%` }} />
          </div>
        </div>
      )}

      {run.status === "idle" && (
        <div className="ai-run-row">
          <button type="button" className="btn btn-primary" onClick={() => read(false)}>
            Read every file
          </button>
          <span className="muted small">Everything is read on this computer.</span>
        </div>
      )}

      {run.status === "error" && (
        <>
          <ErrorPanel title="MyFile couldn't read this folder" error={run.error} />
          <button type="button" className="btn btn-secondary" onClick={() => read(false)}>
            Try again
          </button>
        </>
      )}

      {run.status === "done" && <Done r={run.r} speechReady={speechReady} onTranscribe={() => read(true)} />}

      {run.status === "done" && (
        <div className="format-group">
          <span className="format-group-label">Save everything as</span>
          <div className="format-grid">
            {TARGETS.map((t) => (
              <button
                key={t.id}
                type="button"
                className="format-chip"
                onClick={() => save(t.id)}
                disabled={saving !== null}
                title={t.hint}
              >
                <span className="format-chip-label">{saving === t.id ? "Saving…" : t.label}</span>
                <span className="format-chip-hint">{t.hint}</span>
              </button>
            ))}
          </div>
          {saved && (
            <p className={`feedback ${saved.ok ? "is-ok" : "is-bad"}`} role="status">
              {saved.ok ? <CheckIcon width={16} height={16} /> : <AlertIcon width={16} height={16} />} {saved.text}
            </p>
          )}
          <p className="muted small">
            <SaveIcon width={14} height={14} /> One file with a list of everything, then each file's details and contents. The
            files in the folder aren't changed.
          </p>
        </div>
      )}
    </section>
  );
}

function Done({ r, speechReady, onTranscribe }: { r: ReportSummary; speechReady: boolean; onTranscribe: () => void }) {
  const waiting = r.files.filter((f) => f.recording && !f.transcribed).length;
  return (
    <>
      <div className="stat-row">
        <div className="stat">
          <span className="stat-value">{r.files.length}</span>
          <span className="stat-label">files · {formatSize(r.totalBytes)}</span>
        </div>
        <div className="stat">
          <span className="stat-value">{r.read}</span>
          <span className="stat-label">with readable contents</span>
        </div>
        <div className="stat">
          <span className="stat-value">{r.totalWords.toLocaleString()}</span>
          <span className="stat-label">words extracted</span>
        </div>
      </div>

      {r.notes.map((n) => (
        <p key={n} className="muted small">
          {n}
        </p>
      ))}
      {waiting > 0 &&
        (speechReady ? (
          <div className="ai-run-row">
            <button type="button" className="btn btn-secondary" onClick={onTranscribe}>
              <MicIcon width={16} height={16} /> Also transcribe the {waiting} recording{waiting === 1 ? "" : "s"}
            </button>
            <span className="muted small">Slower: about a minute per 10 minutes of audio.</span>
          </div>
        ) : (
          <p className="muted small">
            To include what's said in the recordings, open one and choose “Transcribe to text” once to download the speech
            model.
          </p>
        ))}

      <details className="report-files">
        <summary>What was found in each file</summary>
        <ul className="doc-list">
          {r.files.map((f) => (
            <li key={f.path} className="doc-row report-row">
              <span className="doc-name" title={f.path}>
                {f.path}
              </span>
              <span className="muted small">{[f.label, ...f.details, formatSize(f.sizeBytes)].join(" · ")}</span>
              <span className="doc-status small">
                {f.words > 0 ? (
                  <span className="ok-text">
                    <CheckIcon width={14} height={14} /> {f.words.toLocaleString()} word{f.words === 1 ? "" : "s"}
                  </span>
                ) : f.note ? (
                  <span className="warn-text">
                    <AlertIcon width={14} height={14} /> {f.note}
                  </span>
                ) : (
                  <span className="muted">Details only</span>
                )}
              </span>
            </li>
          ))}
        </ul>
      </details>
    </>
  );
}
