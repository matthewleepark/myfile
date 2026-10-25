import { useEffect, useRef, useState } from "react";

import { api, extractApi, folderApi, toAppError, type AppError, type FieldType, type Template, type TemplateTable } from "../api";
import { AlertIcon, CheckIcon, CloseIcon, SaveIcon } from "../icons";
import { ErrorPanel, Spinner } from "./common";

const TYPES: FieldType[] = ["text", "date", "money", "number", "boolean"];
const TYPE_LABEL: Record<FieldType, string> = { text: "Text", date: "Date", money: "Money", number: "Number", boolean: "Yes/No" };

interface Props {
  scope: "file" | "folder";
  selectionId: number;
}

/**
 * Custom extraction templates. The AI fills in named fields with quotes;
 * MyFile checks each value's type, its quote, and that numbers and dates
 * really appear in that quote.
 */
export function TemplatePanel({ scope, selectionId }: Props) {
  const [templates, setTemplates] = useState<Template[]>([]);
  const [chosen, setChosen] = useState<string>("syllabus");
  const [editing, setEditing] = useState(false);
  const [provider, setProvider] = useState<"local" | "claude">("claude");
  const [localModel, setLocalModel] = useState<string | null>(null);
  const [ready, setReady] = useState(false);
  const [consented, setConsented] = useState(false);
  const [asking, setAsking] = useState(false);
  const [state, setState] = useState<
    { status: "idle" } | { status: "running" } | { status: "done"; t: TemplateTable } | { status: "error"; error: AppError }
  >({ status: "idle" });

  const reload = () => extractApi.list().then(setTemplates, () => {});
  useEffect(() => {
    reload();
    api
      .localStatus()
      .then((l) => {
        if (l.available) {
          setLocalModel(l.model);
          setProvider("local");
        }
      }, () => {})
      .finally(() => setReady(true));
  }, []);

  const template = templates.find((t) => t.id === chosen);

  async function run() {
    if (provider === "claude" && !consented) {
      setAsking(true);
      return;
    }
    setState({ status: "running" });
    try {
      const t =
        scope === "file"
          ? await extractApi.file(selectionId, chosen, provider)
          : await extractApi.folder(selectionId, chosen, provider);
      setState({ status: "done", t });
    } catch (e) {
      setState({ status: "error", error: toAppError(e) });
    }
  }

  async function grant() {
    setAsking(false);
    try {
      if (scope === "file") await api.grantAiConsent(selectionId);
      else await folderApi.grantConsent(selectionId);
      setConsented(true);
      setState({ status: "running" });
      const t =
        scope === "file"
          ? await extractApi.file(selectionId, chosen, "claude")
          : await extractApi.folder(selectionId, chosen, "claude");
      setState({ status: "done", t });
    } catch (e) {
      setState({ status: "error", error: toAppError(e) });
    }
  }

  return (
    <div className="template-panel">
      <div className="ai-run-row">
        <label className="field-label" htmlFor="tpl">
          Template
        </label>
        <select id="tpl" value={chosen} onChange={(e) => setChosen(e.target.value)} disabled={state.status === "running"}>
          {templates.map((t) => (
            <option key={t.id} value={t.id}>
              {t.name}
              {t.builtIn ? "" : " (yours)"}
            </option>
          ))}
        </select>
        <button type="button" className="btn btn-secondary" onClick={() => setEditing((v) => !v)}>
          {editing ? "Close editor" : "New template…"}
        </button>
        {template && !template.builtIn && (
          <button
            type="button"
            className="btn btn-secondary"
            onClick={async () => {
              await extractApi.remove(template.id);
              setChosen("syllabus");
              reload();
            }}
          >
            Delete template
          </button>
        )}
      </div>
      {template && (
        <p className="chip-row">
          {template.fields.map((f) => (
            <span key={f.key} className="doc-chip">
              {f.label} · {TYPE_LABEL[f.type]}
            </span>
          ))}
        </p>
      )}
      {editing && (
        <TemplateEditor
          onSaved={(t) => {
            setEditing(false);
            reload().then(() => setChosen(t.id));
          }}
        />
      )}

      <div className="ai-run-row">
        {localModel && (
          <div className="segmented" role="radiogroup" aria-label="Which model fills the template?">
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
              >
                {label}
              </button>
            ))}
          </div>
        )}
        <button type="button" className="btn btn-primary" onClick={run} disabled={!ready || !template || state.status === "running"}>
          {scope === "file" ? "Fill the template" : "Fill it for every document"}
        </button>
      </div>
      {asking && <Confirm scope={scope} onAccept={grant} onCancel={() => setAsking(false)} />}
      {state.status === "running" && (
        <p className="result-status" role="status">
          <Spinner /> {scope === "folder" ? "Reading each document…" : "Reading the document…"}
        </p>
      )}
      {state.status === "error" && <ErrorPanel title="Couldn't fill the template" error={state.error} />}
      {state.status === "done" && <ResultTable t={state.t} />}
    </div>
  );
}

function ResultTable({ t }: { t: TemplateTable }) {
  const [saved, setSaved] = useState<string | null>(null);
  const counts = t.rows.flatMap((r) => r.cells).reduce(
    (acc, c) => ({ ...acc, [c.status]: (acc[c.status] ?? 0) + 1 }),
    {} as Record<string, number>,
  );
  const save = async (id: number) => {
    const name = await api.saveOutput(id).catch(() => null);
    if (name) setSaved(name);
  };
  return (
    <div className="template-result">
      <p className="small">
        <span className="ok-text">
          <CheckIcon width={14} height={14} /> {counts["verified"] ?? 0} verified
        </span>{" "}
        · <span className="warn-text">{(counts["unverified"] ?? 0) + (counts["wrong-type"] ?? 0)} need checking</span> ·{" "}
        <span className="muted">{counts["not-found"] ?? 0} not found</span>
      </p>
      <div className="table-wrap">
        <table className="tpl-table">
          <thead>
            <tr>
              <th>Document</th>
              {t.template.fields.map((f) => (
                <th key={f.key}>{f.label}</th>
              ))}
            </tr>
          </thead>
          <tbody>
            {t.rows.map((r) => (
              <tr key={r.document}>
                <th scope="row">{r.document}</th>
                {r.error ? (
                  <td colSpan={t.template.fields.length} className="warn-text">
                    {r.error}
                  </td>
                ) : (
                  r.cells.map((c) => (
                    <td
                      key={c.key}
                      className={`cell-${c.status}`}
                      title={[c.quote ? `“${c.quote}”${c.page ? ` (p. ${c.page})` : ""}` : "", c.problem ?? ""].filter(Boolean).join("\n")}
                    >
                      {c.status === "not-found" ? (
                        <span className="muted">not found</span>
                      ) : (
                        <>
                          {c.status === "verified" ? <CheckIcon width={12} height={12} /> : <AlertIcon width={12} height={12} />} {c.value}
                          {c.problem && <span className="cell-problem">{c.problem}</span>}
                        </>
                      )}
                    </td>
                  ))
                )}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <p className="muted small">Hover a cell to see the quote it came from.</p>
      <div className="output-bar">
        <button type="button" className="btn btn-secondary" onClick={() => save(t.csvOutputId)}>
          <SaveIcon width={16} height={16} /> Export CSV
        </button>
        <button type="button" className="btn btn-secondary" onClick={() => save(t.jsonOutputId)}>
          <SaveIcon width={16} height={16} /> Export JSON
        </button>
        {saved && (
          <p className="feedback is-ok" role="status">
            <CheckIcon width={16} height={16} /> Saved as “{saved}”.
          </p>
        )}
      </div>
      <p className="muted small ai-meta">
        Filled by {t.model} in {(t.durationMs / 1000).toFixed(1)} s ·{" "}
        {t.model.endsWith("(local)") ? "free, on this computer" : `≈$${t.usage.costUsd.toFixed(4)} at API list price`}
      </p>
    </div>
  );
}

function TemplateEditor({ onSaved }: { onSaved: (t: Template) => void }) {
  const [name, setName] = useState("");
  const [fields, setFields] = useState<{ label: string; type: FieldType }[]>([{ label: "", type: "text" }]);
  const [error, setError] = useState<AppError | null>(null);
  const set = (i: number, patch: Partial<{ label: string; type: FieldType }>) =>
    setFields((fs) => fs.map((f, k) => (k === i ? { ...f, ...patch } : f)));
  return (
    <div className="tpl-editor">
      <label className="field-label" htmlFor="tpl-name">
        Template name
      </label>
      <input id="tpl-name" className="text-input" value={name} onChange={(e) => setName(e.target.value)} placeholder="e.g. Internship posting" maxLength={60} />
      <p className="field-label">Fields</p>
      {fields.map((f, i) => (
        <div key={i} className="tpl-field">
          <input
            className="text-input"
            value={f.label}
            onChange={(e) => set(i, { label: e.target.value })}
            placeholder="e.g. Application deadline"
            aria-label={`Field ${i + 1} name`}
            maxLength={60}
          />
          <select value={f.type} onChange={(e) => set(i, { type: e.target.value as FieldType })} aria-label={`Field ${i + 1} type`}>
            {TYPES.map((t) => (
              <option key={t} value={t}>
                {TYPE_LABEL[t]}
              </option>
            ))}
          </select>
          <button
            type="button"
            className="btn btn-icon"
            aria-label="Remove field"
            onClick={() => setFields((fs) => fs.filter((_, k) => k !== i))}
            disabled={fields.length === 1}
          >
            <CloseIcon />
          </button>
        </div>
      ))}
      <div className="output-bar">
        <button type="button" className="btn btn-secondary" onClick={() => setFields((fs) => [...fs, { label: "", type: "text" }])} disabled={fields.length >= 20}>
          Add field
        </button>
        <button
          type="button"
          className="btn btn-primary"
          onClick={async () => {
            try {
              onSaved(await extractApi.save({ name, fields: fields.filter((f) => f.label.trim()) }));
            } catch (e) {
              setError(toAppError(e));
            }
          }}
        >
          Save template
        </button>
      </div>
      {error && <ErrorPanel title="Couldn't save the template" error={error} />}
    </div>
  );
}

function Confirm({ scope, onAccept, onCancel }: { scope: "file" | "folder"; onAccept: () => void; onCancel: () => void }) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    if (ref.current && !ref.current.open) ref.current.showModal();
  }, []);
  return (
    <dialog
      ref={ref}
      className="dialog"
      aria-labelledby="tpl-consent"
      onCancel={(e) => {
        e.preventDefault();
        onCancel();
      }}
    >
      <h2 id="tpl-consent">Send {scope === "folder" ? "these documents" : "this document"} to Claude?</h2>
      <p>
        To fill the template, MyFile will send the text of {scope === "folder" ? "each document in the folder" : "this document"} to{" "}
        <strong>Anthropic's Claude</strong>. The local model keeps everything on this computer instead.
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
