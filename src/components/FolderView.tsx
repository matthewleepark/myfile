import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";

import {
  api,
  factCheckApi,
  folderApi,
  toAppError,
  type AppError,
  type AskResult,
  type FactCheck,
  type FolderAnalysis,
  type FolderProgress,
  type FolderSummary,
  type TimelineEntry,
} from "../api";
import { AlertIcon, CalendarIcon, CheckIcon, CloseIcon, LockIcon, SparkIcon } from "../icons";
import { ErrorPanel, Spinner } from "./common";
import { AgentPanel } from "./AgentPanel";
import { FactCheckView } from "./FactCheckView";
import { EvidenceButton } from "./EvidenceViewer";
import { TemplatePanel } from "./TemplatePanel";
import { DuplicatesPanel } from "./DuplicatesPanel";

interface Props {
  folder: FolderSummary;
  onClose: () => void;
}

/** A stable, distinct color per document so the timeline is easy to scan. */
const docColor = (i: number) => `hsl(${(i * 67) % 360} 70% 62%)`;

const MONTHS = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const prettyDate = (iso: string) => {
  const [y, m, d] = iso.split("-").map(Number);
  const weekday = new Date(Date.UTC(y, m - 1, d)).toLocaleDateString("en-US", { weekday: "short", timeZone: "UTC" });
  return `${weekday}, ${MONTHS[m - 1].slice(0, 3)} ${d}`;
};

type Run =
  | { status: "idle" }
  | { status: "consent" }
  | { status: "running"; startedAt: number }
  | { status: "done"; result: FolderAnalysis }
  | { status: "error"; error: AppError };

export function FolderView({ folder, onClose }: Props) {
  const [run, setRun] = useState<Run>({ status: "idle" });
  const [progress, setProgress] = useState<FolderProgress | null>(null);
  const [consented, setConsented] = useState(false);
  // Local model by default where installed; Claude is the optional upgrade.
  const [provider, setProvider] = useState<"local" | "claude">("claude");
  const [localModel, setLocalModel] = useState<string | null>(null);
  const [providerReady, setProviderReady] = useState(false);
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
  const needsConsent = provider === "claude" && !consented;
  const [elapsed, setElapsed] = useState(0);
  const usable = folder.docs.filter((d) => !d.error);

  useEffect(() => {
    const un = listen<FolderProgress>("folder-progress", (e) => setProgress(e.payload));
    return () => {
      un.then((f) => f());
    };
  }, []);

  useEffect(() => {
    if (run.status !== "running") return;
    const t = setInterval(() => setElapsed(Math.round((Date.now() - run.startedAt) / 1000)), 500);
    return () => clearInterval(t);
  }, [run]);

  // Stop in-flight requests if this view goes away.
  const runningRef = useRef(false);
  runningRef.current = run.status === "running";
  useEffect(() => () => void (runningRef.current && folderApi.cancel()), []);

  async function analyze() {
    setProgress(null);
    setElapsed(0);
    setRun({ status: "running", startedAt: Date.now() });
    try {
      setRun({ status: "done", result: await folderApi.analyze(folder.selectionId, provider) });
    } catch (e) {
      setRun({ status: "error", error: toAppError(e) });
    }
  }

  async function accept() {
    try {
      await folderApi.grantConsent(folder.selectionId);
      setConsented(true);
      await analyze();
    } catch (e) {
      setRun({ status: "error", error: toAppError(e) });
    }
  }

  const perDoc = run.status === "done" ? run.result.perDoc : null;

  return (
    <>
      <section className="file-card" aria-label="Open folder">
        <div className="file-badge kind-pdf">
          <SparkIcon width={26} height={26} />
        </div>
        <div className="file-meta">
          <h2 className="file-name">{folder.name}</h2>
          <p className="file-details">
            Semester brain · {usable.length} document{usable.length === 1 ? "" : "s"}
            {folder.images.length > 0 && ` · ${folder.images.length} image${folder.images.length === 1 ? "" : "s"}`} ·{" "}
            {folder.totalChars.toLocaleString()} characters
          </p>
        </div>
        <div className="file-buttons">
          <button type="button" className="btn btn-icon" onClick={onClose} aria-label="Close folder" title="Close folder">
            <CloseIcon />
          </button>
        </div>
      </section>

      <section className="panel" aria-label="Documents">
        <h3 className="section-title">Documents in this folder</h3>
        <ul className="doc-list">
          {folder.docs.map((d) => {
            const r = perDoc?.find((p) => p.index === d.index);
            return (
              <li key={d.index} className="doc-row">
                <span className="doc-dot" style={{ background: docColor(d.index) }} aria-hidden="true" />
                <span className="doc-name" title={d.name}>
                  {d.name}
                </span>
                <span className="muted small">
                  {d.pages !== null ? `${d.pages} p · ` : ""}
                  {d.chars.toLocaleString()} chars
                </span>
                <span className="doc-status small">
                  {d.error ? (
                    <span className="warn-text">
                      <AlertIcon width={14} height={14} /> {d.error}
                    </span>
                  ) : r ? (
                    r.error ? (
                      <span className="warn-text">
                        <AlertIcon width={14} height={14} /> {r.error}
                      </span>
                    ) : (
                      <span className="ok-text">
                        <CheckIcon width={14} height={14} /> {r.items} item{r.items === 1 ? "" : "s"} · {r.quotesVerified}/
                        {r.quotesChecked} quotes verified
                      </span>
                    )
                  ) : (
                    <span className="muted">Ready</span>
                  )}
                </span>
              </li>
            );
          })}
        </ul>
        {folder.skipped.length > 0 && (
          <p className="muted small">
            Skipped {folder.skipped.length}: {folder.skipped.slice(0, 4).join("; ")}
            {folder.skipped.length > 4 ? "…" : ""}
          </p>
        )}

        {run.status !== "running" && localModel && (
          <div className="segmented" role="radiogroup" aria-label="Which model reads the documents?">
            {(
              [
                ["local", `Local · ${localModel} (private, free)`],
                ["claude", "Claude (cloud, more accurate)"],
              ] as ["local" | "claude", string][]
            ).map(([id, label]) => (
              <button
                key={id}
                type="button"
                role="radio"
                aria-checked={provider === id}
                className={`segment${provider === id ? " is-selected" : ""}`}
                onClick={() => setProvider(id)}
              >
                {label}
              </button>
            ))}
          </div>
        )}
        {run.status !== "running" && (
          <div className="ai-run-row">
            <button
              type="button"
              className="btn btn-primary"
              disabled={usable.length === 0 || !providerReady}
              onClick={() => (needsConsent ? setRun({ status: "consent" }) : analyze())}
            >
              <CalendarIcon width={16} height={16} /> {run.status === "done" ? "Rebuild timeline" : "Build my semester timeline"}
            </button>
            <span className="muted small privacy-note">
              <LockIcon width={14} height={14} />
              {provider === "local"
                ? "Local model: the documents stay on this computer."
                : consented
                  ? "You allowed these documents to be sent to Claude."
                  : "Nothing is sent until you confirm."}
            </span>
          </div>
        )}

        {run.status === "running" && (
          <div className="folder-progress" role="status">
            <div className="progress-head">
              <Spinner />
              <span>
                {progress && progress.phase === "analyzing"
                  ? `Claude has read ${progress.done} of ${progress.total} documents`
                  : "Starting…"}
                {progress?.current ? ` · last: ${progress.current}` : ""}
              </span>
              <span className="muted">{elapsed} s</span>
              <button type="button" className="btn btn-secondary" onClick={() => folderApi.cancel()}>
                Cancel
              </button>
            </div>
            <div
              className="progress-bar"
              role="progressbar"
              aria-valuemin={0}
              aria-valuemax={progress?.total ?? usable.length}
              aria-valuenow={progress?.done ?? 0}
            >
              <span style={{ width: `${progress ? (100 * progress.done) / Math.max(1, progress.total) : 0}%` }} />
            </div>
          </div>
        )}
        {run.status === "error" && <ErrorPanel title="The timeline couldn't be built" error={run.error} />}
      </section>

      {run.status === "consent" && (
        <FolderConsent folder={folder} onAccept={accept} onCancel={() => setRun({ status: "idle" })} />
      )}

      <AgentPanel folder={folder} />
      <DuplicatesPanel selectionId={folder.selectionId} />
      <section className="panel" aria-label="Fill a template for every document">
        <h3 className="section-title">Fill a template for every document</h3>
        <TemplatePanel scope="folder" selectionId={folder.selectionId} />
      </section>
      {run.status === "done" && <Timeline result={run.result} selectionId={folder.selectionId} />}
      {(provider === "local" || consented) && <AskAcross folder={folder} provider={provider} />}
    </>
  );
}

function FolderConsent({ folder, onAccept, onCancel }: { folder: FolderSummary; onAccept: () => void; onCancel: () => void }) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    if (ref.current && !ref.current.open) ref.current.showModal();
  }, []);
  const usable = folder.docs.filter((d) => !d.error);
  return (
    <dialog
      ref={ref}
      className="dialog"
      aria-labelledby="folder-consent-title"
      onCancel={(e) => {
        e.preventDefault();
        onCancel();
      }}
    >
      <h2 id="folder-consent-title">Send {usable.length} documents to Claude?</h2>
      <p>
        MyFile will send the text of these documents ({folder.totalChars.toLocaleString()} characters in total) to{" "}
        <strong>Anthropic's Claude</strong>, one document per request, using the Claude Code app signed in on this computer.
      </p>
      <ul className="dialog-list dialog-files">
        {usable.map((d) => (
          <li key={d.index}>
            {d.name} <span className="muted">({d.chars.toLocaleString()} chars)</span>
          </li>
        ))}
      </ul>
      <ul className="dialog-list">
        <li>Only extracted text is sent, never the files or anything else in the folder.</li>
        <li>MyFile keeps nothing after you close the folder. Anthropic's data policies apply to what it receives.</li>
      </ul>
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

function Timeline({ result: r, selectionId }: { result: FolderAnalysis; selectionId: number }) {
  const [saved, setSaved] = useState<string | null>(null);
  const conflictDays = new Set(r.conflicts.map((c) => c.isoDate));
  const dated = r.timeline.filter((e) => e.isoDate);
  const undated = r.timeline.filter((e) => !e.isoDate);
  const byMonth = new Map<string, TimelineEntry[]>();
  for (const e of dated) {
    const key = e.isoDate!.slice(0, 7);
    byMonth.set(key, [...(byMonth.get(key) ?? []), e]);
  }
  const checked = r.perDoc.reduce((s, d) => s + d.quotesChecked, 0);
  const verified = r.perDoc.reduce((s, d) => s + d.quotesVerified, 0);

  return (
    <section className="panel" aria-label="Semester timeline">
      <h3 className="section-title">Your semester</h3>
      <div className="stat-row">
        <div className="stat">
          <span className="stat-value">{dated.length}</span>
          <span className="stat-label">dated items</span>
        </div>
        <div className={`stat ${r.conflicts.length ? "stat-warn" : ""}`}>
          <span className="stat-value">{r.conflicts.length}</span>
          <span className="stat-label">same-day conflicts</span>
        </div>
        <div className={`stat ${r.crunchWeeks.length ? "stat-warn" : ""}`}>
          <span className="stat-value">{r.crunchWeeks.length}</span>
          <span className="stat-label">crunch weeks</span>
        </div>
        <div className="stat">
          <span className="stat-value">
            {verified}/{checked}
          </span>
          <span className="stat-label">quotes verified</span>
        </div>
      </div>

      {r.conflicts.map((c) => (
        <p key={c.isoDate} className="warn-line">
          <AlertIcon width={16} height={16} /> {prettyDate(c.isoDate)}: {c.docs.join(" and ")} both have something due.
        </p>
      ))}
      {r.crunchWeeks.map((w) => (
        <p key={w.weekStart} className="warn-line">
          <AlertIcon width={16} height={16} /> Crunch week of {prettyDate(w.weekStart)}: {w.count} deadlines across{" "}
          {w.docs.length} document{w.docs.length === 1 ? "" : "s"}.
        </p>
      ))}

      {r.calendar && (
        <div className="calendar-export">
          <button
            type="button"
            className="btn btn-primary"
            onClick={async () => {
              const name = await api.saveOutput(r.calendar!.outputId).catch(() => null);
              if (name) setSaved(name);
            }}
          >
            <CalendarIcon width={16} height={16} /> Add all {r.calendar.events} dates to my calendar
          </button>
          <span className="muted small">One .ics file for every class.</span>
          {saved && (
            <p className="feedback is-ok" role="status">
              <CheckIcon width={16} height={16} /> Saved as “{saved}”.
            </p>
          )}
        </div>
      )}

      {[...byMonth.entries()].map(([month, entries]) => (
        <div key={month} className="tl-month">
          <h4 className="tl-month-title">
            {MONTHS[Number(month.slice(5)) - 1]} {month.slice(0, 4)}
          </h4>
          <ol className="tl-list">
            {entries.map((e, i) => (
              <TimelineRow key={i} e={e} conflict={conflictDays.has(e.isoDate!)} selectionId={selectionId} />
            ))}
          </ol>
        </div>
      ))}
      {undated.length > 0 && (
        <div className="tl-month">
          <h4 className="tl-month-title">No exact date</h4>
          <ol className="tl-list">
            {undated.map((e, i) => (
              <TimelineRow key={i} e={e} conflict={false} selectionId={selectionId} />
            ))}
          </ol>
        </div>
      )}
      <p className="muted small ai-meta">
        Read by {r.model} in {(r.durationMs / 1000).toFixed(1)} s · {r.usage.inputTokens.toLocaleString()} tokens in,{" "}
        {r.usage.outputTokens.toLocaleString()} out · ≈${r.usage.costUsd.toFixed(4)} at API list price. Conflicts and crunch
        weeks are found by MyFile's code, not the AI.
      </p>
    </section>
  );
}

function TimelineRow({ e, conflict, selectionId }: { e: TimelineEntry; conflict: boolean; selectionId: number }) {
  return (
    <li className={`tl-row ${conflict ? "tl-conflict" : ""}`}>
      <span className="tl-date">{e.isoDate ? prettyDate(e.isoDate) : e.date ?? "—"}</span>
      <span className="tl-body">
        <span className="tl-title">
          <span className="doc-chip" style={{ borderColor: docColor(e.docIndex), color: docColor(e.docIndex) }}>
            {e.docName.replace(/\.(pdf|md|txt)$/i, "")}
          </span>
          {e.title}
          {conflict && <span className="tag tag-warn">conflict</span>}
        </span>
        {e.suspicious && (
          <span className="tl-quote warn-text">
            <AlertIcon width={12} height={12} /> Quotes text addressed to an AI (likely planted); left off the calendar
          </span>
        )}
        {e.dateCheck === "unsupported" && (
          <span className="tl-quote warn-text">
            <AlertIcon width={12} height={12} /> Date not in its own quote; left off the calendar
          </span>
        )}
        {e.quote && (
          <span className={`tl-quote ${e.quoteCheck === "verified" ? "ok-text" : "warn-text"}`}>
            {e.quoteCheck === "verified" ? <CheckIcon width={12} height={12} /> : <AlertIcon width={12} height={12} />}“{e.quote}”
            {e.page !== null ? ` (p. ${e.page})` : ""}
            {e.quoteCheck === "verified" && (
              <>
                {" "}
                <EvidenceButton source={{ kind: "folder", selectionId, name: e.docName }} quote={e.quote} page={e.page} label="show" />
              </>
            )}
          </span>
        )}
      </span>
    </li>
  );
}

function AskAcross({ folder, provider }: { folder: FolderSummary; provider: "local" | "claude" }) {
  const [q, setQ] = useState("");
  const [mode, setMode] = useState<"ask" | "fact">("ask");
  const [fact, setFact] = useState<
    { status: "idle" } | { status: "running" } | { status: "done"; r: FactCheck } | { status: "error"; error: AppError }
  >({ status: "idle" });
  const [state, setState] = useState<
    { status: "idle" } | { status: "running" } | { status: "done"; r: AskResult } | { status: "error"; error: AppError }
  >({ status: "idle" });

  async function ask() {
    if (mode === "fact") {
      setFact({ status: "running" });
      try {
        setFact({ status: "done", r: await factCheckApi.folder(folder.selectionId, q, provider) });
      } catch (e) {
        setFact({ status: "error", error: toAppError(e) });
      }
      return;
    }
    setState({ status: "running" });
    try {
      setState({ status: "done", r: await folderApi.ask(folder.selectionId, q, provider) });
    } catch (e) {
      setState({ status: "error", error: toAppError(e) });
    }
  }

  return (
    <section className="panel" aria-label="Ask across all documents">
      <h3 className="section-title">Ask or fact-check across all documents</h3>
      <div className="segmented" role="radiogroup" aria-label="Ask a question or check a claim">
        {(
          [
            ["ask", "Ask a question"],
            ["fact", "Fact-check a claim"],
          ] as ["ask" | "fact", string][]
        ).map(([id, label]) => (
          <button
            key={id}
            type="button"
            role="radio"
            aria-checked={mode === id}
            className={`segment${mode === id ? " is-selected" : ""}`}
            onClick={() => setMode(id)}
          >
            {label}
          </button>
        ))}
      </div>
      <form
        className="ask-row"
        onSubmit={(e) => {
          e.preventDefault();
          if (q.trim()) ask();
        }}
      >
        <span className="ask-icon" aria-hidden="true">
          <SparkIcon />
        </span>
        <input
          className="ask-input"
          value={q}
          onChange={(e) => setQ(e.target.value)}
          placeholder={
            mode === "fact" ? "e.g. Every class accepts late work." : "e.g. Which classes accept late work, and what's the penalty?"
          }
          aria-label="Question about all documents"
          maxLength={1000}
          disabled={state.status === "running"}
        />
        <button type="submit" className="btn btn-primary" disabled={state.status === "running" || !q.trim()}>
          {state.status === "running" ? "Reading…" : "Ask"}
        </button>
      </form>
      {state.status === "running" && (
        <p className="result-status" role="status">
          <Spinner /> Claude is reading all {folder.docs.filter((d) => !d.error).length} documents…
        </p>
      )}
      {mode === "fact" && fact.status === "running" && (
        <p className="result-status" role="status">
          <Spinner /> Checking the claim against every document…
        </p>
      )}
      {mode === "fact" && fact.status === "error" && <ErrorPanel title="The fact-check didn't complete" error={fact.error} />}
      {mode === "fact" && fact.status === "done" && (
        <FactCheckView r={fact.r} sourceFor={(s) => (s ? { kind: "folder", selectionId: folder.selectionId, name: s } : null)} />
      )}
      {state.status === "error" && <ErrorPanel title="That question didn't complete" error={state.error} />}
      {mode === "ask" && state.status === "done" && (
        <div className="ai-answer">
          {!state.r.found && (
            <p className="warn-line">
              <AlertIcon width={16} height={16} /> The AI says these documents don't answer that.
            </p>
          )}
          <p className="answer-text">{state.r.answer}</p>
          {state.r.items.length > 0 && (
            <ul className="ai-items">
              {state.r.items.map((it, i) => (
                <li key={i} className="ai-item">
                  <div className="ai-item-head">
                    {it.source && <span className="doc-chip">{it.source}</span>}
                    <span className="ai-item-title">{it.title}</span>
                    {it.date && <span className="ai-item-date">{it.date}</span>}
                  </div>
                  {it.detail && <p className="ai-item-detail">{it.detail}</p>}
                  {it.quote && (
                    <blockquote className={`quote ${it.quoteCheck === "verified" ? "is-verified" : "is-unverified"}`}>
                      “{it.quote}”
                      <footer>
                        {it.quoteCheck === "verified" ? (
                          <>
                            <CheckIcon width={14} height={14} /> Found in {it.source}
                            {it.page !== null && `, page ${it.page}`}
                            {it.sourceCorrected && " (MyFile corrected which document this came from)"}
                            {it.source && (
                              <>
                                {" "}
                                ·{" "}
                                <EvidenceButton
                                  source={{ kind: "folder", selectionId: folder.selectionId, name: it.source }}
                                  quote={it.quote}
                                  page={it.page}
                                />
                              </>
                            )}
                          </>
                        ) : (
                          <>
                            <AlertIcon width={14} height={14} /> Not found in any document
                          </>
                        )}
                      </footer>
                    </blockquote>
                  )}
                </li>
              ))}
            </ul>
          )}
          {state.r.notes.length > 0 && (
            <div className="checks" role="note">
              <p className="checks-title">MyFile's checks on this answer</p>
              <ul>
                {state.r.notes.map((n, i) => (
                  <li key={i}>{n}</li>
                ))}
              </ul>
            </div>
          )}
          <p className="muted small ai-meta">
            Answered by {state.r.model} in {(state.r.durationMs / 1000).toFixed(1)} s · ≈${state.r.usage.costUsd.toFixed(4)} at API
            list price
          </p>
        </div>
      )}
    </section>
  );
}
