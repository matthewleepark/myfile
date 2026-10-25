import { useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import { api, compareApi, toAppError, type AppError, type Comparison, type Explanation } from "../api";
import { AlertIcon, CalendarIcon, SparkIcon } from "../icons";
import { ErrorPanel, Spinner } from "./common";

interface Props {
  selectionId: number;
  fileName: string;
  onBusy: (busy: boolean) => void;
}

/**
 * Compare two versions. Code computes every difference (and every changed
 * date); the AI only explains them, citing change numbers.
 */
export function ComparePanel({ selectionId, fileName, onBusy }: Props) {
  const [otherPath, setOtherPath] = useState<string | null>(null);
  const [state, setState] = useState<
    { status: "idle" } | { status: "loading" } | { status: "done"; c: Comparison } | { status: "error"; error: AppError }
  >({ status: "idle" });

  async function run(path: string, currentIsNew?: boolean) {
    setOtherPath(path);
    setState({ status: "loading" });
    onBusy(true);
    try {
      setState({ status: "done", c: await compareApi.compare(selectionId, path, currentIsNew ?? null) });
    } catch (e) {
      setState({ status: "error", error: toAppError(e) });
    } finally {
      onBusy(false);
    }
  }

  async function pick() {
    const picked = await open({ multiple: false, directory: false, title: `Choose the other version of ${fileName}` });
    if (typeof picked === "string") run(picked);
  }

  return (
    <>
      {state.status !== "done" && state.status !== "loading" && (
        <div className="ai-run-row">
          <button type="button" className="btn btn-primary" onClick={pick}>
            Choose the other version…
          </button>
          <span className="muted small">Both files stay on this computer for the comparison.</span>
        </div>
      )}
      {state.status === "loading" && (
        <p className="result-status" role="status">
          <Spinner /> Comparing the two versions…
        </p>
      )}
      {state.status === "error" && <ErrorPanel title="Couldn't compare those files" error={state.error} />}
      {state.status === "done" && (
        <CompareResult c={state.c} onSwap={() => run(otherPath!, !state.c.currentIsNew)} onPickAnother={pick} />
      )}
    </>
  );
}

export function CompareResult({ c, onSwap, onPickAnother }: { c: Comparison; onSwap?: () => void; onPickAnother?: () => void }) {
  const d = c.diff;
  const dateChanges = d.changes.filter((x) => x.dateChange);
  const [highlight, setHighlight] = useState<number | null>(null);
  const refs = useRef<Record<number, HTMLLIElement | null>>({});
  const jump = (id: number) => {
    setHighlight(id);
    refs.current[id]?.scrollIntoView({ behavior: "smooth", block: "center" });
  };

  return (
    <div className="compare-result">
      <p className="compare-head">
        <span>
          <span className="muted">Older:</span> <strong>{c.oldName}</strong> <span aria-hidden="true">→</span>{" "}
          <span className="muted">Newer:</span> <strong>{c.newName}</strong>
        </span>
        {onSwap && (
          <button type="button" className="btn btn-secondary" onClick={onSwap}>
            Swap older/newer
          </button>
        )}
        {onPickAnother && (
          <button type="button" className="btn btn-secondary" onClick={onPickAnother}>
            Pick another file
          </button>
        )}
      </p>
      {c.ocr && <p className="muted small">At least one version was read with OCR, so tiny differences may be misreads.</p>}

      <div className="stat-row">
        <div className="stat">
          <span className="stat-value">{d.changed}</span>
          <span className="stat-label">changed</span>
        </div>
        <div className="stat">
          <span className="stat-value">{d.added}</span>
          <span className="stat-label">added</span>
        </div>
        <div className="stat">
          <span className="stat-value">{d.removed}</span>
          <span className="stat-label">removed</span>
        </div>
        <div className="stat">
          <span className="stat-value">{d.unchanged}</span>
          <span className="stat-label">unchanged sentences</span>
        </div>
      </div>

      {d.changes.length === 0 ? (
        <p className="success-line">The two versions have the same text.</p>
      ) : (
        <>
          {dateChanges.length > 0 && (
            <div className="date-changes">
              <p className="checks-title">
                <CalendarIcon width={14} height={14} /> Dates that changed (found by MyFile's code, not the AI)
              </p>
              <ul>
                {dateChanges.map((x) => (
                  <li key={x.id}>
                    <button type="button" className="link-button" onClick={() => jump(x.id)}>
                      Change {x.id}
                    </button>
                    : {x.dateChange}
                  </li>
                ))}
              </ul>
            </div>
          )}

          <ExplainBox c={c} onJump={jump} />

          <h4 className="tl-month-title">Every change</h4>
          <ol className="change-list">
            {d.changes.map((x) => (
              <li
                key={x.id}
                ref={(el) => {
                  refs.current[x.id] = el;
                }}
                className={`change change-${x.kind}${highlight === x.id ? " is-highlighted" : ""}`}
              >
                <span className="change-id">{x.id}</span>
                <span className="change-body">
                  <span className="change-meta">
                    <span className={`tag change-tag-${x.kind}`}>{x.kind}</span>
                    {x.beforePage != null && <span className="muted small">old p. {x.beforePage}</span>}
                    {x.afterPage != null && <span className="muted small">new p. {x.afterPage}</span>}
                    {x.dateChange && <span className="tag tag-warn">date: {x.dateChange}</span>}
                  </span>
                  {x.kind === "changed" ? (
                    <span className="change-text">
                      {x.words.map((w, i) => (
                        <span key={i} className={`w-${w.kind}`}>
                          {w.text}
                        </span>
                      ))}
                    </span>
                  ) : (
                    <span className="change-text">
                      <span className={x.kind === "added" ? "w-added" : "w-removed"}>{x.after ?? x.before}</span>
                    </span>
                  )}
                </span>
              </li>
            ))}
          </ol>
          {d.hiddenChanges > 0 && <p className="muted small">{d.hiddenChanges} more changes aren't listed.</p>}
        </>
      )}
    </div>
  );
}

function ExplainBox({ c, onJump }: { c: Comparison; onJump: (id: number) => void }) {
  const [provider, setProvider] = useState<"local" | "claude">("claude");
  const [localModel, setLocalModel] = useState<string | null>(null);
  // Until we know whether a local model exists, the default isn't settled, so
  // Explain stays disabled (otherwise a click could go to the wrong provider).
  const [providerReady, setProviderReady] = useState(false);
  const [consented, setConsented] = useState(false);
  const [asking, setAsking] = useState(false);
  const [state, setState] = useState<
    { status: "idle" } | { status: "running" } | { status: "done"; e: Explanation } | { status: "error"; error: AppError }
  >({ status: "idle" });

  useEffect(() => {
    api
      .localStatus()
      .then((l) => {
        if (l.available) {
          setLocalModel(l.model);
          setProvider("local");
        }
      }, () => {})
      .finally(() => setProviderReady(true));
  }, []);

  async function explain() {
    if (provider === "claude" && !consented) {
      setAsking(true);
      return;
    }
    setState({ status: "running" });
    try {
      setState({ status: "done", e: await compareApi.explain(c.comparisonId, provider) });
    } catch (e) {
      setState({ status: "error", error: toAppError(e) });
    }
  }

  return (
    <div className="explain-box">
      <div className="ai-run-row">
        {localModel && (
          <div className="segmented" role="radiogroup" aria-label="Which model explains the changes?">
            {(
              [
                ["local", `Local · ${localModel}`],
                ["claude", "Claude"],
              ] as ["local" | "claude", string][]
            ).map(([id, label]) => (
              <button
                key={id}
                type="button"
                role="radio"
                aria-checked={provider === id}
                className={`segment${provider === id ? " is-selected" : ""}`}
                onClick={() => setProvider(id)}
                disabled={state.status === "running"}
              >
                {label}
              </button>
            ))}
          </div>
        )}
        <button type="button" className="btn btn-primary" onClick={explain} disabled={!providerReady || state.status === "running"}>
          <SparkIcon width={16} height={16} /> Explain what changed
        </button>
        <span className="muted small">
          {provider === "local"
            ? "Runs on this computer; nothing is sent."
            : `Sends only the changed passages (${c.explainChars.toLocaleString()} characters), not the whole documents.`}
        </span>
      </div>
      {asking && (
        <ConsentInline
          chars={c.explainChars}
          onAccept={async () => {
            setAsking(false);
            try {
              await compareApi.grantConsent(c.comparisonId);
              setConsented(true);
              setState({ status: "running" });
              setState({ status: "done", e: await compareApi.explain(c.comparisonId, "claude") });
            } catch (e) {
              setState({ status: "error", error: toAppError(e) });
            }
          }}
          onCancel={() => setAsking(false)}
        />
      )}
      {state.status === "running" && (
        <p className="result-status" role="status">
          <Spinner /> {provider === "local" ? "The local model is reading the changes…" : "Claude is reading the changes…"}
        </p>
      )}
      {state.status === "error" && <ErrorPanel title="Couldn't explain the changes" error={state.error} />}
      {state.status === "done" && (
        <div className="explanation">
          <p className="answer-text">{state.e.summary}</p>
          <ul className="ai-items">
            {state.e.points.map((p, i) => (
              <li key={i} className="ai-item">
                <div className="ai-item-head">
                  <span className={`tag ${p.importance === "high" ? "tag-warn" : ""}`}>{p.importance}</span>
                  <span className="ai-item-title">{p.title}</span>
                </div>
                {p.detail && <p className="ai-item-detail">{p.detail}</p>}
                <p className="small">
                  Based on{" "}
                  {p.changes.map((id, k) => (
                    <span key={id}>
                      {k > 0 && ", "}
                      <button type="button" className="link-button" onClick={() => onJump(id)}>
                        change {id}
                      </button>
                    </span>
                  ))}
                </p>
              </li>
            ))}
          </ul>
          {state.e.notes.length > 0 && (
            <div className="checks" role="note">
              <p className="checks-title">MyFile's checks on this explanation</p>
              <ul>
                {state.e.notes.map((n, i) => (
                  <li key={i}>{n}</li>
                ))}
              </ul>
              {state.e.unexplainedChanges.length > 0 && (
                <p className="small">
                  Not explained:{" "}
                  {state.e.unexplainedChanges.map((id, k) => (
                    <span key={id}>
                      {k > 0 && ", "}
                      <button type="button" className="link-button" onClick={() => onJump(id)}>
                        change {id}
                      </button>
                    </span>
                  ))}
                </p>
              )}
              {state.e.unmentionedDateChanges.length > 0 && (
                <p className="small">
                  <AlertIcon width={12} height={12} /> Not mentioned:{" "}
                  {state.e.unmentionedDateChanges.map((id, k) => (
                    <span key={id}>
                      {k > 0 && ", "}
                      <button type="button" className="link-button" onClick={() => onJump(id)}>
                        change {id}
                      </button>
                    </span>
                  ))}
                </p>
              )}
            </div>
          )}
          <p className="muted small ai-meta">
            Explained by {state.e.model} in {(state.e.durationMs / 1000).toFixed(1)} s ·{" "}
            {state.e.model.endsWith("(local)") ? "free, on this computer" : `≈$${state.e.usage.costUsd.toFixed(4)} at API list price`}
          </p>
        </div>
      )}
    </div>
  );
}

function ConsentInline({ chars, onAccept, onCancel }: { chars: number; onAccept: () => void; onCancel: () => void }) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    if (ref.current && !ref.current.open) ref.current.showModal();
  }, []);
  return (
    <dialog
      ref={ref}
      className="dialog"
      aria-labelledby="cmp-consent"
      onCancel={(e) => {
        e.preventDefault();
        onCancel();
      }}
    >
      <h2 id="cmp-consent">Send the changes to Claude?</h2>
      <p>
        MyFile will send only the changed passages from both versions ({chars.toLocaleString()} characters) to{" "}
        <strong>Anthropic's Claude</strong>, not the whole documents.
      </p>
      <div className="dialog-buttons">
        <button type="button" className="btn btn-secondary" onClick={onCancel}>
          Cancel
        </button>
        <button type="button" className="btn btn-primary" onClick={onAccept}>
          Send to Claude
        </button>
      </div>
    </dialog>
  );
}
