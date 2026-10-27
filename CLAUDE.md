# CLAUDE.md

Guidance for AI coding agents working on MyFile.

> MyFile is a desktop file utility that helps users perform practical file operations, including conversion, extraction, and processing, with AI assisting where it adds value while application code controls supported actions, validation, and execution.

Judge every feature, architectural change and development decision against that statement. MyFile helps people work with their files; it is not a general chatbot or a document-summarization app.

## Project

Tauri 2 desktop app (Windows + macOS): a file converter first. Drop a file, see what it can become, convert, save. Drop a folder to save everything in it as one file. The other tools sit under "More tools for this file/folder". React 19 + TypeScript + Vite frontend in `src/`, Rust backend in `src-tauri/src/`. See README.md for the product and the AI-vs-code split.

## Rules

- **Preserve the existing architecture and working functionality.** Prefer focused, maintainable changes over rewrites, and don't refactor unrelated code while doing a focused change.
- **Don't invent features, capabilities, completed work, or test results.** A test is "passed" only if it ran and its result supports that. Report tests that couldn't run, and why.
- **Never advertise unimplemented actions.** An action appears in `actions.rs` (and the UI and README) only once it works end to end.
- **Keep AI connected to a specific file task.** Don't keep growing summarization or general Q&A because it's easy to add.
- **Treat AI output as untrusted input.** Validate it against a schema and the allowlist in code; never let it supply paths, commands, or actions outside the allowlist. The AI provider must really be called: no mock or hardcoded responses presented as AI output.
- **Application code owns the list of supported operations and runs them.** The AI only recommends; see "AI boundaries".
- **No new dependencies or scope without a clear reason**, stated in the change and the log.
- **Ask before destructive or consequential file operations.** MyFile has no delete operation. Outputs are new files saved where the user chooses, with confirmation before overwriting. **Never modify the user's original file.**
- **File access goes through Rust commands that act on the current selected file or folder** (`AppState`). Don't add `tauri-plugin-fs` or broaden capabilities without a specific need.
- **Never build shell command strings** from filenames or document content. Use `std::process::Command` with an argument list.
- **Cross-platform:** no hardcoded paths, and no Windows- or macOS-only APIs outside clearly isolated `#[cfg(...)]` blocks.
- **Keep `src/api.ts` types in sync with the Rust structs** (serde uses camelCase).
- **Keep tests and docs current.** Add or update tests with each behavior change, run them, and update README.md and the UI text so they match what the app really does.
- **Log every change.** `LOG.md` is a short first-person build log (dated sections, then "Issues"), like the other artifacts' logs. Add a bullet for each meaningful change: what was done, decisions, test results, and problems (or that there were none). It's a course requirement.
- **Commits:** focused, no AI attribution lines, no secrets or generated files. Don't commit, push, or rewrite history without the user's go-ahead.

## AI boundaries

The AI is an assistant, not the authority on what runs. Every AI-assisted operation follows these steps:

1. Code detects the file type and the operations supported for it (`convert::targets`, `route::allowed`).
2. Code gives the AI only the approved operation ids and the context it needs.
3. The AI answers in the required JSON schema.
4. Code validates the answer against the approved operations and this file's capabilities.
5. Code checks parameters, paths, file types and other constraints.
6. The user confirms: an AI pick is only pre-selected, and nothing runs until the user clicks.
7. Only the application's own code performs the operation.
8. Success or failure is reported from the real result.

Never run an operation just because the AI asked for it.

**Conversions ("describe it", `convert::suggest_conversion`):**
- **What the model sees:** the file type, the size, this file's available conversion ids, and the request. Never the file's name or contents (`suggestion_prompt`).
- **What `check_suggestion` rejects:**
  - malformed answers: missing, extra or wrong-typed fields (`bad_ai_response`);
  - unknown ids, and ids that exist but aren't available for this file or whose tool isn't installed.
  - Settings are snapped to the values the interface offers.
- **What `run()` checks again:** that the target is a route for this format, that its tool is installed, and that the settings are in range (`Settings::check`).
- **On an invalid answer,** say so and let the user pick from the list. Never substitute a different operation.

**File contents are untrusted data.**
- Document text goes to the model only inside `<document>` tags, with the instruction that it is data, not instructions (`ai.rs`, `folder.rs`).
- Document text can never change the allowlist, permissions, or what runs. `ai::validate` flags quotes that address the AI.
- Prompt wording is not a security boundary; the checks above live in code, and filesystem access stays limited to what each command needs.

## Scope check for new features

Before building a feature, ask:
1. Does it help users perform, discover or manage file operations?
2. Does it improve an existing workflow or meet a clear need?
3. Is AI really useful here, or would plain code be more reliable?
4. Does code still control execution?
5. What complexity, dependencies, risks or upkeep does it add?
6. Can a concrete use case and tests show its value?

Then classify it:
- **Aligned:** directly supports the purpose.
- **Supporting:** strengthens an existing file workflow.
- **Potential scope drift:** moves MyFile toward a different product. Explain why, and suggest a more focused alternative before building it.

Current features:

| Class | Features |
|---|---|
| Aligned | Converter (all formats and engines), "describe it" conversion pick, folder reports ("everything as one file"), PDF text extraction, image compress/resize, OCR, transcription, duplicate finder |
| Supporting | Extraction templates (CSV/JSON export), version comparison, change monitor, "show in document", MCP server |
| Potential scope drift (keep, don't expand) | AI document assistant (summarize, simplify, Q&A), semester timeline, agent mode, fact-check, ask across a folder. Search/"ask your files" and folder watch have backend code but are **not shown in the current UI** |

## New feature requests

For each request:
1. Restate the user benefit in one sentence and how it serves the purpose.
2. Read the relevant code.
3. List the affected files, dependencies, validation rules and tests, plus the risks.
4. Propose the smallest complete change, and ask first if it changes scope or is consequential.
5. Build only that.
6. Run the tests and report the real results.
7. Update the docs and `LOG.md`.

Small, clearly scoped changes don't need this ceremony.

## Evaluation (`eval/`)

- **The cases:** `cases.json` has the document assistant's cases and `conversion-cases.json` has the conversion suggester's. `docs/` holds generated test PDFs; `make-docs.mjs <names>` regenerates only the named ones.
- **Results files:** `RESULTS.md` and `CONVERSION-RESULTS.md` are written by the live runs. `TESTS.md` lists the automated tests behind each guideline, with their purpose, input, expected and observed results.
- **Keep useful cases and add to them.** Never swap real cases for trivial ones.
- **Label how each result was obtained:** automated, live AI, or manual. Nothing in the suite is mocked AI output presented as a live result.
- **A passing eval is evidence about the tested cases**, not proof of security.

## Modules

- `convert/`: the converter.
  - `mod.rs`: formats, detection, routes, commands, and the checked AI suggestion (`suggestion_prompt`, `check_suggestion`).
  - `media.rs`: images, ffmpeg audio/video, and subtitles.
  - `docs.rs`: PDF rendering (hayro) and documents.
  - `data.rs`: data and spreadsheets.
  - `archive.rs`: archives.
  - `office.rs`: runs LibreOffice and pandoc.
  - `pdfwrite.rs` and `docx.rs`: write PDFs and Word files.
  - `report.rs`: reads every file in a folder and saves it all as one Word/PDF/Markdown/HTML/text/JSON file, or a CSV file list. No AI.
  - A conversion appears only when its engine is installed.
- `ai.rs`: prompts, the Claude CLI provider, and `validate`. Every model answer goes through `validate`: quote check, page fixes, date cross-check, date-in-own-quote check, and injection detection.
- `local.rs`: the Ollama provider. It uses the same prompt, schema, and `validate` as Claude, and it is the default wherever it is available.
- `route.rs`: typed request to one allowlisted "More tools" action.
- `folder.rs`: folder mode (opening a folder) and the semester tools (timeline, conflicts, crunch weeks, cross-document ask).
- `agent.rs`: plan, approve, run, briefing. Only a stored plan can run.
- `watch.rs`: folder watch with explicit standing consent (not shown in the current UI).
- `mcp.rs` and `bin/myfile-mcp.rs`: the MCP server. Paths stay inside the root, and writes only create new files.
- `compare.rs`: the diff engine and the optional explanation (points must cite real change ids).
- `monitor.rs`: snapshots, fingerprints, and the file watcher for monitored documents.
- `factcheck.rs`: claim verdicts. A definite verdict needs verified evidence with that stance.
- `extract.rs`: extraction templates. It checks per cell: type, quote, and number or date in the quote.
- `duplicates.rs`: duplicate and version finder. No AI, and it never deletes.
- `ocr.rs`: Tesseract CLI plus extraction of PDF page images.
- `transcribe.rs`: speech to text (Symphonia or ffmpeg decoding, whisper.cpp, model download with SHA-256 check). A transcript becomes the selected file's `Document`.
- `search.rs`: the search index (SQLite FTS5 plus optional Ollama embeddings) and "ask your files". Claude gets only passages stored by `prepare_ask`. Not shown in the current UI.
- `evidence.rs`: "show in document". It finds spans with the quote-check normalization, and only for the open file, the open folder, or indexed paths.
- `eval.rs`: the live evaluations (`eval_live`, `eval_conversion_live`; both `#[ignore]`).

## More rules

- **New model features keep the same shape.** The model makes one bounded decision, and code checks it. Add a deterministic check and a unit test for any new failure mode the evaluation finds.
- **Folder requests register with `folder::cancel_token`.** Never clear the whole cancel map.
- **Release builds set `GGML_NATIVE=OFF`** (plus AVX2/FMA/F16C on x86) so whisper.cpp runs on other CPUs.
- **When following a link would be wrong, read file types with `symlink_metadata`.** MCP, watch and folder reports never follow links.

## Commands

```sh
npm run build                     # tsc + vite build (frontend check)
cd src-tauri && cargo test        # backend unit tests
npm run tauri dev                 # run the app (needs CMake and LIBCLANG_PATH for whisper.cpp)
# Live evaluations (real model calls; EVAL_MODELS=haiku,sonnet,local:qwen2.5:3b)
cd src-tauri && cargo test eval_conversion_live -- --ignored --nocapture
cd src-tauri && cargo test eval_live -- --ignored --nocapture
```
