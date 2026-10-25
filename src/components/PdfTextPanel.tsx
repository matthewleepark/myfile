import { useEffect, useState } from "react";

import { api, toAppError, type AppError, type ExtractedText } from "../api";
import { ErrorPanel, OutputButtons, Spinner } from "./common";

export function PdfTextPanel({ onBusy }: { onBusy: (busy: boolean) => void }) {
  const [state, setState] = useState<
    { status: "loading" } | { status: "done"; result: ExtractedText } | { status: "error"; error: AppError }
  >({ status: "loading" });

  useEffect(() => {
    onBusy(true);
    api
      .extractPdfText()
      .then((result) => setState({ status: "done", result }))
      .catch((e) => setState({ status: "error", error: toAppError(e) }))
      .finally(() => onBusy(false));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  if (state.status === "loading") {
    return (
      <p className="result-status" role="status">
        <Spinner /> Extracting text on this computer…
      </p>
    );
  }
  if (state.status === "error") return <ErrorPanel title="Couldn't extract the text" error={state.error} />;

  const r = state.result;
  return (
    <>
      {r.ocr && (
        <p className="warn-line">
          This text was read with OCR from {r.pageCount > 1 ? "scanned pages" : "a picture"}, on this computer. OCR can misread
          words or numbers, so check anything important against the original.
        </p>
      )}
      <p className="success-line" role="status">
        Extracted {r.charCount.toLocaleString()} characters from {r.pageCount} page{r.pageCount === 1 ? "" : "s"}.
        {r.emptyPages > 0 &&
          ` ${r.emptyPages} page${r.emptyPages === 1 ? " has" : "s have"} no readable text (possibly images or scans).`}
      </p>
      <pre className="text-preview tall" tabIndex={0} aria-label="Extracted text">
        {r.text}
      </pre>
      {r.truncatedForDisplay && (
        <p className="muted small">The text is long, so only the beginning is shown. Copy and Save include all of it.</p>
      )}
      <OutputButtons outputId={r.outputId} copyable />
    </>
  );
}
