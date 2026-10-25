import { useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";

import { api, speechApi, toAppError, type ActionInfo, type AiTask, type AppError, type SpeechStatus, type Transcript } from "../api";
import { formatSize } from "../format";
import { LockIcon, SaveIcon } from "../icons";
import { ErrorPanel, OutputButtons, Spinner } from "./common";

interface Props {
  selectionId: number;
  onBusy: (busy: boolean) => void;
  /** The transcript is ready, so these actions now work on this file. */
  onActions: (actions: ActionInfo[]) => void;
  /** Open the AI assistant on the transcript with this task. */
  onAsk: (task: AiTask) => void;
  onTemplate: () => void;
}

type Progress = { phase: "download" | "transcribe"; done: number; total: number | null };

function clock(ms: number) {
  const s = Math.floor(ms / 1000);
  const hms = s >= 3600 ? [Math.floor(s / 3600), Math.floor(s / 60) % 60, s % 60] : [Math.floor(s / 60), s % 60];
  return hms.map((n, i) => (i === 0 ? String(n) : String(n).padStart(2, "0"))).join(":");
}

/**
 * Speech to text, on this computer: a one-time model download, then
 * transcription with progress, a searchable timestamped transcript,
 * subtitle export, and the AI features on the transcript.
 */
export function TranscribePanel({ selectionId, onBusy, onActions, onAsk, onTemplate }: Props) {
  const [status, setStatus] = useState<SpeechStatus | null>(null);
  const [run, setRun] = useState<
    | { status: "idle" }
    | { status: "downloading" }
    | { status: "running"; started: number }
    | { status: "done"; t: Transcript }
    | { status: "error"; error: AppError }
  >({ status: "idle" });
  const [progress, setProgress] = useState<Progress | null>(null);
  const [now, setNow] = useState(Date.now());

  useEffect(() => {
    speechApi.status().then(setStatus, (e) => setRun({ status: "error", error: toAppError(e) }));
    speechApi.current(selectionId).then((t) => t && setRun({ status: "done", t }), () => {});
    const off = listen<Progress>("speech-progress", (e) => setProgress(e.payload));
    return () => {
      off.then((f) => f());
    };
  }, [selectionId]);

  useEffect(() => {
    if (run.status !== "running") return;
    const id = setInterval(() => setNow(Date.now()), 500);
    return () => clearInterval(id);
  }, [run.status]);

  async function download() {
    setRun({ status: "downloading" });
    setProgress(null);
    onBusy(true);
    try {
      await speechApi.download();
      setStatus(await speechApi.status());
      setRun({ status: "idle" });
    } catch (e) {
      setRun({ status: "error", error: toAppError(e) });
    } finally {
      onBusy(false);
    }
  }

  async function transcribe() {
    setRun({ status: "running", started: Date.now() });
    setProgress(null);
    onBusy(true);
    try {
      const t = await speechApi.transcribe(selectionId);
      setRun({ status: "done", t });
      onActions(t.actions);
    } catch (e) {
      setRun({ status: "error", error: toAppError(e) });
    } finally {
      onBusy(false);
    }
  }

  if (!status) {
    return (
      <p className="result-status" role="status">
        <Spinner /> Checking speech recognition…
      </p>
    );
  }

  const busy = run.status === "running" || run.status === "downloading";
  const pct = progress?.total ? Math.min(100, (progress.done / progress.total) * 100) : null;

  return (
    <div className="transcribe-panel">
      <p className="muted small">
        <LockIcon width={14} height={14} /> Transcription runs on this computer with {status.modelLabel}. The recording never
        leaves it.
      </p>

      {!status.modelInstalled && run.status !== "downloading" && (
        <div className="callout">
          <p>
            Speech recognition needs a one-time download of the {status.modelLabel} model ({formatSize(status.downloadBytes)})
            from Hugging Face. MyFile checks the file's SHA-256 checksum before using it. After that, it works offline.
          </p>
          <div className="output-bar">
            <button type="button" className="btn btn-primary" onClick={download}>
              Download speech model
            </button>
          </div>
        </div>
      )}

      {status.modelInstalled && run.status !== "done" && (
        <div className="output-bar">
          <button type="button" className="btn btn-primary" onClick={transcribe} disabled={busy}>
            Transcribe
          </button>
        </div>
      )}

      {busy && (
        <div className="folder-progress" role="status">
          <div className="progress-head">
            <Spinner />
            <span>
              {run.status === "downloading"
                ? progress
                  ? `Downloading the speech model: ${formatSize(progress.done)} of ${formatSize(progress.total ?? status.downloadBytes)}`
                  : "Starting the download…"
                : progress && progress.done > 0
                  ? `Transcribed ${clock(progress.done * 1000)}${progress.total ? ` of ${clock(progress.total * 1000)}` : ""}`
                  : "Reading the recording…"}
              {run.status === "running" && ` · ${Math.round((now - run.started) / 1000)} s`}
            </span>
            <button type="button" className="btn btn-secondary" onClick={() => speechApi.cancel()}>
              Cancel
            </button>
          </div>
          {pct !== null && (
            <div className="progress-bar" aria-hidden="true">
              <span style={{ width: `${pct}%` }} />
            </div>
          )}
        </div>
      )}

      {run.status === "error" && <ErrorPanel title="Transcription didn't complete" error={run.error} />}
      {run.status === "done" && <TranscriptView t={run.t} onAsk={onAsk} onTemplate={onTemplate} />}
    </div>
  );
}

function TranscriptView({ t, onAsk, onTemplate }: { t: Transcript; onAsk: (task: AiTask) => void; onTemplate: () => void }) {
  const [query, setQuery] = useState("");
  const shown = useMemo(() => {
    const q = query.trim().toLowerCase();
    return q ? t.segments.filter((s) => s.text.toLowerCase().includes(q)) : t.segments;
  }, [query, t.segments]);

  return (
    <div className="transcript">
      <p className="success-line" role="status">
        {clock(t.durationMs)} of audio → {t.words.toLocaleString()} words in{" "}
        {(t.elapsedMs / 1000).toFixed(1)} s · {t.model}
        {t.decoder === "ffmpeg" ? " · decoded by ffmpeg" : ""} · free, on this computer
      </p>
      <p className="muted small">Machine transcription: check names, numbers, and dates against the recording.</p>

      <div className="output-bar">
        <button type="button" className="btn btn-primary" onClick={() => onAsk("summarize")}>
          Summarize it
        </button>
        <button type="button" className="btn btn-secondary" onClick={() => onAsk("deadlines")}>
          Find deadlines &amp; dates
        </button>
        <button type="button" className="btn btn-secondary" onClick={() => onAsk("decisions")}>
          Action items
        </button>
        <button type="button" className="btn btn-secondary" onClick={onTemplate}>
          Fill a template
        </button>
      </div>

      <input
        className="text-input transcript-search"
        type="search"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        placeholder="Search the transcript"
        aria-label="Search the transcript"
      />
      <ol className="transcript-lines">
        {shown.map((s) => (
          <li key={s.startMs}>
            <span className="transcript-time">{clock(s.startMs)}</span>
            <span>{s.text}</span>
          </li>
        ))}
        {shown.length === 0 && <li className="muted">No lines contain “{query}”.</li>}
      </ol>

      <p className="field-label">Transcript with timestamps (.txt)</p>
      <OutputButtons outputId={t.textOutputId} copyable />
      <p className="field-label">Subtitles for the video (.srt for most players, .vtt for the web)</p>
      <SubtitleButtons srtId={t.srtOutputId} vttId={t.vttOutputId} />
    </div>
  );
}

function SubtitleButtons({ srtId, vttId }: { srtId: number; vttId: number }) {
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null);
  const save = async (id: number) => {
    try {
      const name = await api.saveOutput(id);
      if (name) setMsg({ ok: true, text: `Saved as “${name}”.` });
    } catch (e) {
      setMsg({ ok: false, text: toAppError(e).message });
    }
  };
  return (
    <div className="output-bar">
      <button type="button" className="btn btn-secondary" onClick={() => save(srtId)}>
        <SaveIcon width={16} height={16} /> Save .srt
      </button>
      <button type="button" className="btn btn-secondary" onClick={() => save(vttId)}>
        <SaveIcon width={16} height={16} /> Save .vtt
      </button>
      {msg && (
        <p className={`feedback ${msg.ok ? "is-ok" : "is-bad"}`} role="status">
          {msg.text}
        </p>
      )}
    </div>
  );
}
