import { useEffect, useRef, useState } from "react";

import { api, toAppError, type AppError, type CompressOptions, type CompressResult } from "../api";
import { formatSize } from "../format";
import { ErrorPanel, OutputButtons, Spinner } from "./common";

const SIZES = [3840, 2560, 1920, 1280, 800];

interface Props {
  onBusy: (busy: boolean) => void;
  /** Settings chosen by a typed request ("Ask MyFile"), already validated by Rust. */
  initial?: CompressOptions | null;
  autoRun?: boolean;
}

export function ImagePanel({ onBusy, initial, autoRun }: Props) {
  const [format, setFormat] = useState<CompressOptions["format"]>(initial?.format ?? "jpeg");
  const [quality, setQuality] = useState(initial?.quality ?? 75);
  const [resize, setResize] = useState(initial?.maxDimension != null);
  const [maxDimension, setMaxDimension] = useState(initial?.maxDimension ?? 1920);
  const autoRan = useRef(false);

  useEffect(() => {
    if (autoRun && !autoRan.current) {
      autoRan.current = true;
      run();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [autoRun]);
  const [state, setState] = useState<
    | { status: "idle" }
    | { status: "running" }
    | { status: "done"; result: CompressResult }
    | { status: "error"; error: AppError }
  >({ status: "idle" });

  async function run() {
    setState({ status: "running" });
    onBusy(true);
    try {
      const result = await api.compressImage({ format, quality, maxDimension: resize ? maxDimension : null });
      setState({ status: "done", result });
    } catch (e) {
      setState({ status: "error", error: toAppError(e) });
    } finally {
      onBusy(false);
    }
  }

  const running = state.status === "running";

  return (
    <>
      <form
        className="options"
        onSubmit={(e) => {
          e.preventDefault();
          run();
        }}
      >
        <fieldset className="option-group" disabled={running}>
          <legend>Output format</legend>
          <label className="radio">
            <input type="radio" name="format" checked={format === "jpeg"} onChange={() => setFormat("jpeg")} />
            <span>
              <strong>JPEG</strong>: smallest files, best for photos. Transparent areas become white.
            </span>
          </label>
          <label className="radio">
            <input type="radio" name="format" checked={format === "png"} onChange={() => setFormat("png")} />
            <span>
              <strong>PNG</strong>: lossless, keeps transparency. Best for screenshots and graphics.
            </span>
          </label>
        </fieldset>

        {format === "jpeg" && (
          <fieldset className="option-group" disabled={running}>
            <legend>Quality</legend>
            <div className="slider-row">
              <input
                type="range"
                min={30}
                max={95}
                step={5}
                value={quality}
                onChange={(e) => setQuality(Number(e.target.value))}
                aria-label="JPEG quality"
                aria-valuetext={`${quality} out of 100`}
              />
              <output className="slider-value">{quality}</output>
            </div>
            <p className="muted small">Lower numbers give smaller files with more visible blur. 70–80 suits most photos.</p>
          </fieldset>
        )}

        <fieldset className="option-group" disabled={running}>
          <legend>Size</legend>
          <label className="checkbox">
            <input type="checkbox" checked={resize} onChange={(e) => setResize(e.target.checked)} />
            <span>Scale down so the longest side is at most</span>
            <select
              value={maxDimension}
              onChange={(e) => setMaxDimension(Number(e.target.value))}
              disabled={!resize}
              aria-label="Maximum size in pixels"
            >
              {SIZES.map((s) => (
                <option key={s} value={s}>
                  {s} px
                </option>
              ))}
            </select>
          </label>
          <p className="muted small">Smaller images are never enlarged. The proportions stay the same.</p>
        </fieldset>

        <div>
          <button type="submit" className="btn btn-primary" disabled={running}>
            {running ? "Compressing…" : "Compress image"}
          </button>
        </div>
      </form>

      {running && (
        <p className="result-status" role="status">
          <Spinner /> Compressing on this computer…
        </p>
      )}
      {state.status === "error" && <ErrorPanel title="Couldn't compress the image" error={state.error} />}
      {state.status === "done" && <CompressSummary result={state.result} />}
    </>
  );
}

function CompressSummary({ result: r }: { result: CompressResult }) {
  const change = r.outputBytes - r.originalBytes;
  const percent = Math.abs(change / r.originalBytes) * 100;
  const resized = r.width !== r.originalWidth || r.height !== r.originalHeight;

  return (
    <div className="compress-result">
      <img className="compress-preview" src={r.previewDataUrl} alt="Preview of the compressed image" />
      <div className="compress-facts">
        {change < 0 ? (
          <p className="success-line" role="status">
            {percent < 0.1 ? "Slightly smaller" : `${percent.toFixed(1)}% smaller`}: {formatSize(r.originalBytes)} →{" "}
            {formatSize(r.outputBytes)}
          </p>
        ) : (
          <p className="warn-line" role="status">
            The result is {change === 0 ? "the same size" : `${percent.toFixed(1)}% larger`} ({formatSize(r.originalBytes)} →{" "}
            {formatSize(r.outputBytes)}). The original is probably already well compressed. Try JPEG, a lower quality, or scaling down.
          </p>
        )}
        <dl className="facts">
          <dt>Dimensions</dt>
          <dd>
            {resized ? `${r.originalWidth} × ${r.originalHeight} → ` : ""}
            {r.width} × {r.height} px
          </dd>
          <dt>New file</dt>
          <dd>{r.fileName}</dd>
        </dl>
        <p className="muted small">
          Your original file hasn't been changed. Photo metadata, such as camera details and GPS location, isn't copied
          into the new file.
        </p>
        <OutputButtons outputId={r.outputId} copyable={false} />
      </div>
    </div>
  );
}
