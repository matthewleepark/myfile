import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";

import {
  agentApi,
  api,
  folderApi,
  toAppError,
  type AgentPlan,
  type AgentRun,
  type AgentStepResult,
  type AgentTool,
  type AppError,
  type FolderSummary,
} from "../api";
import { AlertIcon, CheckIcon, SaveIcon, SparkIcon } from "../icons";
import { ErrorPanel, Spinner } from "./common";

const TOOL_LABELS: Record<AgentTool, string> = {
  find_deadlines: "Find deadlines",
  summarize: "Summarize",
  ask: "Ask across documents",
  find_conflicts: "Find conflicts",
  build_calendar: "Build calendar",
  compress_image: "Compress image",
};

const EXAMPLES = [
  "Get my week ready",
  "Put every deadline on my calendar and warn me about clashes",
  "Summarize each class and make the photos small enough to email",
];

type State =
  | { status: "idle" }
  | { status: "planning" }
  | { status: "plan"; plan: AgentPlan }
  | { status: "running"; plan: AgentPlan; startedAt: number }
  | { status: "done"; plan: AgentPlan; run: AgentRun }
  | { status: "error"; error: AppError };

/**
 * Agent mode: you give a goal, Claude proposes a plan from MyFile's six
 * tools (seeing only file names), you approve it, MyFile runs it step by step,
 * and Claude writes a short briefing from the validated results.
 */
export function AgentPanel({ folder }: { folder: FolderSummary }) {
  const [goal, setGoal] = useState("");
  const [state, setState] = useState<State>({ status: "idle" });
  const [live, setLive] = useState<Record<number, AgentStepResult>>({});
  const [saved, setSaved] = useState<Record<number, string>>({});

  useEffect(() => {
    const un = listen<AgentStepResult>("agent-step", (e) => setLive((m) => ({ ...m, [e.payload.index]: e.payload })));
    return () => {
      un.then((f) => f());
    };
  }, []);

  async function plan(g = goal) {
    if (!g.trim()) return;
    setGoal(g);
    setState({ status: "planning" });
    try {
      setState({ status: "plan", plan: await agentApi.plan(folder.selectionId, g) });
    } catch (e) {
      setState({ status: "error", error: toAppError(e) });
    }
  }

  async function run(p: AgentPlan) {
    setLive({});
    setSaved({});
    setState({ status: "running", plan: p, startedAt: Date.now() });
    try {
      setState({ status: "done", plan: p, run: await agentApi.run(p.planId) });
    } catch (e) {
      setState({ status: "error", error: toAppError(e) });
    }
  }

  const p = state.status === "plan" || state.status === "running" || state.status === "done" ? state.plan : null;
  const results: Record<number, AgentStepResult> =
    state.status === "done" ? Object.fromEntries(state.run.steps.map((s) => [s.index, s])) : live;

  return (
    <section className="panel" aria-label="Agent mode">
      <h3 className="section-title">Agent mode</h3>
      <p className="muted small">
        Say what you want done with this folder. Claude plans it using only MyFile's six tools, sees only file names
        while planning, and nothing runs until you approve the plan.
      </p>
      <form
        className="ask-row"
        onSubmit={(e) => {
          e.preventDefault();
          plan();
        }}
      >
        <span className="ask-icon" aria-hidden="true">
          <SparkIcon />
        </span>
        <input
          className="ask-input"
          value={goal}
          onChange={(e) => setGoal(e.target.value)}
          placeholder="e.g. Get my week ready"
          aria-label="Goal for this folder"
          maxLength={500}
          disabled={state.status === "planning" || state.status === "running"}
        />
        <button type="submit" className="btn btn-primary" disabled={!goal.trim() || state.status === "planning" || state.status === "running"}>
          {state.status === "planning" ? "Planning…" : "Make a plan"}
        </button>
      </form>
      {state.status === "idle" && (
        <div className="chip-row">
          {EXAMPLES.map((ex) => (
            <button key={ex} type="button" className="chip" onClick={() => plan(ex)}>
              {ex}
            </button>
          ))}
        </div>
      )}
      {state.status === "planning" && (
        <p className="result-status" role="status">
          <Spinner /> Claude is planning from the list of {folder.docs.length + folder.images.length} files…
        </p>
      )}
      {state.status === "error" && <ErrorPanel title="The agent stopped" error={state.error} />}

      {p && (
        <div className="agent-plan">
          <p className="agent-summary">
            <span className="tag">AI plan</span> {p.summary}
          </p>
          <ol className="agent-steps">
            {p.steps.map((s) => {
              const r = results[s.index];
              return (
                <li key={s.index} className={`agent-step ${r ? "is-" + r.status : ""}`}>
                  <span className="agent-step-icon" aria-hidden="true">
                    {r?.status === "running" ? <Spinner /> : r?.status === "done" ? <CheckIcon width={16} height={16} /> : r?.status === "error" ? <AlertIcon width={16} height={16} /> : s.index + 1}
                  </span>
                  <span className="agent-step-body">
                    <span className="agent-step-title">
                      {TOOL_LABELS[s.tool]}
                      {s.target && <span className="doc-chip">{s.tool === "ask" ? `“${s.target}”` : s.target}</span>}
                      {s.usesAi && <span className="tag">AI</span>}
                    </span>
                    <span className="muted small">{s.reason}</span>
                    {r && r.status !== "running" && <span className={`agent-result ${r.status === "error" ? "warn-text" : ""}`}>{r.message}</span>}
                    {r?.outputId != null && (
                      <span>
                        <button
                          type="button"
                          className="btn btn-secondary"
                          onClick={async () => {
                            const name = await api.saveOutput(r.outputId!).catch(() => null);
                            if (name) setSaved((m) => ({ ...m, [s.index]: name }));
                          }}
                        >
                          <SaveIcon width={14} height={14} /> {r.outputLabel ?? "Save"}
                        </button>
                        {saved[s.index] && <span className="feedback is-ok"> Saved as “{saved[s.index]}”.</span>}
                      </span>
                    )}
                  </span>
                </li>
              );
            })}
          </ol>
          {p.adjusted.length > 0 && (
            <div className="checks" role="note">
              <p className="checks-title">MyFile's checks on this plan</p>
              <ul>
                {p.adjusted.map((a, i) => (
                  <li key={i}>{a}</li>
                ))}
              </ul>
            </div>
          )}

          {state.status === "plan" && (
            <>
              <p className="small">
                {p.docsSent.length > 0 ? (
                  <>
                    Running this plan sends the text of <strong>{p.docsSent.join(", ")}</strong> to Anthropic's Claude.
                    Approving is your permission.
                  </>
                ) : (
                  <>This plan doesn't send any document text to Claude.</>
                )}
              </p>
              <div className="dialog-buttons" style={{ justifyContent: "flex-start", marginTop: 0 }}>
                <button type="button" className="btn btn-primary" onClick={() => run(p)}>
                  Approve and run
                </button>
                <button
                  type="button"
                  className="btn btn-secondary"
                  onClick={() => {
                    agentApi.discard();
                    setState({ status: "idle" });
                  }}
                >
                  Discard
                </button>
              </div>
              <p className="muted small">
                Planned by {p.model} · ≈${p.usage.costUsd.toFixed(4)} at API list price
              </p>
            </>
          )}
          {state.status === "running" && (
            <div className="ai-waiting" role="status">
              <Spinner />
              <span>Running the approved plan…</span>
              <button type="button" className="btn btn-secondary" onClick={() => folderApi.cancel()}>
                Stop
              </button>
            </div>
          )}
          {state.status === "done" && (
            <>
              {state.run.briefing && (
                <div className="briefing">
                  <p className="checks-title" style={{ color: "var(--accent)" }}>
                    Briefing
                  </p>
                  <p className="answer-text" style={{ whiteSpace: "pre-line" }}>
                    {state.run.briefing}
                  </p>
                </div>
              )}
              <p className="muted small">
                Ran {state.run.steps.length} steps in {(state.run.durationMs / 1000).toFixed(1)} s ·{" "}
                {state.run.usage.inputTokens.toLocaleString()} tokens in · ≈${state.run.usage.costUsd.toFixed(4)} at API list price.
                The briefing was written only from the validated results above.
              </p>
            </>
          )}
        </div>
      )}
    </section>
  );
}
