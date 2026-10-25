import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";

import {
  api,
  searchApi,
  toAppError,
  type AppError,
  type FilesAnswer,
  type IndexReport,
  type IndexStatus,
  type PreparedAsk,
  type SearchResult,
} from "../api";
import { AlertIcon, CheckIcon, LockIcon, SparkIcon } from "../icons";
import { ErrorPanel, Spinner } from "./common";
import { EvidenceButton } from "./EvidenceViewer";

type Progress = { phase: "reading" | "meaning" | "download"; done: number; total: number; current: string };

/** Re-check the folders in the background when the last update is older than this. */
const STALE_SECS = 10 * 60;

function reportText(r: IndexReport) {
  const parts = [
    r.added && `${r.added} added`,
    r.updated && `${r.updated} updated`,
    r.removed && `${r.removed} removed`,
    r.failed && `${r.failed} couldn't be read`,
    r.embedded && `${r.embedded} passages understood for meaning search`,
    r.skippedOverLimit && `${r.skippedOverLimit} over the file limit`,
  ].filter(Boolean);
  return parts.length ? parts.join(" · ") : "Everything was already up to date.";
}

/**
 * Search your files: a local index of folders you choose. Word search
 * always; meaning search when the local embedding model is on; and "Ask"
 * answers from the best passages with checked quotes.
 */
export function SearchPanel({ onOpen }: { onOpen: (path: string) => void }) {
  const [status, setStatus] = useState<IndexStatus | null>(null);
  const [query, setQuery] = useState("");
  const [result, setResult] = useState<SearchResult | null>(null);
  // The query the results are for (the box may have changed since).
  const [searched, setSearched] = useState("");
  const [busy, setBusy] = useState<null | "search" | "index" | "ask">(null);
  const [progress, setProgress] = useState<Progress | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [manage, setManage] = useState(false);
  const [ask, setAsk] = useState<
    | { status: "idle" }
    | { status: "preparing" }
    | { status: "ready"; p: PreparedAsk; provider: "local" | "claude" }
    | { status: "running"; p: PreparedAsk; provider: "local" | "claude" }
    | { status: "done"; p: PreparedAsk; a: FilesAnswer }
    | { status: "error"; error: AppError }
  >({ status: "idle" });
  const [localModel, setLocalModel] = useState<string | null | undefined>(undefined);
  const [provider, setProvider] = useState<"local" | "claude">("local");
  const refreshed = useRef(false);

  const reload = () => searchApi.status().then(setStatus, (e) => setError(toAppError(e)));

  useEffect(() => {
    reload();
    api.localStatus().then(
      (l) => {
        setLocalModel(l.available ? l.model : null);
        if (!l.available) setProvider("claude");
      },
      () => {
        setLocalModel(null);
        setProvider("claude");
      },
    );
    const off = listen<Progress>("search-index-progress", (e) => setProgress(e.payload));
    return () => {
      off.then((f) => f());
    };
  }, []);

  // Quietly bring the index up to date once per launch.
  useEffect(() => {
    if (!status || refreshed.current || status.running || status.roots.length === 0) return;
    refreshed.current = true;
    const oldest = Math.min(...status.roots.map((r) => r.indexedAt ?? 0));
    if (Date.now() / 1000 - oldest > STALE_SECS) runIndex(() => searchApi.reindex(), true);
  }, [status]);

  async function runIndex(task: () => Promise<IndexReport>, quiet = false) {
    setBusy("index");
    setProgress(null);
    setError(null);
    try {
      const r = await task();
      if (!quiet) setMessage(reportText(r));
    } catch (e) {
      const err = toAppError(e);
      if (err.code !== "cancelled") setError(err);
    } finally {
      setBusy(null);
      setProgress(null);
      reload();
    }
  }

  async function addFolder() {
    const picked = await open({ directory: true, multiple: false, title: "Choose a folder for MyFile to search" });
    if (typeof picked === "string") runIndex(() => searchApi.addFolder(picked));
  }

  async function search() {
    if (!query.trim()) return;
    setBusy("search");
    setError(null);
    setAsk({ status: "idle" });
    try {
      setResult(await searchApi.search(query));
      setSearched(query);
    } catch (e) {
      setError(toAppError(e));
    } finally {
      setBusy(null);
    }
  }

  async function startAsk() {
    if (!query.trim()) return;
    setAsk({ status: "preparing" });
    setResult(null);
    try {
      const p = await searchApi.prepareAsk(query);
      if (provider === "local") {
        runAsk(p, "local");
      } else {
        setAsk({ status: "ready", p, provider });
      }
    } catch (e) {
      setAsk({ status: "error", error: toAppError(e) });
    }
  }

  async function runAsk(p: PreparedAsk, provider: "local" | "claude") {
    setAsk({ status: "running", p, provider });
    try {
      setAsk({ status: "done", p, a: await searchApi.ask(p.askId, provider) });
    } catch (e) {
      setAsk({ status: "error", error: toAppError(e) });
    }
  }

  if (!status) return null;
  const hasRoots = status.roots.length > 0;
  const meaningOn = status.vectors > 0 && status.embedModelInstalled;

  return (
    <section className="panel search-panel" aria-label="Search your files">
      <h3 className="section-title">Search your files</h3>

      {!hasRoots ? (
        <div className="callout">
          <p>
            Search what your documents <em>say</em>, not just their names. Choose folders (like a class folder or Documents) and
            MyFile builds a search index of their PDFs and text files.
          </p>
          <p className="muted small">
            <LockIcon width={14} height={14} /> The index stays in MyFile's data folder on this computer. You can delete it at any
            time.
          </p>
          <div className="output-bar">
            <button type="button" className="btn btn-primary" onClick={addFolder} disabled={busy === "index"}>
              Choose a folder to search…
            </button>
          </div>
        </div>
      ) : (
        <>
          <form
            className="search-row"
            onSubmit={(e) => {
              e.preventDefault();
              search();
            }}
          >
            <input
              className="text-input"
              type="search"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="e.g. the late policy for my biology lab"
              aria-label="Search your files"
              maxLength={300}
            />
            <button type="submit" className="btn btn-primary" disabled={!query.trim() || busy === "search"}>
              Search
            </button>
            <button
              type="button"
              className="btn btn-secondary"
              onClick={startAsk}
              disabled={!query.trim() || localModel === undefined || ask.status === "preparing" || ask.status === "running"}
              title="Answer the question from the best matching passages, with checked quotes"
            >
              <SparkIcon width={16} height={16} /> Ask
            </button>
          </form>
          {localModel && (
            <div className="segmented" role="radiogroup" aria-label="Which model answers Ask?">
              {(
                [
                  ["local", `Ask with local · ${localModel}`],
                  ["claude", "Ask with Claude"],
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
          <p className="muted small">
            {status.roots.length} folder{status.roots.length === 1 ? "" : "s"} · {status.passages.toLocaleString()} passages ·
            matching {meaningOn ? "words and meaning" : "words"} ·{" "}
            <button type="button" className="link-button" onClick={() => setManage((m) => !m)}>
              {manage ? "Hide folders" : "Manage folders"}
            </button>
          </p>
        </>
      )}

      {busy === "index" && (
        <div className="folder-progress" role="status">
          <div className="progress-head">
            <Spinner />
            <span>
              {progress?.phase === "download"
                ? `Downloading ${status.embedModel}… ${progress.total ? Math.round((progress.done / progress.total) * 100) + "%" : ""}`
                : progress?.phase === "meaning"
                  ? `Understanding passages for meaning search: ${progress.done} of ${progress.total}`
                  : progress && progress.total
                    ? `Reading ${progress.current || "files"} (${progress.done} of ${progress.total})`
                    : "Looking for documents…"}
            </span>
            <button type="button" className="btn btn-secondary" onClick={() => searchApi.stop()}>
              Stop
            </button>
          </div>
          {progress && progress.total > 0 && (
            <div className="progress-bar" aria-hidden="true">
              <span style={{ width: `${Math.min(100, (progress.done / progress.total) * 100)}%` }} />
            </div>
          )}
        </div>
      )}
      {message && busy !== "index" && (
        <p className="feedback is-ok" role="status">
          <CheckIcon width={16} height={16} /> {message}
        </p>
      )}
      {error && <ErrorPanel title="Search had a problem" error={error} />}

      {manage && hasRoots && (
        <div className="callout search-manage">
          <ul className="search-roots">
            {status.roots.map((r) => (
              <li key={r.path} className="search-root">
                <span className="search-root-info">
                  <strong>{r.name}</strong>{" "}
                  <span className="muted small">
                    {r.files} files · {r.passages} passages{r.failed ? ` · ${r.failed} couldn't be read` : ""}
                  </span>
                  <br />
                  <span className="muted small">{r.path}</span>
                </span>
                <button
                  type="button"
                  className="btn btn-secondary"
                  onClick={async () => {
                    await searchApi.removeFolder(r.path);
                    reload();
                  }}
                  disabled={busy === "index"}
                >
                  Remove
                </button>
              </li>
            ))}
          </ul>
          <div className="output-bar">
            <button type="button" className="btn btn-secondary" onClick={addFolder} disabled={busy === "index"}>
              Add a folder…
            </button>
            <button type="button" className="btn btn-secondary" onClick={() => runIndex(() => searchApi.reindex())} disabled={busy === "index"}>
              Update the index
            </button>
            <button
              type="button"
              className="btn btn-secondary"
              onClick={async () => {
                try {
                  await searchApi.deleteIndex();
                  setResult(null);
                  setMessage("The search index was deleted.");
                } catch (e) {
                  setError(toAppError(e));
                }
                reload();
              }}
              disabled={busy === "index"}
            >
              Delete the index
            </button>
          </div>
          {!meaningOn && (
            <div>
              <p className="small">
                <strong>Meaning search</strong> also finds passages that use different words ("moving out" finds "notice to
                vacate"). It uses the local model {status.embedModel} through Ollama
                {status.embedModelInstalled ? "" : ", a one-time download of about 270 MB"}. Nothing leaves this computer.
              </p>
              <button
                type="button"
                className="btn btn-secondary"
                onClick={() => runIndex(() => searchApi.enableMeaning())}
                disabled={busy === "index" || !status.ollama}
              >
                Turn on meaning search
              </button>
              {!status.ollama && <p className="muted small">Needs Ollama running (https://ollama.com).</p>}
            </div>
          )}
        </div>
      )}

      {busy === "search" && (
        <p className="result-status" role="status">
          <Spinner /> Searching…
        </p>
      )}
      {result && <Results r={result} query={searched} onOpen={onOpen} />}
      <AskView ask={ask} onOpen={onOpen} onSend={(p) => runAsk(p, "claude")} onCancel={() => setAsk({ status: "idle" })} />
    </section>
  );
}

function Snippet({ parts }: { parts: { text: string; hit: boolean }[] }) {
  return (
    <span className="snippet">
      {parts.map((p, i) => (p.hit ? <mark key={i}>{p.text}</mark> : <span key={i}>{p.text}</span>))}
    </span>
  );
}

function Results({ r, query, onOpen }: { r: SearchResult; query: string; onOpen: (path: string) => void }) {
  if (r.hits.length === 0) {
    return <p className="muted">No passages match. Try other words{r.mode === "words" ? ", or turn on meaning search" : ""}.</p>;
  }
  return (
    <div className="search-results">
      <p className="muted small" role="status">
        {r.hits.length} file{r.hits.length === 1 ? "" : "s"} · matched by {r.mode} · {r.elapsedMs} ms
      </p>
      <ul className="search-hits">
        {r.hits.map((h) => (
          <li key={h.path} className="search-hit">
            <button type="button" className="search-hit-open" onClick={() => onOpen(h.path)} title={`Open ${h.path}`}>
              <span className="search-hit-name">{h.name}</span>
              <span className="muted small">
                {" "}
                in {h.folder} · {h.kind === "pdf" ? "PDF" : "text"}
              </span>
            </button>
            <ul className="search-passages">
              {h.passages.map((p) => (
                <li key={p.chunkId}>
                  {p.page !== null && <span className="doc-chip">p. {p.page}</span>}
                  {p.matchedBy === "meaning" && (
                    <span className="doc-chip" title="Matched by meaning, not the exact words">
                      similar meaning
                    </span>
                  )}{" "}
                  <Snippet parts={p.snippet} /> <EvidenceButton source={{ kind: "passage", chunkId: p.chunkId, query }} label="show on page" />
                </li>
              ))}
            </ul>
          </li>
        ))}
      </ul>
    </div>
  );
}

function AskView({
  ask,
  onOpen,
  onSend,
  onCancel,
}: {
  ask:
    | { status: "idle" }
    | { status: "preparing" }
    | { status: "ready"; p: PreparedAsk; provider: "local" | "claude" }
    | { status: "running"; p: PreparedAsk; provider: "local" | "claude" }
    | { status: "done"; p: PreparedAsk; a: FilesAnswer }
    | { status: "error"; error: AppError };
  onOpen: (path: string) => void;
  onSend: (p: PreparedAsk) => void;
  onCancel: () => void;
}) {
  if (ask.status === "idle") return null;
  if (ask.status === "preparing")
    return (
      <p className="result-status" role="status">
        <Spinner /> Finding the passages that answer this…
      </p>
    );
  if (ask.status === "error") return <ErrorPanel title="Couldn't answer that" error={ask.error} />;

  const sources = (
    <ul className="dialog-list">
      {ask.p.sources.map((s) => (
        <li key={s.path}>
          <strong>{s.name}</strong>{" "}
          <span className="muted small">
            {s.passages} passage{s.passages === 1 ? "" : "s"}
            {s.pages.length > 0 && ` · page${s.pages.length === 1 ? "" : "s"} ${s.pages.join(", ")}`}
          </span>
        </li>
      ))}
    </ul>
  );

  if (ask.status === "ready")
    return (
      <div className="callout">
        <p>
          To answer, MyFile would send <strong>these {ask.p.chars.toLocaleString()} characters of passages</strong> and your
          question to <strong>Anthropic's Claude</strong>. Nothing else from your files is sent.
        </p>
        {sources}
        <div className="output-bar">
          <button type="button" className="btn btn-secondary" onClick={onCancel}>
            Cancel
          </button>
          <button type="button" className="btn btn-primary" onClick={() => onSend(ask.p)}>
            Send to Claude
          </button>
        </div>
      </div>
    );

  if (ask.status === "running")
    return (
      <div>
        <p className="result-status" role="status">
          <Spinner /> {ask.provider === "local" ? "The local model is reading" : "Claude is reading"} {ask.p.sources.length} file
          {ask.p.sources.length === 1 ? "" : "s"}…
        </p>
        {sources}
      </div>
    );

  const a = ask.a;
  return (
    <div className="ai-answer">
      {!a.found && (
        <p className="warn-line">
          <AlertIcon width={16} height={16} /> The AI says these passages don't answer that.
        </p>
      )}
      <p className="answer-text" style={{ whiteSpace: "pre-line" }}>
        {a.answer}
      </p>
      {a.items.length > 0 && (
        <ul className="ai-items">
          {a.items.map((it, i) => (
            <li key={i} className="ai-item">
              <div className="ai-item-head">
                {it.source && a.paths[it.source] && (
                  <button type="button" className="doc-chip link-chip" onClick={() => onOpen(a.paths[it.source!])}>
                    {it.source}
                  </button>
                )}
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
                        {it.sourceCorrected && " (MyFile corrected which file this came from)"}
                        {it.source && a.paths[it.source] && (
                          <>
                            {" "}
                            · <EvidenceButton source={{ kind: "indexed", path: a.paths[it.source] }} quote={it.quote} page={it.page} />
                          </>
                        )}
                      </>
                    ) : (
                      <>
                        <AlertIcon width={14} height={14} /> Not found in the passages. The AI may have reworded or invented it.
                      </>
                    )}
                  </footer>
                </blockquote>
              )}
            </li>
          ))}
        </ul>
      )}
      {a.notes.length > 0 && (
        <div className="checks" role="note">
          <p className="checks-title">MyFile's checks on this answer</p>
          <ul>
            {a.notes.map((n, i) => (
              <li key={i}>{n}</li>
            ))}
          </ul>
        </div>
      )}
      <details className="muted small">
        <summary>Answered from {ask.p.sources.length} file(s)</summary>
        {sources}
      </details>
      <p className="muted small ai-meta">
        Answered by {a.model} in {(a.durationMs / 1000).toFixed(1)} s ·{" "}
        {a.model.endsWith("(local)") ? "free, on this computer" : `≈$${a.usage.costUsd.toFixed(4)} at API list price`}
      </p>
    </div>
  );
}
