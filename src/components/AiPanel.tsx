import { useEffect, useRef, useState } from "react";

import {
  api,
  documentStats,
  factCheckApi,
  toAppError,
  type FactCheck,
  type AiItem,
  type AiResult,
  type AiStatus,
  type AiTask,
  type AppError,
  type DocumentStats,
} from "../api";
import { AlertIcon, CalendarIcon, CheckIcon, LockIcon } from "../icons";
import { ErrorPanel, OutputButtons, Spinner } from "./common";
import { FactCheckView } from "./FactCheckView";
import { EvidenceButton } from "./EvidenceViewer";

const TASKS: { id: AiTask | "fact-check"; label: string; run: string }[] = [
  { id: "summarize", label: "Summarize", run: "Summarize document" },
  { id: "deadlines", label: "Deadlines & dates", run: "Find deadlines and dates" },
  { id: "requirements", label: "Requirements", run: "List requirements" },
  { id: "decisions", label: "Decisions & action items", run: "Extract decisions and action items" },
  { id: "simplify", label: "Explain simply", run: "Explain in plain language" },
  { id: "question", label: "Ask a question", run: "Ask" },
  { id: "fact-check", label: "Fact-check a claim", run: "Check this claim" },
];

/** The assistant's tasks, plus fact checking (which has its own verdict format). */
type PanelTask = AiTask | "fact-check";

const MAX_QUESTION = 1000;

type Settled = { ok: true; result: AiResult } | { ok: false; error: AppError };
type SettledFact = { ok: true; result: FactCheck } | { ok: false; error: AppError };

type RunState =
  | { status: "idle" }
  | { status: "running"; startedAt: number }
  | { status: "done"; result: AiResult }
  | { status: "compare"; claude: Settled; local: Settled }
  | { status: "fact"; result: FactCheck }
  | { status: "fact-compare"; claude: SettledFact; local: SettledFact }
  | { status: "error"; error: AppError };

/** Which model reads the document. */
type Provider = "claude" | "local" | "both";

interface Props {
  fileName: string;
  selectionId: number;
  onBusy: (busy: boolean) => void;
  /** Preselected by a typed request ("Ask MyFile"). */
  initialTask?: AiTask;
  initialQuestion?: string;
  /** Start right away (still asking for consent first). */
  autoRun?: boolean;
}

export function AiPanel({ fileName, selectionId, onBusy, initialTask, initialQuestion, autoRun }: Props) {
  const [status, setStatus] = useState<AiStatus | null>(null);
  const [task, setTask] = useState<PanelTask>(initialTask ?? "summarize");
  const [question, setQuestion] = useState(initialQuestion ?? "");
  const autoRan = useRef(false);
  const [provider, setProvider] = useState<Provider>("claude");
  const [local, setLocal] = useState<{ available: boolean; model: string; message: string | null } | null>(null);
  const usesClaude = provider !== "local";

  // The local model is the default wherever it's installed: free, private,
  // and no account needed. Claude is the optional upgrade.
  useEffect(() => {
    api.localStatus().then(
      (l) => {
        setLocal(l);
        if (l.available) setProvider("local");
      },
      () => setLocal({ available: false, model: "", message: "Couldn't check for a local model." }),
    );
  }, []);
  const [consented, setConsented] = useState(false);
  const [consentStats, setConsentStats] = useState<DocumentStats | "loading" | null>(null);
  const [run, setRun] = useState<RunState>({ status: "idle" });
  const [elapsed, setElapsed] = useState(0);

  useEffect(() => {
    api.aiStatus().then(setStatus, () =>
      setStatus({ available: false, provider: "Claude Code CLI", model: null, maxChars: 0 }),
    );
  }, []);

  // A routed request starts once we know the AI is available. Consent is
  // still requested by requestRun() if it hasn't been given for this file.
  useEffect(() => {
    const ready = status && local && (provider === "local" ? local.available : status.available);
    if (autoRun && ready && !autoRan.current) {
      autoRan.current = true;
      requestRun();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [autoRun, status, local, provider]);

  // If this panel goes away mid-request (another action or file was chosen),
  // stop the request instead of leaving it running in the background.
  const runningRef = useRef(false);
  runningRef.current = run.status === "running";
  useEffect(
    () => () => {
      if (runningRef.current) api.cancelAi();
    },
    [],
  );

  // Count seconds while waiting. There's no real progress signal from the
  // model, so MyFile shows elapsed time rather than an invented percentage.
  useEffect(() => {
    if (run.status !== "running") return;
    const timer = setInterval(() => setElapsed(Math.round((Date.now() - run.startedAt) / 1000)), 500);
    return () => clearInterval(timer);
  }, [run]);

  const running = run.status === "running";
  const needsText = task === "question" || task === "fact-check";
  const questionInvalid = needsText && (question.trim() === "" || question.length > (task === "fact-check" ? 500 : MAX_QUESTION));
  const current = TASKS.find((t) => t.id === task)!;

  async function requestRun() {
    // The local model never sends anything off this computer, so it needs no consent.
    if (usesClaude && !consented) {
      // Extract locally first so the prompt can say exactly what will be sent.
      setConsentStats("loading");
      try {
        setConsentStats(await documentStats(selectionId));
      } catch (e) {
        setConsentStats(null);
        setRun({ status: "error", error: toAppError(e) });
      }
      return;
    }
    await send();
  }

  async function send() {
    setElapsed(0);
    setRun({ status: "running", startedAt: Date.now() });
    onBusy(true);
    if (task === "fact-check") {
      const claim = question.trim();
      const settle = (p: Promise<FactCheck>): Promise<SettledFact> =>
        p.then((result) => ({ ok: true as const, result }), (e) => ({ ok: false as const, error: toAppError(e) }));
      try {
        if (provider === "both") {
          const [claude, localResult] = await Promise.all([
            settle(factCheckApi.file(selectionId, claim, "claude")),
            settle(factCheckApi.file(selectionId, claim, "local")),
          ]);
          setRun({ status: "fact-compare", claude, local: localResult });
        } else {
          setRun({ status: "fact", result: await factCheckApi.file(selectionId, claim, provider) });
        }
      } catch (e) {
        setRun({ status: "error", error: toAppError(e) });
      } finally {
        onBusy(false);
      }
      return;
    }
    const aiTask = task as AiTask;
    const q = task === "question" ? question.trim() : null;
    try {
      if (provider === "both") {
        // Same document, same task, same checks: only the model differs.
        const settle = (p: Promise<AiResult>): Promise<Settled> =>
          p.then((result) => ({ ok: true as const, result }), (e) => ({ ok: false as const, error: toAppError(e) }));
        const [claude, localResult] = await Promise.all([
          settle(api.runAi(selectionId, aiTask, q)),
          settle(api.runAiLocal(selectionId, aiTask, q)),
        ]);
        setRun({ status: "compare", claude, local: localResult });
      } else {
        const result =
          provider === "local" ? await api.runAiLocal(selectionId, aiTask, q) : await api.runAi(selectionId, aiTask, q);
        setRun({ status: "done", result });
      }
    } catch (e) {
      setRun({ status: "error", error: toAppError(e) });
    } finally {
      onBusy(false);
    }
  }

  async function acceptConsent() {
    try {
      await api.grantAiConsent(selectionId);
      setConsented(true);
      setConsentStats(null);
      await send();
    } catch (e) {
      setConsentStats(null);
      setRun({ status: "error", error: toAppError(e) });
    }
  }

  if (status && local && !status.available && !local.available) {
    return (
      <ErrorPanel
        title="The AI assistant isn't set up yet"
        error={{
          code: "ai_unavailable",
          message:
            "It needs either a local model (free: install Ollama from ollama.com, then run \"ollama pull qwen2.5:3b\") or the Claude Code app signed in to a paid Claude plan. Everything else in MyFile works without either.",
        }}
      />
    );
  }

  return (
    <>
      <div className="ai-controls">
        <div className="segmented" role="radiogroup" aria-label="Which model reads the document?">
          {(
            [
              ["claude", "Claude (cloud)"],
              ["local", local?.model ? `Local · ${local.model}` : "Local model"],
              ["both", "Both, side by side"],
            ] as [Provider, string][]
          ).map(([id, label]) => (
            <button
              key={id}
              type="button"
              role="radio"
              aria-checked={provider === id}
              className={`segment${provider === id ? " is-selected" : ""}`}
              onClick={() => setProvider(id)}
              disabled={
                running || (id !== "claude" && !local?.available) || (id !== "local" && status?.available === false)
              }
              title={
                id !== "claude" && !local?.available
                  ? local?.message ?? ""
                  : id !== "local" && status?.available === false
                    ? "Claude needs the Claude Code app, signed in to a paid Claude plan."
                    : ""
              }
            >
              {label}
            </button>
          ))}
        </div>
        {local && !local.available && <p className="muted small">Local model unavailable: {local.message}</p>}
        <div className="chip-row" role="radiogroup" aria-label="What should the AI do?">
          {TASKS.map((t) => (
            <button
              key={t.id}
              type="button"
              role="radio"
              aria-checked={task === t.id}
              className={`chip${task === t.id ? " is-selected" : ""}`}
              onClick={() => setTask(t.id)}
              disabled={running}
            >
              {t.label}
            </button>
          ))}
        </div>

        {needsText && (
          <div className="question">
            <label htmlFor="ai-question" className="field-label">
              {task === "fact-check" ? "The claim to check" : "Your question about this document"}
            </label>
            <textarea
              id="ai-question"
              rows={3}
              maxLength={MAX_QUESTION}
              value={question}
              onChange={(e) => setQuestion(e.target.value)}
              placeholder={task === "fact-check" ? "e.g. Late work is accepted for full credit." : "e.g. What happens if I submit late?"}
              disabled={running}
              onKeyDown={(e) => {
                if (e.key === "Enter" && (e.ctrlKey || e.metaKey) && !questionInvalid && !running) requestRun();
              }}
            />
            <p className="muted small">
              {question.length}/{MAX_QUESTION} characters. Press Ctrl+Enter (⌘+Enter on Mac) to send.
            </p>
          </div>
        )}

        <div className="ai-run-row">
          <button
            type="button"
            className="btn btn-primary"
            onClick={requestRun}
            disabled={running || questionInvalid || status === null || local === null || consentStats === "loading"}
          >
            {current.run}
          </button>
          <span className="muted small privacy-note">
            <LockIcon width={14} height={14} />
            {provider === "local"
              ? "Local model: the text stays on this computer."
              : consented
                ? `You allowed this document's text to be sent to Claude.`
                : "Nothing is sent until you confirm."}
          </span>
        </div>
      </div>

      {consentStats && (
        <ConsentDialog
          fileName={fileName}
          stats={consentStats}
          onAccept={acceptConsent}
          onCancel={() => setConsentStats(null)}
        />
      )}

      {running && (
        <div className="ai-waiting" role="status">
          <Spinner />
          <span>
            {provider === "both"
              ? "Claude and the local model are reading the same document…"
              : provider === "local"
                ? "The local model is reading the document on this computer…"
                : "Claude is reading the document…"}{" "}
            {elapsed > 0 && <span className="muted">{elapsed} s</span>}
          </span>
          <button
            type="button"
            className="btn btn-secondary"
            onClick={() => {
              api.cancelAi();
              api.cancelLocal();
            }}
          >
            Cancel
          </button>
        </div>
      )}
      {run.status === "error" && <ErrorPanel title="The AI request didn't complete" error={run.error} />}
      {run.status === "done" && <AiAnswerView result={run.result} selectionId={selectionId} />}
      {run.status === "fact" && <FactCheckView r={run.result} sourceFor={() => ({ kind: "current", selectionId })} />}
      {run.status === "fact-compare" && (
        <div className="compare">
          {(
            [
              ["Claude (cloud)", run.claude],
              [`Local · ${local?.model ?? ""} (on this computer)`, run.local],
            ] as [string, SettledFact][]
          ).map(([title, s]) => (
            <div key={title} className="compare-col">
              <h4 className="compare-title">{title}</h4>
              {s.ok ? <FactCheckView r={s.result} sourceFor={() => ({ kind: "current", selectionId })} /> : <ErrorPanel title="This model didn't complete" error={s.error} />}
            </div>
          ))}
        </div>
      )}
      {run.status === "compare" && (
        <div className="compare">
          {(
            [
              ["Claude (cloud)", run.claude],
              [`Local · ${local?.model ?? ""} (on this computer)`, run.local],
            ] as [string, Settled][]
          ).map(([title, s]) => (
            <div key={title} className="compare-col">
              <h4 className="compare-title">{title}</h4>
              {s.ok ? <AiAnswerView result={s.result} selectionId={selectionId} /> : <ErrorPanel title="This model didn't complete" error={s.error} />}
            </div>
          ))}
        </div>
      )}
    </>
  );
}

function ConsentDialog({
  fileName,
  stats,
  onAccept,
  onCancel,
}: {
  fileName: string;
  stats: DocumentStats | "loading";
  onAccept: () => void;
  onCancel: () => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);

  useEffect(() => {
    const dialog = ref.current;
    if (dialog && !dialog.open) dialog.showModal();
  }, []);

  return (
    <dialog
      ref={ref}
      className="dialog"
      aria-labelledby="consent-title"
      onCancel={(e) => {
        e.preventDefault();
        onCancel();
      }}
    >
      <h2 id="consent-title">Send this document to Claude?</h2>
      {stats === "loading" ? (
        <p className="result-status">
          <Spinner /> Reading the document on this computer…
        </p>
      ) : (
        <>
          <p>
            To answer, MyFile will send <strong>the text of “{fileName}”</strong> (
            {stats.chars.toLocaleString()} characters
            {stats.pages !== null && ` from ${stats.pages} page${stats.pages === 1 ? "" : "s"}`}) and your request to{" "}
            <strong>Anthropic's Claude</strong>, using the Claude Code app signed in on this computer.
          </p>
          {stats.transcript && (
            <p className="muted small">
              This text is a machine transcript of the recording, so it may contain misheard words. Quotes are checked
              against the transcript.
            </p>
          )}
          {stats.ocr && (
            <p className="muted small">
              This text was read with OCR, so it may contain misreads. Quotes are checked against this OCR text.
            </p>
          )}
          <ul className="dialog-list">
            <li>Only the extracted text is sent. The file itself and the rest of your computer are not.</li>
            <li>
              MyFile doesn't keep a copy, and tells Claude Code not to save the conversation on this computer. Anthropic's
              data policies apply to what it receives.
            </li>
            <li>You'll be asked again for each new file.</li>
          </ul>
        </>
      )}
      <div className="dialog-buttons">
        <button type="button" className="btn btn-secondary" onClick={onCancel}>
          Cancel
        </button>
        <button type="button" className="btn btn-primary" onClick={onAccept} disabled={stats === "loading"}>
          Send to Claude
        </button>
      </div>
    </dialog>
  );
}

/** Renders "- " lines as a list and everything else as paragraphs. */
function AnswerText({ text }: { text: string }) {
  const blocks: { list: boolean; lines: string[] }[] = [];
  for (const raw of text.split("\n")) {
    const line = raw.trim();
    if (!line) {
      blocks.push({ list: false, lines: [] });
      continue;
    }
    const isItem = /^[-•*]\s+/.test(line);
    const last = blocks[blocks.length - 1];
    const content = isItem ? line.replace(/^[-•*]\s+/, "") : line;
    if (last && last.list === isItem && last.lines.length > 0) last.lines.push(content);
    else blocks.push({ list: isItem, lines: [content] });
  }
  return (
    <div className="answer-text">
      {blocks
        .filter((b) => b.lines.length > 0)
        .map((b, i) =>
          b.list ? (
            <ul key={i}>
              {b.lines.map((l, j) => (
                <li key={j}>{l}</li>
              ))}
            </ul>
          ) : (
            <p key={i}>{b.lines.join(" ")}</p>
          ),
        )}
    </div>
  );
}

function AiAnswerView({ result: r, selectionId }: { result: AiResult; selectionId: number }) {
  return (
    <article className="ai-answer" aria-label="AI answer">
      {!r.found && (
        <p className="warn-line">
          <AlertIcon width={16} height={16} /> The AI says the document doesn't contain what you asked for.
        </p>
      )}
      {r.checks.injectionPhrases.length > 0 && (
        <p className="warn-line" role="alert">
          <AlertIcon width={16} height={16} /> This document contains text that looks like instructions to an AI: “
          {r.checks.injectionPhrases[0]}”. MyFile tells the AI to ignore instructions in documents, but check this answer.
        </p>
      )}
      <TrustSummary result={r} />
      <AnswerText text={r.answer} />

      {r.items.length > 0 && (
        <ul className="ai-items">
          {r.items.map((item, i) => (
            <AiItemCard key={i} item={item} timeMs={r.itemTimes?.[i] ?? null} selectionId={selectionId} />
          ))}
        </ul>
      )}

      {r.notes.length > 0 && (
        <div className="checks" role="note">
          <p className="checks-title">MyFile's checks on this answer</p>
          <ul>
            {r.notes.map((n, i) => (
              <li key={i}>{n}</li>
            ))}
          </ul>
        </div>
      )}

      {r.calendar && <CalendarExport calendar={r.calendar} />}
      <UsageLine result={r} />
      <OutputButtons outputId={r.outputId} copyable />
    </article>
  );
}

/** Headline of what MyFile's own code verified, e.g. "7 of 8 quotes found in the document". */
function TrustSummary({ result: r }: { result: AiResult }) {
  const { quotesChecked, quotesVerified, pagesCorrected, pagesRemoved, datesUnsupported } = r.checks;
  if (quotesChecked === 0) {
    if (r.items.length === 0) return null;
    return (
      <p className="trust trust-none">
        <AlertIcon width={16} height={16} /> No quotes were given, so MyFile couldn't check these points against the
        document.
      </p>
    );
  }
  const all = quotesVerified === quotesChecked && datesUnsupported === 0;
  const fixes = pagesCorrected + pagesRemoved;
  return (
    <p className={`trust ${all ? "trust-ok" : "trust-warn"}`} role="status">
      {all ? <CheckIcon width={16} height={16} /> : <AlertIcon width={16} height={16} />}
      <span>
        <strong>
          {quotesVerified} of {quotesChecked} quote{quotesChecked === 1 ? "" : "s"}
        </strong>{" "}
        found word for word in the document
        {fixes > 0 && `, ${fixes} page number${fixes === 1 ? "" : "s"} fixed by MyFile`}
        {datesUnsupported > 0 && `, ${datesUnsupported} date${datesUnsupported === 1 ? "" : "s"} not found in ${datesUnsupported === 1 ? "its" : "their"} own quote`}.
      </span>
    </p>
  );
}

function CalendarExport({ calendar: c }: { calendar: NonNullable<AiResult["calendar"]> }) {
  const [saved, setSaved] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  async function save() {
    try {
      const name = await api.saveOutput(c.outputId);
      if (name) setSaved(name);
    } catch (e) {
      setError(toAppError(e).message);
    }
  }
  return (
    <div className="calendar-export">
      <button type="button" className="btn btn-primary" onClick={save}>
        <CalendarIcon width={16} height={16} /> Add {c.events} date{c.events === 1 ? "" : "s"} to my calendar
      </button>
      <span className="muted small">
        Saves a .ics file that Outlook, Google Calendar, and Apple Calendar can import.
        {c.skipped > 0 &&
          ` ${c.skipped} date${c.skipped === 1 ? " was" : "s were"} left out because the exact day or year wasn't certain.`}
      </span>
      {saved && (
        <p className="feedback is-ok" role="status">
          <CheckIcon width={16} height={16} /> Saved as “{saved}”. Open it to add the events.
        </p>
      )}
      {error && <p className="feedback is-bad" role="status">{error}</p>}
    </div>
  );
}

function UsageLine({ result: r }: { result: AiResult }) {
  const u = r.usage;
  const local = r.model.endsWith("(local)");
  const cost = u.costUsd < 0.01 ? `${u.costUsd.toFixed(4)}` : `${u.costUsd.toFixed(3)}`;
  return (
    <p className="muted small ai-meta">
      Answered by {r.model} in {(r.durationMs / 1000).toFixed(1)} s ({(u.apiMs / 1000).toFixed(1)} s of model time) from{" "}
      {r.charsSent.toLocaleString()} characters · {u.inputTokens.toLocaleString()} tokens in,{" "}
      {u.outputTokens.toLocaleString()} out · <span title="The Claude Code CLI's list-price estimate. On a Claude subscription this isn't billed per answer; it counts against your plan's usage limits.">
        {local ? "free: ran on this computer, nothing sent" : `≈${cost} at API list price`}
      </span>
      . AI can make mistakes, so check important details against the document.
    </p>
  );
}

function clock(ms: number) {
  const s = Math.floor(ms / 1000);
  return s >= 3600
    ? `${Math.floor(s / 3600)}:${String(Math.floor(s / 60) % 60).padStart(2, "0")}:${String(s % 60).padStart(2, "0")}`
    : `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

function AiItemCard({ item, timeMs, selectionId }: { item: AiItem; timeMs: number | null; selectionId: number }) {
  return (
    <li className="ai-item">
      <div className="ai-item-head">
        {item.category && <span className="tag">{item.category}</span>}
        <span className="ai-item-title">{item.title}</span>
        {item.date && <span className="ai-item-date">{item.date}</span>}
      </div>
      {item.detail && <p className="ai-item-detail">{item.detail}</p>}
      {item.quote && (
        <blockquote className={`quote ${item.quoteCheck === "verified" ? "is-verified" : "is-unverified"}`}>
          “{item.quote}”
          <footer>
            {item.quoteCheck === "verified" ? (
              <>
                <CheckIcon width={14} height={14} /> {timeMs !== null ? `Said at ${clock(timeMs)} in the recording` : "Found in the document"}
                {item.page !== null && `, page ${item.page}`} ·{" "}
                <EvidenceButton source={{ kind: "current", selectionId }} quote={item.quote} page={item.page} />
              </>
            ) : (
              <>
                <AlertIcon width={14} height={14} /> Not found in the document. The AI may have reworded or invented it.
              </>
            )}
          </footer>
        </blockquote>
      )}
      {!item.quote && item.page !== null && <p className="muted small">Page {item.page}</p>}
      {item.suspicious && (
        <p className="inferred">
          <AlertIcon width={12} height={12} /> This comes from text in the document that is addressed to an AI, so it is
          probably planted. Left off the calendar.
        </p>
      )}
      {item.dateCheck === "unsupported" && (
        <p className="inferred">
          <AlertIcon width={12} height={12} /> This date isn't in the quoted passage. It may be wrong or planted, so it was
          left off the calendar.
        </p>
      )}
      {item.basis === "inferred" && <p className="inferred">Inferred: not stated directly in the document.</p>}
    </li>
  );
}
