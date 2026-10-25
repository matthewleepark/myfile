import { useEffect, useRef, useState } from "react";

import { evidenceApi, toAppError, type AppError, type Evidence } from "../api";
import { CloseIcon } from "../icons";
import { ErrorPanel, Spinner } from "./common";

/** Where a quote or passage lives. */
export type EvidenceSource =
  | { kind: "current"; selectionId: number }
  | { kind: "folder"; selectionId: number; name: string }
  | { kind: "indexed"; path: string }
  | { kind: "passage"; chunkId: number; query: string };

/** A small "Show in document" button that opens the viewer. */
export function EvidenceButton({ source, quote, page, label = "Show in document" }: { source: EvidenceSource; quote?: string | null; page?: number | null; label?: string }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button type="button" className="link-button evidence-link" onClick={() => setOpen(true)}>
        {label}
      </button>
      {open && <EvidenceViewer source={source} quote={quote ?? null} page={page ?? null} onClose={() => setOpen(false)} />}
    </>
  );
}

/**
 * The page a quote came from, with the exact span marked. MyFile finds the
 * span itself, the same way the quote check did, so the highlight is what
 * was verified.
 */
export function EvidenceViewer({ source, quote, page, onClose }: { source: EvidenceSource; quote: string | null; page: number | null; onClose: () => void }) {
  const ref = useRef<HTMLDialogElement>(null);
  const markRef = useRef<HTMLElement>(null);
  const [ev, setEv] = useState<Evidence | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  // A passage lookup resolves to its file; paging then reads that file.
  const [path, setPath] = useState<string | null>(source.kind === "indexed" ? source.path : null);
  const [q, setQ] = useState<string | null>(quote);

  useEffect(() => {
    if (ref.current && !ref.current.open) ref.current.showModal();
    load(page, false);
  }, []);

  useEffect(() => {
    markRef.current?.scrollIntoView({ block: "center" });
  }, [ev]);

  async function load(p: number | null, stay: boolean) {
    setError(null);
    try {
      if (source.kind === "passage" && !path) {
        const [file, e] = await evidenceApi.passage(source.chunkId, source.query);
        setPath(file);
        setQ(e.matched || null);
        setEv(e);
        return;
      }
      if (source.kind === "current") setEv(await evidenceApi.current(source.selectionId, q, p, stay));
      else if (source.kind === "folder") setEv(await evidenceApi.folder(source.selectionId, source.name, q, p, stay));
      else setEv(await evidenceApi.indexed(path!, q, p, stay));
    } catch (e) {
      setError(toAppError(e));
    }
  }

  return (
    <dialog
      ref={ref}
      className="dialog evidence-dialog"
      aria-label="Source in the document"
      onCancel={(e) => {
        e.preventDefault();
        onClose();
      }}
    >
      <div className="evidence-head">
        <div>
          <h2 className="evidence-title">{ev?.name ?? "Opening…"}</h2>
          {ev && (
            <p className="muted small">
              {ev.page !== null ? `Page ${ev.page} of ${ev.pageCount}` : "Whole document"}
              {q &&
                (ev.found
                  ? source.kind === "passage"
                    ? " · marked: the best match for your search"
                    : " · the marked text is exactly what was quoted"
                  : source.kind === "passage"
                    ? ""
                    : " · the quote isn't on this page")}
            </p>
          )}
        </div>
        <button type="button" className="btn btn-icon" aria-label="Close" onClick={onClose}>
          <CloseIcon />
        </button>
      </div>
      {error && <ErrorPanel title="Couldn't show the source" error={error} />}
      {!ev && !error && (
        <p className="result-status">
          <Spinner /> Reading the document…
        </p>
      )}
      {ev && (
        <div className="evidence-body" tabIndex={0}>
          {ev.before}
          {ev.matched && <mark ref={markRef}>{ev.matched}</mark>}
          {ev.after}
          {!ev.before && !ev.matched && !ev.after && <span className="muted">This page has no text.</span>}
        </div>
      )}
      {ev && ev.page !== null && ev.pageCount > 1 && (
        <div className="dialog-buttons">
          <button type="button" className="btn btn-secondary" disabled={ev.page <= 1} onClick={() => load(ev.page! - 1, true)}>
            Previous page
          </button>
          <button type="button" className="btn btn-secondary" disabled={ev.page >= ev.pageCount} onClick={() => load(ev.page! + 1, true)}>
            Next page
          </button>
        </div>
      )}
    </dialog>
  );
}
