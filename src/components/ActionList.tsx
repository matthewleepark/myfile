import type { ActionId, ActionInfo, FileKind } from "../api";
import { CompareIcon, EyeIcon, MicIcon, TableIcon, PdfTextIcon, ShrinkIcon, SparkIcon } from "../icons";

interface Props {
  kind: FileKind;
  actions: ActionInfo[];
  activeId: ActionId | null;
  busy: boolean;
  onPick: (id: ActionId) => void;
}

export function ActionIcon({ id }: { id: ActionId }) {
  switch (id) {
    case "ai-assistant":
      return <SparkIcon />;
    case "compress-image":
      return <ShrinkIcon />;
    case "extract-pdf-text":
    case "read-image-text":
      return <PdfTextIcon />;
    case "preview-text":
      return <EyeIcon />;
    case "compare-document":
      return <CompareIcon />;
    case "monitor-document":
      return <EyeIcon />;
    case "extract-fields":
      return <TableIcon />;
    case "transcribe":
      return <MicIcon />;
  }
}

export function ActionList({ kind, actions, activeId, busy, onPick }: Props) {
  if (actions.length === 0) {
    return (
      <section className="actions" aria-label="Actions">
        <h3 className="section-title">Actions</h3>
        <p className="muted">
          {kind === "unsupported"
            ? "MyFile can't do anything with this type of file yet. It works with PDFs, text files, and common images."
            : "There are no actions for this file yet."}
        </p>
      </section>
    );
  }

  return (
    <section className="actions" aria-label="Actions">
      <h3 className="section-title">What do you want to do?</h3>
      <ul className="action-grid">
        {actions.map((a) => (
          <li key={a.id}>
            <button
              type="button"
              className={`action-card${activeId === a.id ? " is-active" : ""}`}
              onClick={() => onPick(a.id)}
              disabled={busy && activeId !== a.id}
              aria-pressed={activeId === a.id}
            >
              <span className="action-icon">
                <ActionIcon id={a.id} />
              </span>
              <span className="action-text">
                <span className="action-label">
                  {a.label}
                  {a.usesAi && <span className="tag">AI</span>}
                </span>
                <span className="action-desc">{a.description}</span>
              </span>
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}
