import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";

import { toAppError, watchApi, type AppError, type WatchEvent, type WatchInfo } from "../api";
import { AlertIcon, CalendarIcon, CheckIcon, CloseIcon, EyeIcon } from "../icons";
import { ErrorPanel, Spinner } from "./common";

/**
 * Folder watch: notices new PDFs and text files in one folder (e.g.
 * Downloads). Reading them automatically is a standing permission the user
 * chooses explicitly; it's shown the whole time and ends when watching stops.
 */
export function WatchPanel({ onOpen }: { onOpen: (path: string) => void }) {
  const [info, setInfo] = useState<WatchInfo | null>(null);
  const [choosing, setChoosing] = useState<string | null>(null); // folder path awaiting the consent choice
  const [events, setEvents] = useState<WatchEvent[]>([]);
  const [error, setError] = useState<AppError | null>(null);
  const [saved, setSaved] = useState<Record<number, string>>({});

  useEffect(() => {
    const un = listen<WatchEvent>("watch-event", (e) =>
      setEvents((list) => {
        const rest = list.filter((x) => !(x.watchId === e.payload.watchId && x.seq === e.payload.seq));
        return [e.payload, ...rest].slice(0, 20);
      }),
    );
    return () => {
      un.then((f) => f());
    };
  }, []);

  async function pickFolder() {
    setError(null);
    const picked = await open({ multiple: false, directory: true, title: "Choose a folder to watch (for example, Downloads)" });
    if (typeof picked === "string") setChoosing(picked);
  }

  async function start(autoRead: boolean) {
    const path = choosing!;
    setChoosing(null);
    try {
      setInfo(await watchApi.start(path, autoRead));
      setEvents([]);
      setSaved({});
    } catch (e) {
      setError(toAppError(e));
    }
  }

  async function stop() {
    await watchApi.stop();
    setInfo(null);
  }

  const mine = info ? events.filter((e) => e.watchId === info.watchId) : [];

  return (
    <section className="watch" aria-label="Folder watch">
      {!info ? (
        <div className="watch-bar">
          <EyeIcon width={16} height={16} />
          <span className="muted small">Want MyFile to notice new syllabi as they download?</span>
          <button type="button" className="btn btn-secondary" onClick={pickFolder}>
            Watch a folder…
          </button>
        </div>
      ) : (
        <div className="watch-bar is-on" role="status">
          <span className="watch-dot" aria-hidden="true" />
          <span>
            Watching <strong>{info.folder}</strong> ·{" "}
            {info.autoRead ? (
              <span className="warn-text">auto-reading ON: new documents are sent to Claude</span>
            ) : (
              <span className="muted">notify only, nothing is sent</span>
            )}
          </span>
          <button type="button" className="btn btn-secondary" onClick={stop}>
            <CloseIcon width={14} height={14} /> Stop
          </button>
        </div>
      )}
      {error && <ErrorPanel title="Couldn't watch that folder" error={error} />}

      {choosing && <WatchConsent folder={choosing} onChoose={start} onCancel={() => setChoosing(null)} />}

      {mine.length > 0 && (
        <ul className="watch-inbox">
          {mine.map((e) => (
            <li key={e.seq} className="watch-item">
              <span className="watch-icon" aria-hidden="true">
                {e.status === "reading" ? (
                  <Spinner />
                ) : e.status === "done" ? (
                  <CheckIcon width={16} height={16} />
                ) : e.status === "error" ? (
                  <AlertIcon width={16} height={16} />
                ) : (
                  <EyeIcon width={16} height={16} />
                )}
              </span>
              <span className="watch-body">
                <strong>{e.name}</strong>
                <span className="muted small">{e.message}</span>
              </span>
              <span className="watch-actions">
                {e.calendarEvents > 0 && (
                  <button
                    type="button"
                    className="btn btn-secondary"
                    onClick={async () => {
                      try {
                        const name = await watchApi.saveCalendar(e.watchId, e.seq);
                        if (name) setSaved((m) => ({ ...m, [e.seq]: name }));
                      } catch (err) {
                        setError(toAppError(err));
                      }
                    }}
                  >
                    <CalendarIcon width={14} height={14} /> {saved[e.seq] ? "Saved" : `Add ${e.calendarEvents} to calendar`}
                  </button>
                )}
                <button type="button" className="btn btn-secondary" onClick={() => onOpen(e.path)}>
                  Open
                </button>
              </span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

function WatchConsent({ folder, onChoose, onCancel }: { folder: string; onChoose: (auto: boolean) => void; onCancel: () => void }) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    if (ref.current && !ref.current.open) ref.current.showModal();
  }, []);
  const name = folder.split(/[\\/]/).filter(Boolean).pop() ?? folder;
  return (
    <dialog
      ref={ref}
      className="dialog"
      aria-labelledby="watch-title"
      onCancel={(e) => {
        e.preventDefault();
        onCancel();
      }}
    >
      <h2 id="watch-title">Watch “{name}”?</h2>
      <p>
        While MyFile is open, it will notice new PDFs and text files that appear in this folder (not its subfolders).
        Files still downloading are ignored until they finish.
      </p>
      <ul className="dialog-list">
        <li>
          <strong>Read them automatically:</strong> each new document's text is sent to Anthropic's Claude to find
          deadlines, and you get a notification with what it found.
        </li>
        <li>
          <strong>Just notify me:</strong> nothing is sent; you decide what to open.
        </li>
        <li>Watching stops when you press Stop or close MyFile. Nothing about it is saved.</li>
      </ul>
      <div className="dialog-buttons">
        <button type="button" className="btn btn-secondary" onClick={onCancel}>
          Cancel
        </button>
        <button type="button" className="btn btn-secondary" onClick={() => onChoose(false)}>
          Just notify me
        </button>
        <button type="button" className="btn btn-primary" onClick={() => onChoose(true)}>
          Read them automatically
        </button>
      </div>
    </dialog>
  );
}
