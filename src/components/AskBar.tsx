import { useState } from "react";

import { routeRequest, toAppError, type AppError, type FileKind, type RouteDecision } from "../api";
import { SparkIcon } from "../icons";
import { ErrorPanel, Spinner } from "./common";

const EXAMPLES: Record<FileKind, string> = {
  pdf: "e.g. what do I need to do this week?",
  text: "e.g. who is responsible for what?",
  image: "e.g. make this small enough to email",
  audio: "e.g. turn this into text",
  video: "e.g. make subtitles for this",
  unsupported: "",
};

interface Props {
  kind: FileKind;
  selectionId: number;
  disabled: boolean;
  /** Called with a token from onStart, so the parent can ignore stale decisions. */
  onStart: () => number;
  onDecision: (d: RouteDecision, token: number) => void;
}

/**
 * "Ask MyFile": a typed request that the model maps to one allowlisted action.
 * Only the sentence and the file type are sent — not the file.
 */
export function AskBar({ kind, selectionId, disabled, onStart, onDecision }: Props) {
  const [text, setText] = useState("");
  const [state, setState] = useState<
    { status: "idle" } | { status: "thinking" } | { status: "done"; d: RouteDecision } | { status: "error"; error: AppError }
  >({ status: "idle" });

  if (kind === "unsupported") return null;

  async function submit() {
    if (!text.trim()) return;
    setState({ status: "thinking" });
    const token = onStart();
    try {
      const d = await routeRequest(selectionId, text);
      setState({ status: "done", d });
      if (d.action !== "none") onDecision(d, token);
    } catch (e) {
      setState({ status: "error", error: toAppError(e) });
    }
  }

  const thinking = state.status === "thinking";

  return (
    <section className="ask" aria-label="Ask MyFile">
      <form
        className="ask-row"
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
      >
        <span className="ask-icon" aria-hidden="true">
          <SparkIcon />
        </span>
        <input
          className="ask-input"
          value={text}
          onChange={(e) => setText(e.target.value)}
          placeholder={`Tell MyFile what you want, ${EXAMPLES[kind]}`}
          aria-label="Tell MyFile what you want to do with this file"
          maxLength={500}
          disabled={thinking || disabled}
        />
        <button type="submit" className="btn btn-primary" disabled={thinking || disabled || !text.trim()}>
          {thinking ? "Thinking…" : "Go"}
        </button>
      </form>
      <p className="muted small ask-note">
        Only what you type and the file type are sent to Claude to choose an action, never the file. Any document
        reading still asks for permission first.
      </p>

      {thinking && (
        <p className="result-status" role="status">
          <Spinner /> Choosing an action…
        </p>
      )}
      {state.status === "error" && <ErrorPanel title="MyFile couldn't act on that" error={state.error} />}
      {state.status === "done" && <Decision d={state.d} />}
    </section>
  );
}

function Decision({ d }: { d: RouteDecision }) {
  return (
    <div className={`decision ${d.action === "none" ? "decision-none" : ""}`} role="status">
      <p>
        <span className="tag">AI</span> {d.reason || (d.action === "none" ? "That isn't something MyFile can do." : "")}
      </p>
      {d.adjusted.map((a, i) => (
        <p key={i} className="small decision-adjusted">
          MyFile adjusted the AI's choice: {a}
        </p>
      ))}
      <p className="muted small">
        Chosen by {d.model} in {(d.usage.apiMs / 1000).toFixed(1)} s · ≈${d.usage.costUsd.toFixed(4)} at API list price ·
        checked against MyFile's list of allowed actions
      </p>
    </div>
  );
}
