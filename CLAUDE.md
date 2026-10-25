# CLAUDE.md

Guidance for AI coding agents working on MyFile.

## Project

Tauri 2 desktop app (Windows + macOS): a file converter first. Drop a file, see what it can become, convert, save. The other tools sit under "More tools for this file". React 19 + TypeScript + Vite frontend in `src/`, Rust backend in `src-tauri/src/`. See README.md for the product and the AI-vs-code split.

## Rules

- **Log every change.** Append an entry to `LOG.md` (date, plan, what changed, what was verified) for each meaningful piece of work. It's a course requirement.
- **Never advertise unimplemented actions.** An action appears in `actions.rs` only once it works end to end.
- **File access goes through Rust commands that act on the current selected file** (`AppState`). Don't add `tauri-plugin-fs` or broaden capabilities without a specific need.
- **Never modify the user's original file.** Outputs are new files saved where the user chooses, with confirmation before overwriting.
- **The AI provider must really be called.** No mock or hardcoded responses presented as AI output. Treat model output as untrusted: validate it against a schema, and never let it supply paths, commands, or actions outside the allowlist.
- **Never build shell command strings** from filenames or document content. Use `std::process::Command` with an argument list.
- **Cross-platform:** no hardcoded paths, and no Windows- or macOS-only APIs outside clearly isolated `#[cfg(...)]` blocks.
- **Keep `src/api.ts` types in sync with the Rust structs** (serde uses camelCase).
- **Commits:** no AI attribution lines.

## Modules

- `convert/`: the converter. `mod.rs` holds formats, detection, routes, commands and the checked AI suggestion; `media.rs` handles images, ffmpeg audio/video and subtitles; `docs.rs` handles PDF rendering (hayro) and documents; `data.rs` handles data and spreadsheets; `archive.rs` handles archives; `office.rs` runs LibreOffice and pandoc; `pdfwrite.rs` writes PDFs. A conversion appears only when its engine is installed.
- `ai.rs`: prompts, the Claude CLI provider, and `validate`. Every model answer goes through `validate`: quote check, page fixes, date cross-check, date-in-own-quote check, and injection detection.
- `local.rs`: the Ollama provider. It uses the same prompt, schema, and `validate` as Claude, and it is the default wherever it is available.
- `route.rs`: typed request to one allowlisted action.
- `folder.rs`: semester brain (timeline, conflicts, crunch weeks, cross-document ask).
- `agent.rs`: plan, approve, run, briefing. Only a stored plan can run.
- `watch.rs`: folder watch with explicit standing consent.
- `mcp.rs` and `bin/myfile-mcp.rs`: the MCP server. Paths stay inside the root, and writes only create new files.
- `compare.rs`: the diff engine and the optional explanation (points must cite real change ids).
- `monitor.rs`: snapshots, fingerprints, and the file watcher for monitored documents.
- `factcheck.rs`: claim verdicts. A definite verdict needs verified evidence with that stance.
- `extract.rs`: extraction templates. It checks per cell: type, quote, and number or date in the quote.
- `duplicates.rs`: duplicate and version finder. No AI, and it never deletes.
- `ocr.rs`: Tesseract CLI plus extraction of PDF page images.
- `transcribe.rs`: speech to text (Symphonia or ffmpeg decoding, whisper.cpp, model download with SHA-256 check). A transcript becomes the selected file's `Document`.
- `search.rs`: the search index (SQLite FTS5 plus optional Ollama embeddings) and "ask your files". Claude gets only passages stored by `prepare_ask`.
- `evidence.rs`: "show in document". It finds spans with the quote-check normalization, and only for the open file, the open folder, or indexed paths.
- `eval.rs`: the live evaluation (`#[ignore]`).

## More rules

- **New model features keep the same shape.** The model makes one bounded decision, and code checks it. Add a deterministic check and a unit test for any new failure mode the evaluation finds.
- **Folder requests register with `folder::cancel_token`.** Never clear the whole cancel map.
- **Release builds set `GGML_NATIVE=OFF`** (plus AVX2/FMA/F16C on x86) so whisper.cpp runs on other CPUs.
- **When following a link would be wrong, read file types with `symlink_metadata`.** MCP and watch never follow links.

## Commands

```sh
npm run build                     # tsc + vite build (frontend check)
cd src-tauri && cargo test        # backend unit tests
npm run tauri dev                 # run the app (needs CMake and LIBCLANG_PATH for whisper.cpp)
```
