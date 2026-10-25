import { useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";

import {
  convertApi,
  toAppError,
  type AppError,
  type ConversionChoices,
  type ConversionResult,
  type ConvertSettings,
  type ConvertTarget,
  type Suggestion,
} from "../api";
import { formatSize } from "../format";
import { CheckIcon, SaveIcon, SparkIcon } from "../icons";
import { ErrorPanel, Spinner } from "./common";

const SIZES = [3840, 2560, 1920, 1280, 800, 480];
const BITRATES = [64, 96, 128, 192, 256, 320];

interface Props {
  selectionId: number;
  onBusy: (busy: boolean) => void;
}

/** The main screen for a file: what it can become, settings, convert, save. */
export function ConvertPanel({ selectionId, onBusy }: Props) {
  const [choices, setChoices] = useState<ConversionChoices | null>(null);
  const [loadError, setLoadError] = useState<AppError | null>(null);
  const [target, setTarget] = useState<ConvertTarget | null>(null);
  const [settings, setSettings] = useState<ConvertSettings>({ quality: null, maxDimension: null, bitrate: null });
  const [run, setRun] = useState<
    { status: "idle" } | { status: "running"; fraction: number | null } | { status: "done"; r: ConversionResult; label: string } | { status: "error"; error: AppError }
  >({ status: "idle" });
  const [saved, setSaved] = useState<{ ok: boolean; text: string } | null>(null);

  useEffect(() => {
    convertApi.choices(selectionId).then(setChoices, (e) => setLoadError(toAppError(e)));
    const off = listen<{ fraction: number | null }>("convert-progress", (e) =>
      setRun((r) => (r.status === "running" ? { status: "running", fraction: e.payload.fraction } : r)),
    );
    return () => {
      off.then((f) => f());
    };
  }, [selectionId]);

  const groups = useMemo(() => {
    const m = new Map<string, ConvertTarget[]>();
    for (const t of choices?.targets.filter((t) => t.available) ?? []) {
      m.set(t.group, [...(m.get(t.group) ?? []), t]);
    }
    return [...m.entries()];
  }, [choices]);

  function pick(t: ConvertTarget, preset?: Partial<ConvertSettings>) {
    setTarget(t);
    setSettings({ quality: null, maxDimension: null, bitrate: null, ...preset });
    setRun({ status: "idle" });
    setSaved(null);
  }

  async function convert() {
    if (!target) return;
    setRun({ status: "running", fraction: null });
    setSaved(null);
    onBusy(true);
    try {
      const r = await convertApi.convert(selectionId, target.id, settings);
      setRun({ status: "done", r, label: target.label });
    } catch (e) {
      const err = toAppError(e);
      setRun(err.code === "cancelled" ? { status: "idle" } : { status: "error", error: err });
    } finally {
      onBusy(false);
    }
  }

  async function save(r: ConversionResult) {
    try {
      const s = await convertApi.save(r.conversionId);
      if (s) setSaved({ ok: true, text: s.count === 1 ? `Saved as “${s.name}”.` : `Saved ${s.count} files in “${s.name}”.` });
    } catch (e) {
      setSaved({ ok: false, text: toAppError(e).message });
    }
  }

  if (loadError) return <ErrorPanel title="Couldn't check this file" error={loadError} />;
  if (!choices)
    return (
      <p className="result-status" role="status">
        <Spinner /> Checking what this file can become…
      </p>
    );
  if (!choices.format || choices.targets.length === 0) {
    return (
      <section className="panel convert-panel">
        <p>MyFile doesn't have a conversion for this kind of file yet.</p>
      </section>
    );
  }

  const running = run.status === "running";
  return (
    <section className="panel convert-panel" aria-label="Convert">
      <h3 className="section-title">Convert this {choices.formatLabel} to</h3>

      <Describe selectionId={selectionId} disabled={running} onPick={(s) => {
        const t = choices.targets.find((x) => x.id === s.target && x.available);
        if (t) pick(t, { quality: s.quality, maxDimension: s.maxDimension, bitrate: s.bitrate });
      }} />

      {groups.map(([group, list]) => (
        <div key={group} className="format-group">
          <span className="format-group-label">{group}</span>
          <div className="format-grid" role="radiogroup" aria-label={`${group} formats`}>
            {list.map((t) => (
              <button
                key={t.id}
                type="button"
                role="radio"
                aria-checked={target?.id === t.id}
                className={`format-chip${target?.id === t.id ? " is-selected" : ""}`}
                onClick={() => pick(t)}
                disabled={running}
                title={t.hint ?? undefined}
              >
                <span className="format-chip-label">{t.label}</span>
                {t.hint && <span className="format-chip-hint">{t.hint}</span>}
              </button>
            ))}
          </div>
        </div>
      ))}
      {choices.missingTools.length > 0 && (
        <p className="muted small">
          More formats for this file need {choices.missingTools.join(" or ")} (
          {choices.targets.filter((t) => !t.available).map((t) => t.label).join(", ")}).
        </p>
      )}

      {target && (
        <div className="convert-settings">
          {target.knobs.includes("quality") && (
            <label className="setting">
              <span>Quality</span>
              <input
                type="range"
                min={30}
                max={95}
                value={settings.quality ?? 80}
                onChange={(e) => setSettings((s) => ({ ...s, quality: Number(e.target.value) }))}
                disabled={running}
              />
              <span className="setting-value">{settings.quality ?? 80}</span>
            </label>
          )}
          {target.knobs.includes("size") && (
            <label className="setting">
              <span>Size</span>
              <select
                value={settings.maxDimension ?? ""}
                onChange={(e) => setSettings((s) => ({ ...s, maxDimension: e.target.value ? Number(e.target.value) : null }))}
                disabled={running}
              >
                <option value="">Original</option>
                {SIZES.map((n) => (
                  <option key={n} value={n}>
                    Up to {n}px
                  </option>
                ))}
              </select>
            </label>
          )}
          {target.knobs.includes("bitrate") && (
            <label className="setting">
              <span>Bitrate</span>
              <select
                value={settings.bitrate ?? ""}
                onChange={(e) => setSettings((s) => ({ ...s, bitrate: e.target.value ? Number(e.target.value) : null }))}
                disabled={running}
              >
                <option value="">Standard</option>
                {BITRATES.map((n) => (
                  <option key={n} value={n}>
                    {n} kbit/s
                  </option>
                ))}
              </select>
            </label>
          )}
          <button type="button" className="btn btn-primary" onClick={convert} disabled={running}>
            Convert to {target.label}
          </button>
        </div>
      )}

      {run.status === "running" && (
        <div className="folder-progress" role="status">
          <div className="progress-head">
            <Spinner />
            <span>Converting{run.fraction !== null ? ` · ${Math.round(run.fraction * 100)}%` : "…"}</span>
            <button type="button" className="btn btn-secondary" onClick={() => convertApi.cancel()}>
              Cancel
            </button>
          </div>
          {run.fraction !== null && (
            <div className="progress-bar" aria-hidden="true">
              <span style={{ width: `${run.fraction * 100}%` }} />
            </div>
          )}
        </div>
      )}
      {run.status === "error" && <ErrorPanel title="That conversion didn't work" error={run.error} />}
      {run.status === "done" && (
        <div className="convert-result">
          {run.r.preview && <img className="convert-preview" src={run.r.preview} alt="Preview of the result" />}
          <div className="convert-result-body">
            <p className="success-line" role="status">
              {run.r.files.length === 1 ? run.r.files[0].name : `${run.r.files.length} files`} · {formatSize(run.r.totalBytes)}
              {run.r.files.length === 1 && run.r.totalBytes < run.r.originalBytes && (
                <> ({Math.round((1 - run.r.totalBytes / run.r.originalBytes) * 100)}% smaller)</>
              )}
            </p>
            {run.r.files.length > 1 && (
              <details className="muted small">
                <summary>Show the files</summary>
                <ul className="convert-files">
                  {run.r.files.map((f) => (
                    <li key={f.name}>
                      {f.name} · {formatSize(f.sizeBytes)}
                    </li>
                  ))}
                </ul>
              </details>
            )}
            <div className="output-bar">
              <button type="button" className="btn btn-primary" onClick={() => save(run.r)}>
                <SaveIcon width={16} height={16} /> {run.r.files.length === 1 ? "Save…" : "Save to a folder…"}
              </button>
            </div>
            {saved && (
              <p className={`feedback ${saved.ok ? "is-ok" : "is-bad"}`} role="status">
                {saved.ok && <CheckIcon width={16} height={16} />} {saved.text}
              </p>
            )}
            <p className="muted small">Your original file isn't changed.</p>
          </div>
        </div>
      )}
    </section>
  );
}

/** "Describe it": the AI picks the conversion and settings from a sentence. */
function Describe({ selectionId, disabled, onPick }: { selectionId: number; disabled: boolean; onPick: (s: Suggestion) => void }) {
  const [text, setText] = useState("");
  const [state, setState] = useState<{ status: "idle" } | { status: "asking" } | { status: "done"; s: Suggestion } | { status: "error"; error: AppError }>({
    status: "idle",
  });

  async function ask() {
    if (!text.trim()) return;
    setState({ status: "asking" });
    try {
      const s = await convertApi.suggest(selectionId, text);
      setState({ status: "done", s });
      if (s.target) onPick(s);
    } catch (e) {
      setState({ status: "error", error: toAppError(e) });
    }
  }

  return (
    <div className="describe">
      <form
        className="search-row"
        onSubmit={(e) => {
          e.preventDefault();
          ask();
        }}
      >
        <input
          className="text-input"
          value={text}
          onChange={(e) => setText(e.target.value)}
          placeholder="Or describe it, e.g. “small enough to email” or “a PDF to print”"
          aria-label="Describe the conversion you want"
          maxLength={300}
          disabled={disabled}
        />
        <button type="submit" className="btn btn-secondary" disabled={disabled || !text.trim() || state.status === "asking"}>
          {state.status === "asking" ? <Spinner /> : <SparkIcon width={16} height={16} />} Suggest
        </button>
      </form>
      {state.status === "done" && (
        <p className="muted small" role="status">
          {state.s.target ? (
            <>
              <strong>{state.s.target.toUpperCase()}</strong>
              {state.s.quality !== null && ` · quality ${state.s.quality}`}
              {state.s.maxDimension !== null && ` · up to ${state.s.maxDimension}px`}
              {state.s.bitrate !== null && ` · ${state.s.bitrate} kbit/s`}: {state.s.reason}
            </>
          ) : (
            <>No conversion fits that. {state.s.reason}</>
          )}
          {state.s.adjusted.length > 0 && <> ({state.s.adjusted.join(" ")})</>}
          <span className="ai-meta"> · picked by {state.s.model}; only your words and the file type were used</span>
        </p>
      )}
      {state.status === "error" && <ErrorPanel title="Couldn't suggest a conversion" error={state.error} />}
    </div>
  );
}
