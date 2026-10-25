import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";

import { monitorApi, toAppError, type AppError, type Comparison, type MonitorItem } from "../api";
import { AlertIcon, CheckIcon, EyeIcon } from "../icons";
import { CompareResult } from "./ComparePanel";
import { ErrorPanel, Spinner } from "./common";

/** The action panel: start monitoring the open document. */
export function MonitorPanel({ selectionId, onAdded }: { selectionId: number; onAdded: () => void }) {
  const [state, setState] = useState<
    { status: "idle" } | { status: "saving" } | { status: "done"; item: MonitorItem } | { status: "error"; error: AppError }
  >({ status: "idle" });

  async function start() {
    setState({ status: "saving" });
    try {
      const item = await monitorApi.add(selectionId);
      setState({ status: "done", item });
      onAdded();
    } catch (e) {
      setState({ status: "error", error: toAppError(e) });
    }
  }

  return (
    <>
      <p className="muted small">
        MyFile keeps a copy of this document's <strong>text</strong> in its own data folder on this computer, and checks it
        whenever the file changes (and each time MyFile starts). You'll be told what changed, not just that the file was
        saved. "Stop monitoring" deletes the copy. Nothing is sent anywhere.
      </p>
      {state.status !== "done" && (
        <div>
          <button type="button" className="btn btn-primary" onClick={start} disabled={state.status === "saving"}>
            <EyeIcon width={16} height={16} /> {state.status === "saving" ? "Taking a snapshot…" : "Start monitoring"}
          </button>
        </div>
      )}
      {state.status === "done" && (
        <p className="success-line" role="status">
          Monitoring “{state.item.name}”. {state.item.summary}
        </p>
      )}
      {state.status === "error" && <ErrorPanel title="Couldn't start monitoring" error={state.error} />}
    </>
  );
}

const STATUS_LABEL: Record<MonitorItem["status"], string> = {
  unchanged: "No changes",
  "metadata-only": "Saved, same text",
  "text-changed": "Changed",
  missing: "File missing",
  unreadable: "Can't read",
};

/** The always-available list of monitored documents. */
export function MonitoredList({ refreshKey }: { refreshKey: number }) {
  const [items, setItems] = useState<MonitorItem[] | null>(null);
  const [open, setOpen] = useState<{ id: number; c: Comparison } | null>(null);
  const [busy, setBusy] = useState<number | null>(null);
  const [error, setError] = useState<AppError | null>(null);

  const refresh = useCallback(() => {
    monitorApi.list().then(setItems, (e) => setError(toAppError(e)));
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh, refreshKey]);

  // The backend reports changes as they happen (and at startup).
  useEffect(() => {
    const un = listen("monitor-event", () => refresh());
    return () => {
      un.then((f) => f());
    };
  }, [refresh]);

  if (!items || items.length === 0) return null;

  async function see(id: number) {
    setBusy(id);
    setError(null);
    try {
      setOpen({ id, c: await monitorApi.changes(id) });
    } catch (e) {
      setError(toAppError(e));
    } finally {
      setBusy(null);
    }
  }

  return (
    <section className="monitored" aria-label="Monitored documents">
      <h3 className="section-title">Monitored documents</h3>
      <ul className="watch-inbox">
        {items.map((it) => (
          <li key={it.id} className={`watch-item ${it.status === "text-changed" ? "is-changed" : ""}`}>
            <span className="watch-icon" aria-hidden="true">
              {it.status === "text-changed" ? (
                <AlertIcon width={16} height={16} />
              ) : it.status === "missing" || it.status === "unreadable" ? (
                <AlertIcon width={16} height={16} />
              ) : (
                <CheckIcon width={16} height={16} />
              )}
            </span>
            <span className="watch-body">
              <strong>
                {it.name} <span className={`tag ${it.status === "text-changed" ? "tag-warn" : ""}`}>{STATUS_LABEL[it.status]}</span>
              </strong>
              <span className="muted small">{it.summary}</span>
            </span>
            <span className="watch-actions">
              {it.status === "text-changed" && (
                <>
                  <button type="button" className="btn btn-primary" onClick={() => see(it.id)} disabled={busy === it.id}>
                    {busy === it.id ? <Spinner /> : "See what changed"}
                  </button>
                  <button
                    type="button"
                    className="btn btn-secondary"
                    onClick={async () => {
                      await monitorApi.accept(it.id).catch((e) => setError(toAppError(e)));
                      if (open?.id === it.id) setOpen(null);
                      refresh();
                    }}
                  >
                    Mark as reviewed
                  </button>
                </>
              )}
              <button
                type="button"
                className="btn btn-secondary"
                onClick={async () => {
                  await monitorApi.remove(it.id).catch((e) => setError(toAppError(e)));
                  if (open?.id === it.id) setOpen(null);
                  refresh();
                }}
              >
                Stop monitoring
              </button>
            </span>
          </li>
        ))}
      </ul>
      {error && <ErrorPanel title="Monitoring problem" error={error} />}
      {open && (
        <div className="panel">
          <h3 className="section-title">What changed since the snapshot</h3>
          <CompareResult c={open.c} />
        </div>
      )}
    </section>
  );
}
