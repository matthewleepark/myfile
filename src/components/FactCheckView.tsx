import type { FactCheck } from "../api";
import { EvidenceButton, type EvidenceSource } from "./EvidenceViewer";
import { AlertIcon, CheckIcon } from "../icons";

const LABEL: Record<FactCheck["verdict"], string> = {
  supported: "Supported",
  contradicted: "Contradicted",
  ambiguous: "Ambiguous",
  "not-found": "Not found in these documents",
  unverified: "Couldn't verify",
};

const STANCE: Record<FactCheck["evidence"][number]["stance"], string> = {
  supports: "supports",
  contradicts: "contradicts",
  context: "context",
};

/** One fact-check result: MyFile's verdict (after checking evidence), then the evidence. */
export function FactCheckView({ r, sourceFor }: { r: FactCheck; sourceFor?: (source: string | null) => EvidenceSource | null }) {
  const downgraded = r.modelVerdict !== r.verdict;
  return (
    <article className="ai-answer factcheck" aria-label="Fact-check result">
      <p className="factcheck-claim">
        <span className="muted small">Claim</span>
        <br />“{r.claim}”
      </p>
      <p className={`verdict verdict-${r.verdict}`} role="status">
        {r.verdict === "supported" ? <CheckIcon width={18} height={18} /> : <AlertIcon width={18} height={18} />}
        {LABEL[r.verdict]}
        {downgraded && <span className="small verdict-note"> (the AI said “{LABEL[r.modelVerdict].toLowerCase()}”)</span>}
      </p>
      {r.explanation && <p className="answer-text">{r.explanation}</p>}
      {r.missing && (
        <p className="muted small">
          <strong>What would settle it:</strong> {r.missing}
        </p>
      )}
      {r.evidence.length > 0 && (
        <ul className="ai-items">
          {r.evidence.map((e, i) => (
            <li key={i} className="ai-item">
              <div className="ai-item-head">
                <span className={`tag ${e.stance === "contradicts" ? "tag-warn" : ""}`}>{STANCE[e.stance]}</span>
                {e.source && <span className="doc-chip">{e.source}</span>}
              </div>
              <blockquote className={`quote ${e.quoteCheck === "verified" ? "is-verified" : "is-unverified"}`}>
                “{e.quote}”
                <footer>
                  {e.quoteCheck === "verified" ? (
                    <>
                      <CheckIcon width={14} height={14} /> Found in the document{e.page != null && `, page ${e.page}`}
                      {sourceFor?.(e.source) && (
                        <>
                          {" "}
                          · <EvidenceButton source={sourceFor(e.source)!} quote={e.quote} page={e.page} />
                        </>
                      )}
                    </>
                  ) : (
                    <>
                      <AlertIcon width={14} height={14} /> Not found in any document; not counted as evidence
                    </>
                  )}
                </footer>
              </blockquote>
            </li>
          ))}
        </ul>
      )}
      {r.notes.length > 0 && (
        <div className="checks" role="note">
          <p className="checks-title">MyFile's checks on this verdict</p>
          <ul>
            {r.notes.map((n, i) => (
              <li key={i}>{n}</li>
            ))}
          </ul>
        </div>
      )}
      <p className="muted small ai-meta">
        Checked by {r.model} in {(r.durationMs / 1000).toFixed(1)} s ·{" "}
        {r.model.endsWith("(local)") ? "free, on this computer" : `≈$${r.usage.costUsd.toFixed(4)} at API list price`}
      </p>
    </article>
  );
}
