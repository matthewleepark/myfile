import { useEffect, useState } from "react";

import { api, toAppError, type AppError, type TextPreview } from "../api";
import { ErrorPanel, Spinner } from "./common";

export function TextPreviewPanel() {
  const [state, setState] = useState<
    { status: "loading" } | { status: "done"; preview: TextPreview } | { status: "error"; error: AppError }
  >({ status: "loading" });

  useEffect(() => {
    api
      .previewText()
      .then((preview) => setState({ status: "done", preview }))
      .catch((e) => setState({ status: "error", error: toAppError(e) }));
  }, []);

  if (state.status === "loading") {
    return (
      <p className="result-status">
        <Spinner /> Reading the file…
      </p>
    );
  }
  if (state.status === "error") return <ErrorPanel title="Couldn't preview the file" error={state.error} />;

  return (
    <>
      <pre className="text-preview" tabIndex={0} aria-label="File preview">
        {state.preview.text}
      </pre>
      {state.preview.truncated && (
        <p className="muted small">Showing the first 64 KB. The rest of the file isn't shown here.</p>
      )}
    </>
  );
}
