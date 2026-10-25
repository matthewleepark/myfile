import { useState } from "react";

import { duplicatesApi, toAppError, type AppError, type DuplicateGroup, type DuplicateReport } from "../api";
import { formatSize } from "../format";
import { LockIcon } from "../icons";
import { ErrorPanel, Spinner } from "./common";

const KIND_LABEL: Record<DuplicateGroup["kind"], string> = {
  identical: "Identical copies",
  "same-text": "Same text, different files",
  versions: "Two versions",
  "similar-images": "Look-alike images",
};

/**
 * Duplicate detection for the open folder. Runs entirely in code on this
 * computer; MyFile only lists what it found and never deletes anything.
 */
export function DuplicatesPanel({ selectionId }: { selectionId: number }) {
  const [state, setState] = useState<
    { status: "idle" } | { status: "running" } | { status: "done"; r: DuplicateReport } | { status: "error"; error: AppError }
  >({ status: "idle" });

  async function run() {
    setState({ status: "running" });
    try {
      setState({ status: "done", r: await duplicatesApi.find(selectionId) });
    } catch (e) {
      setState({ status: "error", error: toAppError(e) });
    }
  }

  return (
    <section className="panel" aria-label="Find duplicates">
      <h3 className="section-title">Find duplicates and old versions</h3>
      <p className="muted small">
        <LockIcon width={14} height={14} /> Runs on this computer without AI. MyFile only lists what it finds; it never deletes or
        moves a file.
      </p>
      <div className="output-bar">
        <button type="button" className="btn btn-secondary" onClick={run} disabled={state.status === "running"}>
          {state.status === "done" ? "Check again" : "Find duplicates"}
        </button>
      </div>
      {state.status === "running" && (
        <p className="result-status" role="status">
          <Spinner /> Comparing files…
        </p>
      )}
      {state.status === "error" && <ErrorPanel title="Couldn't check for duplicates" error={state.error} />}
      {state.status === "done" && <Report r={state.r} />}
    </section>
  );
}

function Report({ r }: { r: DuplicateReport }) {
  if (r.groups.length === 0) {
    return (
      <p className="success-line" role="status">
        No duplicates among {r.filesChecked} files.
      </p>
    );
  }
  return (
    <div className="dup-groups">
      <p className="small" role="status">
        Found {r.groups.length} group{r.groups.length === 1 ? "" : "s"} among {r.filesChecked} files
        {r.reclaimableBytes > 0 && <> · identical copies take up {formatSize(r.reclaimableBytes)} extra</>}.
      </p>
      {r.groups.map((g, i) => (
        <div key={i} className={`dup-group dup-${g.kind}`}>
          <p className="dup-head">
            <span className="dup-kind">{KIND_LABEL[g.kind]}</span>
            {g.similarity !== null && g.kind !== "same-text" && <span className="doc-chip">{g.similarity}% alike</span>}
          </p>
          <ul className="dup-files">
            {g.files.map((f) => (
              <li key={f.name}>
                <span className="dup-name">{f.name}</span>
                <span className="muted small">
                  {formatSize(f.sizeBytes)}
                  {f.modified !== null && <> · {new Date(f.modified * 1000).toLocaleString()}</>}
                </span>
                {f.newest && <span className="doc-chip">newest</span>}
              </li>
            ))}
          </ul>
          <p className="small">{g.detail}</p>
          {g.dateChanges.length > 0 && (
            <ul className="date-changes">
              {g.dateChanges.map((d) => (
                <li key={d}>Date moved: {d}</li>
              ))}
            </ul>
          )}
        </div>
      ))}
    </div>
  );
}
