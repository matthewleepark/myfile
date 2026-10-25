import { useState } from "react";

import { api, toAppError, type AppError } from "../api";
import { AlertIcon, CheckIcon, CopyIcon, SaveIcon } from "../icons";

export function ErrorPanel({ title, error }: { title: string; error: AppError }) {
  return (
    <div className="error-panel" role="alert">
      <AlertIcon />
      <div>
        <p className="error-title">{title}</p>
        <p>{error.message}</p>
      </div>
    </div>
  );
}

export function Spinner() {
  return <span className="spinner" aria-hidden="true" />;
}

type Feedback = { ok: true; text: string } | { ok: false; text: string } | null;

/** Copy and Save buttons for a result held by the backend. */
export function OutputButtons({ outputId, copyable }: { outputId: number; copyable: boolean }) {
  const [feedback, setFeedback] = useState<Feedback>(null);
  const [saving, setSaving] = useState(false);

  async function copy() {
    try {
      const text = await api.outputText(outputId);
      await navigator.clipboard.writeText(text);
      setFeedback({ ok: true, text: "Copied to the clipboard." });
    } catch (e) {
      setFeedback({ ok: false, text: `Couldn't copy: ${toAppError(e).message}` });
    }
  }

  async function save() {
    setSaving(true);
    try {
      const name = await api.saveOutput(outputId);
      setFeedback(name ? { ok: true, text: `Saved as “${name}”.` } : null);
    } catch (e) {
      setFeedback({ ok: false, text: toAppError(e).message });
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="output-bar">
      {copyable && (
        <button type="button" className="btn btn-secondary" onClick={copy}>
          <CopyIcon width={16} height={16} /> Copy
        </button>
      )}
      <button type="button" className="btn btn-secondary" onClick={save} disabled={saving}>
        <SaveIcon width={16} height={16} /> {saving ? "Choosing location…" : "Save as…"}
      </button>
      {feedback && (
        <p className={`feedback ${feedback.ok ? "is-ok" : "is-bad"}`} role="status">
          {feedback.ok ? <CheckIcon width={16} height={16} /> : <AlertIcon width={16} height={16} />}
          {feedback.text}
        </p>
      )}
    </div>
  );
}
