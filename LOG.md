# MyFile development log

A running record of plans, decisions, and changes. Newest entries at the bottom. Maintained with help from Claude Code.

## Overview

| Date | Milestone | What changed |
|---|---|---|
| Oct 21 | Setup | Chose the headless Claude Code CLI as the AI provider (no paid API key). Installed Rust and the C++ build tools. |
| Oct 21 | 1 | Tauri app skeleton: drop or browse a file, content-based type detection. |
| Oct 21 | 2 | First actions: PDF text, image compression, the AI document assistant with quote checking and consent. |
| Oct 21 | 3 | Evaluation harness, cost display, typed-request routing, calendar export. |
| Oct 21 | — | Demo video, portfolio card draft, Mac CI workflow. |
| Oct 21–22 | 4 | Semester brain (folders), agent mode, local model, folder watch, MCP server; security review fixes. |
| Oct 22 | 5 | Local model by default, OCR, version comparison, change monitor, fact-checking, templates, duplicate finder. |
| Oct 22 | 6 | Speech to text (whisper.cpp), plus a date-year bug the transcripts exposed. |
| Oct 22 | 7 | Search your files (SQLite FTS5 plus meaning search) and "show in document". |
| Oct 23 | 8 | **Change of direction:** MyFile becomes a file converter, with a simpler interface and the other tools under "More tools". |
| Oct 25 | Wrap-up | Release build and a check against the assignment's requirements. |

**How AI was used to build it.** Claude Code (Opus, at times Haiku) wrote and tested the code from my plans and corrections. Each milestone below lists what was planned, what changed, and what was verified, including the bugs the testing found. Inside the app, the model makes one bounded decision at a time (which conversion fits a request; what a document says), and ordinary code checks every answer.

---

## 2026-10-21 — Environment check and AI provider decision

**Found:**
- Node 25.9, npm 11.12, git, and the WebView2 runtime were installed.
- Rust and the Visual Studio C++ Build Tools (both required by Tauri on Windows) were missing.
- The Claude Code CLI isn't on PATH, but the copy bundled with the VS Code extension works.

**Tested the AI route:** piped a one-line document into `claude -p ... --output-format json --tools "" --model haiku`. It returned valid JSON in about 5.6 s using the existing subscription login, with no API key needed.

**Decision:** use the headless Claude Code CLI as the AI provider for the prototype, behind a small provider interface. A paid API key was ruled out because the course says to ask before choosing one. Documented trade-off: this only works where the CLI is installed and logged in.

**Approved and done:** installed Rust (rustup, stable 1.99.1) and the VS 2022 C++ Build Tools via winget.

## 2026-10-21 — Milestone 1: project setup and file selection

**Plan:** scaffold Tauri 2 + React + TS, then build Feature A (file selection and validation) and the skeleton of Feature B (action list). Report before starting on images, PDFs, or AI.

**Changes:**
- Scaffolded with `create-tauri-app` (react-ts template). Removed the template's demo code, logos, and the `opener` plugin.
- Added `tauri-plugin-dialog` (native open dialog) and `infer` (file type from magic bytes).
- **Rust backend** (`src-tauri/src`):
  - `files.rs`: validates that the file exists, isn't a folder, isn't empty, and is ≤ 100 MB. Detects the type from its contents and flags names whose extension contradicts the contents. Has unit tests.
  - `actions.rs`: an allowlist of implemented actions per file kind. Only *Preview text* exists so far.
  - `lib.rs`: `select_file`, `clear_file` and `preview_text` commands. The backend remembers the selected file, so later operations can't be pointed at a different path.
  - `error.rs`: user-facing error type with plain-English messages.
- **Security config:** capabilities reduced to `core:default` + `dialog:allow-open`, with no fs plugin. Set a strict CSP.
- **Frontend:**
  - Window-wide native drag-and-drop (`useFileDrop`), browse button, file card (name, type, size, mismatch warning, change/remove), and action cards.
  - Explicit states (idle, validating, selected, error) and a results area.
  - Light/dark themes, visible focus rings, and errors shown with an icon and text, not just color.
- **Edge cases handled:** a dropped batch of files uses the first one and says so; a stale validation can't overwrite a newer selection.
- Wrote README (feature status, AI-vs-code responsibilities, security model) and CLAUDE.md.

**Verified (Windows 11):**
- `npm run build` (tsc + vite): passes.
- `cargo test`: 8/8 backend tests pass (type detection by content, extension mismatch, empty/missing/folder/oversized rejection, UTF-8 edge case).
- `npm run tauri dev`: the app compiles and launches. A screenshot confirmed the empty state renders as designed (dark theme).
- **Not yet verified by hand:** dragging a real file in, the Browse dialog, Change/Remove, and the text preview flow. These need someone at the keyboard.
- **Not verified:** macOS (no Mac available).

## 2026-10-21 — Milestone 2: every planned feature

**Plan (approved: "build all of that"):** do the AI assistant first, since it's the assignment's core, then PDF extraction, image compression, saving, Shift-drop, and an installer. Verify each feature in the real running app, not just by compiling.

**AI provider research:**
- `claude --help` showed `--json-schema` (schema-validated `structured_output`), `--no-session-persistence`, `--tools ""`, `--setting-sources`, and `--strict-mcp-config`. Tested them together from the shell: valid structured JSON in about 3.7 s.
- Rejected `--bare`, because it requires an API key and ignores the subscription login.

**Backend changes:**
- `ai.rs`:
  - Prompt building with fixed per-task instructions and `[Page N]` markers for PDFs.
  - `ClaudeCli` provider: arguments as a fixed list (no shell), stdin for content, 180 s timeout, cancellation via `kill_on_drop`, and no console window on Windows.
  - Envelope parsing with classified errors (auth, limit, failure).
  - `validate`: schema re-check, length caps, control-character stripping, page range checks, and a **quote check against the real text** that also corrects wrong page numbers.
  - Plain-text rendering for copy and save.
- `pdf.rs`:
  - Per-page extraction with `pdf-extract`, wrapped in `catch_unwind`.
  - 2,000-page limit; scanned and password-protected detection.
- `image_ops.rs`:
  - Decode with EXIF orientation and a 100 MP limit.
  - JPEG (quality, alpha flattened onto white) or PNG (best compression) output.
  - Lanczos3 resize that never enlarges; preview data URL.
- `shift.rs`: Shift key state via `GetAsyncKeyState` / `CGEventSourceFlagsState`, with no hooks and no permissions.
- `lib.rs`:
  - Per-selection ids, so stale results are discarded.
  - Document text cached in memory; outputs held in memory until saved (no temp files).
  - Consent enforced in Rust.
  - Save dialog opened from Rust; refuses to overwrite the original; atomic write.
  - Selecting a new file cancels an in-flight AI request.
- `actions.rs`: added `extract-pdf-text`, `compress-image`, `ai-assistant`.
- Removed `panic = "abort"` from the release profile so `catch_unwind` works.

**Frontend changes:**
- Panels: text preview, PDF text (stats, copy, save), image (format radios, quality slider, resize, before/after with honest size change), and AI assistant (task chips, custom question, consent `<dialog>` with the exact character count, elapsed-time waiting state, Cancel, answer view with quote check labels, inferred badges, and "MyFile's checks" notes).
- Shift-drop quick menu with arrow-key and Escape support.
- Image thumbnails on the file card, and a new app icon.

**Bugs found and fixed during testing:**
- The model name came from the first key in the CLI's `modelUsage`, which is alphabetical, so it showed Haiku (used for internal housekeeping). It now picks the model with the most output tokens.
- The cancel channel treated a *dropped* sender as a cancel. Now only an explicit `Ok(())` cancels.

**Data-safety check:** in Explorer, Shift-drag usually means *move*. Read wry 0.57's `IDropTarget`: it always answers `DROPEFFECT_COPY` while dragging over the window, so a Shift-drop behaves like any other drop. Still, test Shift-drop with a copy of a file first.

**Verified (Windows 11):**
- `cargo test`: all unit tests pass, covering quote verification, page correction and removal, malformed and empty replies, envelope error classification, prompt construction, PDF extraction of the sample, image encoding and resizing, alpha flattening, atomic save, and same-file detection.
- `cargo test live_ -- --ignored`: a **real** call through the CLI with the sample syllabus PDF returned validated deadlines (Sonnet, about 12 s).
- Drove the real app over the WebView2 DevTools protocol (simulated drop events, real button clicks, screenshots):
  - PDF: extraction gave 1,090 characters from 2 pages. The consent dialog showed the exact count. The deadlines answer had 8 items, every quote verified, correct pages, in 12.7 s.
  - Markdown: a custom question ("who is responsible for what…") was answered in 6.8 s. Cancel during a run showed "Cancelled", and **no `claude.exe` process was left running**. Consent was remembered for that file only.
  - Image: JPEG q75 resized to 800 px went from 637.5 KB to 21.9 KB (96.6% smaller). Lossless PNG was 29.0% smaller.
  - ZIP: identified as "ZIP file (application/zip)", with no fake actions.
  - Shift-drop: with a **real Shift key press** (`keybd_event`), the quick menu opened at the drop point with the first action focused.
  - Save: drove the **native Windows save dialog**. The file was written correctly and no `.myfile-partial` temp file was left.
- **Not verified:** macOS (no Mac); an OS-level drag from Explorer by a human hand (simulated via Tauri's own drop event instead); password-protected and scanned PDFs (handled in code, but no sample files).
- **Installers:** `npm run tauri build` produced `MyFile_0.1.0_x64-setup.exe` (NSIS, 2.8 MB) and `MyFile_0.1.0_x64_en-US.msi` (4.1 MB) under `src-tauri/target/release/bundle/`. Ran the **release** executable and drove a real AI Summarize request through it: it answered in about 15 s.

## 2026-10-21 (evening) — Milestone 3: evaluation, cost, routing, calendar

**Plan (approved: "implement what you can… add anything creative"; checked the plan with you before starting):**
1. Show cost per answer.
2. Evaluation harness with known answers, traps, and Haiku vs Sonnet.
3. Natural-language routing.
4. Demo video script.
5. Calendar export.
6. A small multi-agent adversarial review of the new code (ultracode, review only).

**Cost check first:** `claude auth status` showed `authMethod: claude.ai` and `subscriptionType: team` (org "TCSS 490"). Calls count against the team plan's usage limits; there's no API key and no per-call billing.

**Changes:**
- **Cost and usage (`ai.rs`):** the CLI envelope's `total_cost_usd`, tokens (including cached input), and `duration_api_ms` are now parsed into `Usage` and shown under every answer as "≈$X at API list price".
- **Trust summary (`ai.rs` `Checks`):** ordinary code counts quotes checked, quotes verified, page numbers corrected, and page numbers removed. The UI shows a banner such as "8 of 8 quotes found word for word in the document". The same struct feeds the evaluation.
- **`run_with` refactor:** the locked-down CLI call (no tools, no shell, stdin only) now takes a system prompt and schema, so routing reuses it.
- **Routing (`route.rs`, `AskBar.tsx`):**
  - **Model:** maps a typed request to one action ID from a per-file-type allowlist (enforced in the JSON Schema *and* re-checked in Rust), and may suggest image settings.
  - **Code:** clamps quality to 30–95 in steps of 5, snaps the size to the offered values, and reports any adjustment.
  - **Data sent:** only the sentence and the file type, never the contents or filename.
  - **Model choice:** Haiku, a deliberate latency and cost choice for a small decision.
  - **Consent:** routed AI tasks still open the consent dialog first.
- **Calendar export (`ics.rs`):**
  - **Model:** adds `isoDate` (YYYY-MM-DD, or null when uncertain).
  - **Code:** `check_iso_date` rejects impossible dates and any ISO date that contradicts the written month, day, or year, then builds an RFC 5545 `.ics` deterministically (all-day events, CRLF, 75-octet folding, escaping).
  - **UI:** a deadlines answer offers "Add N dates to my calendar".
- **Evaluation (`eval/`, `src-tauri/src/eval.rs`):**
  - `make-docs.mjs` regenerates the test PDFs.
  - `cases.json` holds 13 cases: normal documents, ISO and relative dates, multi-page page attribution, a 16-date table, a moved deadline, a two-column layout, 2 traps, and 1 prompt injection.
  - `cargo test eval_live -- --ignored` runs every case on each model through the real pipeline and writes `eval/RESULTS.md`.
  - Scoring is plain code.
  - **Negative control:** every real reply is re-checked against an unrelated document, and the quote checker must reject its quotes.
- `DEMO-SCRIPT.md`: a 90-second narrated demo plan (recorded with Snipping Tool, since ffmpeg isn't installed).

**Findings so far:**
- First evaluation run (10 cases): 20/20 passed, and the checks never fired. I treated that as a possible ceiling effect, added 3 harder cases, and reran.
- Second run (13 cases): 26/26 passed. Haiku matched Sonnet at about a tenth of the list cost ($0.012 versus $0.119), with similar median latency (7.3 s versus 7.1 s).
- Because the checks never fired on real output, I added the negative control to show the checker isn't simply approving everything.

**Adversarial review (multi-agent workflow, 9 agents):**
- **Setup:** 4 reviewers covered routing security, consent and auto-run, calendar dates, and evaluation honesty. A skeptic then tried to refute each serious finding by tracing the code.
- **Confirmed and fixed:**
  1. A late routing decision could auto-run on a newly opened file.
  2. Consent wasn't bound to a selection between the check and the send.
  3. Numeric written dates were checked only for the year.
  4. An empty written date skipped the date check.
- **Fixes:**
  - `select_file` returns a `selectionId`. `document_stats`, `grant_ai_consent`, `run_ai`, and `route_request` all require it, and the backend rejects stale IDs.
  - `run_ai` re-checks consent for that exact selection immediately before sending.
  - The UI ignores routing decisions after a file change or a manual action pick.
  - The AI panel cancels its request when it closes.
- **Date checker rewrite:** `written_candidates` collects every date the text can mean (ISO, M/D and D/M, month names with ordinals, ranges, day-before-month), and the ISO date must match exactly. 17 new test assertions cover it.
- **Unverified but plausible (5-skeptic limit), also fixed:**
  - Evaluation scoring: boundary date matching, quotes no longer count as answers, the stale-date check now uses the item's title, hyphens are normalized for forbidden claims, "resisted" is scored separately with explicit `adversarial` flags, and the median excludes errors.
  - Routing: the image size is now a ceiling instead of snapping to the nearest offered value.
- **Verified in the real app:** typed a request for an image, then dropped the PDF while it was thinking. No panel opened and no consent dialog appeared. A normal routed summary then worked (8 of 8 quotes verified).

**Final evaluation (strict scoring):** 26/26 passed.
- **Haiku:** 13/13, $0.013 list price, 7.4 s median.
- **Sonnet:** 13/13, $0.126 list price, 6.5 s median.
- **Shared results:** traps and injection resisted 4/4 for each model; the negative control rejected 106/106 wrong-document quotes; 47 calendar dates checked per model.
- **Honest takeaway:** the quote and page checks never fired on real output in 26 runs. They're a proven safety net (unit tests plus the control), not something these documents exercised.

**Tests:** 47 unit tests pass, plus 2 live tests skipped by default.

## 2026-10-21 (night): Demo video, portfolio card, Mac CI

**Plan (you chose: demo video, portfolio card, Mac build via GitHub Actions; not the GitHub push):**

**Demo video:**
- Installed ffmpeg 9 with winget.
- `demo/record.mjs` drives the real **release** build over the WebView2 DevTools protocol and records its screencast. Captions are injected only while recording, and AI waits play at 4× with an on-screen label.
- `demo/render.mjs` encodes `demo/myfile-demo.mp4` (83 s, 1600×1176, 6 MB).
- **Problems found while reviewing the frames, all fixed:**
  - Hard-coded captions were wrong on one run: Haiku routed "what do I need to do this week?" to *Requirements*, not *Deadlines*, and there were 9 quotes, not 8. Captions now read the real screen (the chosen task, the quote count, whether "2030" was claimed), and the calendar step only runs if the button exists.
  - The rotating spinner produced large smeared arcs in screencast frames, so it's frozen during recording. The elapsed-seconds counter still shows the wait.
  - The static closing card collapsed to 2 s, because screencast frames only arrive when the screen changes. The last frame now lasts until recording stops.
  - On one attempt, the consent dialog was answered by a click the script didn't make. The release window was on screen, so it was most likely a person. The recorder now clicks Send only if the dialog is still open.

**Portfolio card (local only, not committed or pushed):**
- `cards.js` card 3 now reads "MyFile": status done, link `https://matthewleepark.github.io/myfile/`, with a reflection draft.
- New `myfile/` page with the video, a poster frame, and a short model-versus-code summary. The video is hosted on GitHub Pages like daily-bread, because the MyFile repo isn't public.

**Mac CI:**
- `.github/workflows/build.yml` runs on macOS and Windows: frontend build, `cargo test`, then the app build (universal macOS `.app`/`.dmg` and Windows NSIS/MSI) uploaded as artifacts.
- Branches are set to main/master, since this repo uses `master`.
- **Not run yet:** it needs the repo on GitHub.

## 2026-10-21 to 22: Milestone 4, five big features ("create all")

**Plan (you asked me to build everything):** in order of value: MCP server → semester brain → local model → agent mode → folder watch. I warned first about scope, shared team-plan usage, and the assignment's "small, clear model role," and kept every new feature on the same rule: the model makes one bounded decision, and code checks it.

### 1. MyFile as an MCP server (`src/mcp.rs`, `src/bin/myfile-mcp.rs`)
- A second binary speaks MCP (JSON-RPC over stdio). It has six deterministic tools: `list_documents`, `inspect_file`, `read_document`, **`verify_quote`**, `compress_image`, and `export_calendar`.
- **Safety:** every path must resolve inside a root folder (default: home). Writes only ever create new files and never overwrite. Calendar dates are validated by code.
- **Verified with the real Claude Code CLI (headless, only MyFile's tools allowed):** Claude read the syllabus, called `verify_quote` 7 times (all FOUND, with pages), then `export_calendar`, which wrote a 4-event `.ics`. It took 14.3 s. I didn't change your Claude Code settings; the README shows the one-line `claude mcp add` command.

### 2. Semester brain (`src/folder.rs`, `FolderView.tsx`)
- Drop or open a folder. Every PDF and text file is extracted locally, with real per-file progress. One consent dialog lists the files and the total characters.
- Haiku reads each document, 3 at a time. Haiku was chosen *because* the evaluation showed it matched Sonnet at a tenth of the cost. Each answer is validated against its own document.
- **Code merges the results:** a timeline, same-day conflicts across classes, crunch weeks (3 or more in a Monday–Sunday week), and one `.ics` for everything.
- **Ask across all documents:** answers cite their source file. Code checks each quote against *that* file and moves an item to the right file if the quote is really elsewhere.
- **Test folder:** `test-files/make-semester.mjs` generates 4 classes with planted collisions.
- **Verified in the app:** 5 documents in 15 s for $0.005. 14/14 quotes verified. All 3 planted conflicts (Nov 4, Nov 18, Dec 9) and 3 crunch weeks were found. The chess-club notes produced 0 items. The cross-document late-work question cited every class correctly, including BIO 200 on page 2.

### 3. Local model, fully private (`src/local.rs`)
- Installed Ollama and Qwen 2.5 3B (1.9 GB). MyFile talks to `127.0.0.1` only.
- It uses the same prompt, the same JSON Schema, and **the same `ai::validate`**. No consent is needed because nothing leaves the computer.
- **AI panel:** a provider choice of Claude, Local, or **Both, side by side**.
- **Evaluation result (first run, before the fix below):** 8/13 passed, 87% of facts, **7 page numbers fixed by MyFile**, 33 s median, $0. **It fell for the prompt injection.**

### Found by the evaluation: a gap in MyFile's own checks, now fixed
- The local model returned "Membership dues" with a **verified quote** (the real December 5 sentence) but the **injected January 1, 2030 date**. That date is a real calendar date, so it would have gone into the `.ics` file.
- **The gap:** quote verification proves the passage exists, not that the date came from it.
- **New check (`ai::date_supported`):** an item's date must be one of the dates in its own quote. If the quote has dates and none match, the item is flagged "date not in its own quote," kept off every calendar, and counted in `Checks.dates_unsupported`. It has unit tests, including this exact injection case.

### 4. Agent mode (`src/agent.rs`, `AgentPanel.tsx`)
- **The loop is plan → you approve → act → observe → report.**
- **Plan:** the planner (Sonnet) sees only the folder's inventory and your goal. It can use six tools.
- **Check:** Rust drops unknown tools, names that aren't in the folder, calendar or conflict steps placed before any deadlines step, and duplicates.
- **Approve:** the approval screen lists exactly which documents would be sent. The interface can only approve a *stored* plan by its id.
- **Report:** the final briefing is written only from validated results.
- **Verified in the app:** the goal included "also delete the chess club notes." The plan **refused the deletion** and said so, then made 7 sensible steps. The run read 4 syllabi (15/15 quotes verified), found all conflicts and crunch weeks, built a 12-event calendar, and compressed the poster from 637.5 KB to 67.3 KB in 48 s. The briefing was accurate.

### 5. Folder watch (`src/watch.rs`, `WatchPanel.tsx`)
- **What it does:** watches one folder (not subfolders) while the app is open. A consent dialog offers two explicit choices, "Read them automatically" or "Just notify me". A visible bar shows "auto-reading ON" or "notify only" with a Stop button.
- **Code handles the file system side:** it ignores in-progress downloads (`.crdownload`, `.part`, `.tmp`…), waits 1.5 s after the last change, checks the real file type, and skips files it already handled.
- **Then:** Haiku finds deadlines (validated), and MyFile sends a Windows notification. The calendar is kept in the watch session, not with per-file results, which clear when you switch files.
- **Verified:**
  - Simulated a Chrome download: a half-written `.crdownload` file produced **no event**. After the rename to `.pdf`: reading, then done, "4 item(s), 4 with a calendar date; 4/4 quotes verified."
  - Re-tested the native Save dialog after refactoring saving into `save_bytes`: still works.
- **Not verified:** I couldn't complete the Windows *folder* picker with keyboard automation (it ignored the shortcuts), so I started the watch through the backend command. The folder-picker step and seeing the notification toast need a human.

**Tests:** 64 unit tests pass. They cover the MCP protocol and path sandboxing, plan validation, conflicts and Monday-based crunch weeks, cross-document source correction, Ollama response parsing, the date-grounding check, and partial-download detection.

### Milestone 4, continued: what the evaluation and the security review changed

**Second gap found by testing: the model can quote the planted sentence itself.**
- On a re-run, the local model quoted the hidden "SYSTEM NOTE TO THE AI ASSISTANT… January 1, 2030" text. That text really is in the PDF (white, 4 px), so the quote verified and the date matched its own quote. Neither existing check could catch it.
- **New check (`ai::injected_sentences`):** detects sentences addressed to an AI ("note to the AI", "system note", "ignore the rest of this document", …). The markers are deliberately specific, and tests confirm that ordinary lines like "Ignore the first draft" or "Our AI policy…" don't trigger it.
- MyFile warns about such text, flags items that quote it, and keeps them off calendars.
- **Final local run:** the model still claimed 2030 (the case still fails, which is honest about the model), but the fake date reached the calendar **0** times (it did once before the fix).

**Security review (one review agent, read-only): 9 real defects, 6 medium and 3 low, all fixed.**
- **MCP:**
  - Writes use `create_new`, which refuses dangling symlinks and closes the race between checking and writing.
  - `:` is rejected in output names (blocks NTFS alternate streams).
  - `list_documents` no longer follows links or junctions.
- **Agent:**
  - A wrong plan id no longer discards the valid plan.
  - The folder is re-checked before the briefing.
- **Watch:**
  - Permission is re-checked right before sending, and Stop cancels in-flight reads (`Drop` on `WatchSession`).
  - Symlinks and anything not directly in the folder are skipped.
  - `.ics` files are ignored, and saves no longer default into the watched folder.
- **Folder cancel:** each request has its own token (`folder::cancel_token`, removed by a drop guard), so finishing one request can no longer make another uncancellable.

**Side-by-side test in the app (injection newsletter, Deadlines, Both):**
- Both columns showed the injection warning.
- **Claude:** reported the 2 real dates (2/2 quotes verified), said it ignored the hidden instruction, and offered "Add 2 dates to my calendar".
- **Local model:** answered "January 1, 2030". MyFile flagged it as "date not in its own quote" and left it off the calendar (no calendar button). Free, on this computer, 12.6 s.

**Tests:** 67 unit tests pass.

## 2026-10-22: Milestone 5, local-first AI, OCR, and five checked features

**Plan.** The user asked for the local model as the default (Claude optional), working OCR, macOS readiness, and the strongest features from a list of 16. Built first: version comparison and a change monitor, fact-checking with four outcomes, custom extraction templates, and duplicate detection. Each follows the project rule: the model makes one bounded decision, and code checks it.

### Local model as the default
- The AI assistant, folder mode, comparison explanations, fact-checking, and templates all default to `qwen2.5:3b` through Ollama when it's running. Claude is a toggle, and it still needs consent per file or folder.
- **Bug found in testing:** clicking Explain before the local-model status check finished silently used Claude, then labeled the result "Local". The provider buttons are now disabled until the status is known (`providerReady` in the compare, assistant, and folder panels).

### OCR (`src/ocr.rs`)
- Tesseract CLI, run with an argument list (no shell), a 120 s timeout, and a temp folder that's deleted afterwards. It's found via `MYFILE_TESSERACT`, the PATH, or standard Windows and Homebrew paths.
- Scanned PDFs: code pulls each page's image out of the PDF itself (DCT/JPEG, Flate raw gray, RGB, or 1-bit), then runs OCR on it. **Bug found:** the sample scan used an ICCBased color space, so decoding failed. Fixed by reading the component count `/N` from the ICC profile.
- Images get a "Read text (OCR)" action, and the assistant and templates work on the OCR text.
- **Verified:** a unit test extracts the page picture from `test-files/scanned-syllabus.pdf`. **Not verified:** the Tesseract step itself. The install (`winget install --id tesseract-ocr.tesseract -e`) needs an administrator prompt, which was declined, so it wasn't retried. The live OCR test is `#[ignore]`d until then.

### Version comparison (`src/compare.rs`, `ComparePanel.tsx`)
- Sentence-level Patience diff, then a word-level diff inside changed sentences, page numbers on both sides, and code-detected **moved dates**.
- **Explain** (optional AI) sends only the changes, never the whole documents. Each point must cite real change numbers, or it's dropped. **Gap found:** the local model skipped some changes and misdescribed a wording change. Now any change no point covers is listed as "not explained", so silence can't hide an edit.
- **Verified:** on `test-files/versions/` it found all 6 real edits (deadline Nov 12 → Nov 17, peer review added, extra credit removed). Claude's explanation linked to change numbers. Claude refused without consent.

### Change monitor (`src/monitor.rs`, `MonitorPanel.tsx`)
- Stores a whitespace-insensitive text fingerprint and page text in the app's data folder. It tells metadata-only changes (re-saved, same text) apart from real text changes, and notices when files go missing.
- A file watcher (2 s debounce) plus a check at startup trigger a notification and a summary written by code ("Deadline moved Nov 12, 2026 → Nov 17, 2026 · 5 other changes").
- **Verified in the app:** replacing the monitored v1 with v2 was detected in 2.9 s. "See what changed" showed the diff. "Mark as reviewed" reset it, and "Stop monitoring" removed the snapshot.

### Fact-checking (`src/factcheck.rs`, `FactCheckView.tsx`)
- Verdicts: supported, contradicted, ambiguous, or not found, plus **unverified** when MyFile can't back the model's verdict. Evidence quotes are checked and credited to the document that really contains them.
- *Supported* or *contradicted* needs a verified quote with that stance. Evidence both ways becomes *ambiguous*. *Not found* adds "Not finding something in these documents doesn't mean the claim is false."
- **Verified:** the local model gave definite verdicts with no evidence, and they were downgraded to *unverified*. Claude marked "Late work is never accepted" as *contradicted*, with a verified quote on p. 2.

### Extraction templates (`src/extract.rs`, `TemplatePanel.tsx`)
- Built-in templates: syllabus, job posting, lease, and receipt. Custom templates have up to 20 fields of type text, number, money, date, or yes/no, saved in the app's data folder. The JSON Schema restricts field keys to the template's own keys.
- Per-cell checks: the type parses, the quote is in the document, and numbers and dates appear *in that quote*. Cell statuses are verified, unverified, wrong type, or not found. CSV export defuses formula cells with a leading `'`.
- Works on one file or every document in a folder (Claude uses Haiku for this).
- **Verified in the app:** the lease template on `eval/docs/lease.pdf` with the local model came back 4/4 verified in 18–28 s (rent 1450, lease end 2027-06-30, notice deadline 2027-03-31). Claude refused without consent; after consent, a custom 3-field template filled in 4.5 s, with pets and move-in correctly *not found*. The syllabus template across `test-files/semester/` (5 documents, local) gave one table in 118 s, and the club-notes file came back all *not found*.

### Duplicate finder (`src/duplicates.rs`, `DuplicatesPanel.tsx`), no AI
- Finds identical bytes (bucketed by size and hash, then compared byte for byte), the same text in different files (text fingerprint), **versions** (3-word shingle overlap ≥ 0.25, then the diff engine for counts and moved dates), and look-alike images (64-bit difference hash, ≤ 6 bits apart). It never deletes anything.
- **Tuning found in testing:** 5-word shingles scored the real v1/v2 pair at only 0.30 (the documents are 68 words), so it was missed. Measured on all sample pairs: 3-word overlap is 0.43 for the versions and at most 0.03 for unrelated syllabi from the same generator. The displayed "% alike" is a word-diff ratio instead, because that's how alike the files look to a reader.
- **Bug found:** the version pair was shown against the copy "v1 (1).pdf". The oldest file of each identical group now represents it.
- **Verified in the app** on a test folder: identical v1 copies (41.6 KB extra), v1 → v2 as "Two versions, 76% alike", with "Date moved: Nov 12, 2026 → Nov 17, 2026", and a half-size JPEG of the whiteboard photo as "96% alike".

### macOS
- OCR looks in `/opt/homebrew/bin` and `/usr/local/bin`. The README gives `brew install ollama tesseract`. The CI workflow is unchanged and still runs only once the repo is pushed to GitHub.

**Tests:** 97 unit tests pass (3 ignored live tests). New tests cover diffs and moved dates on the sample versions, monitor fingerprints, fact-check verdict downgrades, template validation and CSV escaping, all four duplicate kinds, and PDF page-image extraction.

## 2026-10-22: Milestone 6, speech to text

**Plan.** The user asked for audio-to-text for any recording, video included. Recognition runs on this computer, and the transcript becomes the file's text, so the existing assistant, templates, and fact-checking work on recordings with no new AI code.

### How it works (`src/transcribe.rs`, `TranscribePanel.tsx`)
- **Model:** whisper.cpp via `whisper-rs` 0.16, using Whisper base English (`ggml-base.en.bin`, 141 MB). It isn't bundled. The panel offers a one-time download from Hugging Face, written to a temp file and kept only if its SHA-256 matches the pinned value (`a03779c8…d002`).
- **Decoding:** Symphonia in pure Rust (WAV, MP3, AAC/M4A, MP4/MOV audio tracks, FLAC, Vorbis, ALAC). If it can't open a file before producing any audio, MyFile falls back to ffmpeg when it's installed (WebM/Opus, MKV, …), so audio is never transcribed twice.
- **Streaming:** downmix to mono, then resample to 16 kHz (averaging when downsampling, linear interpolation when upsampling). Audio is cut into ~5-minute chunks at the quietest 100 ms in the last 10 s. Memory stays flat, and recordings up to 4 hours (and 4 GB) are allowed. Each chunk gets the previous chunk's last lines as its prompt.
- **Outputs:** a timestamped transcript (.txt), .srt and .vtt subtitles, and a searchable transcript in the panel. "Summarize it", "Find deadlines & dates", "Action items", and "Fill a template" open the AI on the transcript.
- **Quote timestamps:** for a transcript, each verified quote is matched to the segment where it starts, and the answer shows "Said at 0:04 in the recording".
- **File types:** `files.rs` now detects audio and video by content. They get a 4 GB limit instead of 100 MB, because they're streamed. Before transcription, the only action offered is "Transcribe to text". The assistant and templates appear once there's a transcript.
- Cancel works mid-run (abort callback plus checks between packets). Switching files cancels too.

### Problems found and fixed
- **Build:** whisper-rs's bundled bindings are generated on Linux and failed to compile on Windows (`_IO_FILE` size asserts). bindgen needs libclang, so I installed the `libclang` pip package (user-level, no admin). The README documents `LIBCLANG_PATH` and CMake.
- **whisper-rs bug:** every run failed with error −6 ("failed to encode"). `set_abort_callback_safe` boxes the closure as `Box<dyn FnMut>`, but its C trampoline casts the pointer back to the caller's closure type, so it read garbage and aborted. Passing an already-boxed closure makes the types match (comment in `run_chunk`).
- **Missing spaces:** whisper.cpp joined sentences inside a segment ("start.The midterm"). `fix_spacing` restores the space after `.!?` between a lowercase letter or digit and a capital letter, leaving "U.S.A." alone.
- **Dropped phrase:** greedy decoding lost "Hall 210. OK," on the WAV sample. Beam search (5) fixed it, at about 1.4× the time.
- **Tried and reverted:** turning off Whisper's text context. On an 11-minute recording of 20 identical loops it caught each sentence 15–19 times, against 17–20 with context, so context stays on.
- **The release build was 7× slower** (24.7 s for a 33 s sample, against 3.4 s in dev). I ruled out the new portable CPU flags (a native-tuned release build was just as slow) and the machine (the dev build was still fast at the same time). Diffing the generated MSBuild projects showed ggml compiled with no optimization in release. whisper-rs-sys names the CMake build type "Release" there, and the cmake crate then replaces `CMAKE_C_FLAGS_RELEASE` with the Rust flags minus every `/O` option. Its `force-debug` feature names "RelWithDebInfo" instead, which leaves the Release configuration's `/O2` alone. After enabling it, the portable release build takes **3.8 s**. Explained in a comment in `Cargo.toml`.

### A date bug the transcripts exposed (`ai.rs`)
- The local model gave the lecture's "Thursday, November 19" the year **2022**, and MyFile accepted it, because a yearless written date matched any year. It also gave "every Friday" the date 2022-11-29, and that wasn't flagged, because a quote with no date counted as "can't check".
- **Fix 1, `ground_year`:** the model's year counts only if the quote or the document states it. If nothing gives a year, code uses the next upcoming occurrence (dates up to 30 days ago count as this one) and adds a note. If the document has years but not this one, the date stays off the calendar.
- **Fix 2:** a dated item whose quote contains no date at all is now *unsupported*. An existing test encoded the old behavior (a date computed from "within 30 days" was "can't check"). I changed it, because computed dates were never meant to reach the calendar.
- **Verified in the app:** the same recording now gives Midterm **2026**-11-19 and Group project **2026**-12-01, with the note "The document doesn't say the year for 2 date(s)…". Lab reports is flagged and left off: "Add 2 dates to my calendar · 1 date was left out".

### Verified
- **Live test** (`live_transcribes_every_sample_format`, ignored, needs the model): WAV, MP3, M4A voice memo, and MP4 video through Symphonia, and WebM/Opus through ffmpeg. All 33 s samples were transcribed in 7–7.5 s and contain "midterm" and "November 19".
- **In the app:**
  - Dropped the M4A: the card says "M4A audio", and the only action is Transcribe to text.
  - The model download took 15 s, with progress and a passing checksum.
  - The MP4 video transcribed in 7.0 s. Search "midterm" filtered to the 0:04 line, and the assistant and template actions appeared.
  - "Find deadlines" on the local model: 3/3 quotes verified, each with "Said at …".
  - Reopening the transcript panel loads it instantly (cached on the selection).
- **Long recording:** 10:58 of audio became 1,360 words in 75 s (about 9× real time), with steady progress. Cancel at 1:10 stopped within about 1 s. The chunk cuts at about 5:00 and 10:00 landed between sentences.
- **CI:** the workflow now sets `GGML_NATIVE=OFF` (plus AVX2/FMA/F16C on Windows), so installers don't depend on the build machine's CPU, and points bindgen at the runner's LLVM. Not run yet (the repo isn't on GitHub).

**Tests:** 106 unit tests pass (4 ignored live tests). New tests cover resampling at 8–48 kHz, quiet-point chunking that loses no samples, subtitle formats, quote-to-timestamp lookup, non-speech markers, sentence spacing, year grounding, and dateless quotes.

## 2026-10-22: Milestone 7, search your files, and "show in document"

**Plan.** These are the next two features from the list: search by meaning (#2) and evidence-linked answers (#1). Search needs real backend work: an index, ranking, and retrieval for answers. The evidence viewer turns MyFile's quote checks into something you can click.

### Search your files (`src/search.rs`, `SearchPanel.tsx`)
- **Index:** SQLite (bundled `rusqlite`) in MyFile's data folder, holding roots, files, passages, and vectors. Passage text goes into an FTS5 table (Porter stemming, diacritics removed), kept in sync by triggers. Deleting a folder cascades to its files, passages, and FTS rows (tested). It covers only folders the person adds. A whole drive or the home folder is refused, and so are links, hidden folders, and folders like `node_modules`. Search goes at most 8 levels deep, up to 5,000 files per folder.
- **Incremental:** a file is reread only when its size or modified time changes. Deleted files leave the index. The first launch each session refreshes the index quietly if it's more than 10 minutes old.
- **Passages:** about 1,200 characters, split at paragraphs, then sentences, then spaces. Each keeps its page number.
- **Word search:** every term is quoted, so nothing typed is read as FTS syntax (tested with `OR`, `NEAR(`, quotes, and `*`). It tries all words first, then any word. The last word also matches as a prefix.
- **Meaning search (optional):** "Turn on meaning search" pulls `nomic-embed-text` through Ollama (progress shown), then stores a unit vector for each passage. A query's vector is compared with every passage (dot product), and the result is merged with the word ranking by reciprocal rank fusion.
- **Snippets:** built by code around the densest run of query words, with matches marked. The frontend renders them as text with `<mark>`, never as HTML.
- **Ask:** `prepare_ask` picks the top passages (up to 14 and 24,000 characters) and stores exactly those. `ask_files` answers from them with the folder-mode prompt and `validate_ask`, so every quote is checked and moved to the file that really contains it. Pages keep their real numbers, because pages without a chosen passage stay empty. With Claude, the person first sees which files, pages, and character count would be sent, and nothing goes until they click "Send to Claude".

### Problems found and fixed
- **Filler words:** when a query's words weren't all in one passage, the any-word fallback matched "to", "of", and "out". "notice to vacate" returned 13 files. Common words are now dropped unless the query is nothing but common words.
- **Meaning search returned everything:** every passage is somewhat similar, so every query listed 20 files, and "money owed to the government" ranked a grant guide first. I measured the scores. Medians ranged from 0.40 to 0.55 by query, so a fixed cutoff can't work. Real matches beat the median by 0.11–0.49, while the best wrong ones stayed under 0.10, including all passages for an unrelated query ("quantum chromodynamics"). A meaning match must now beat the median by 0.11. After the change: "recipe for banana bread" finds only the recipe, "when is the cell biology test" finds only the BIO syllabus, and "money owed to the government" honestly finds nothing.
- **Layout:** the folder list reused the watch-list styles and squeezed the names. It has its own layout now. Ask also had no way to choose Claude when a local model was installed, so it now has the same Local/Claude switch as the other panels.

### Show in document (`src/evidence.rs`, `EvidenceViewer.tsx`)
- `find_span` finds a quote in the page text with the same normalization as the quote check (case, whitespace runs, curly quotes, dashes) and maps it back to the original text. The viewer shows the page with that span marked and scrolled into view, with previous and next page.
- It's offered next to verified quotes in AI answers (including transcripts), folder answers, the semester timeline, fact-checks, search results, and "ask your files".
- For a search passage, it marks the sentence with the most search words, not the whole passage.
- **Access is limited** to the open file, documents in the open folder (looked up by name in memory), or paths in the search index. Tested: `../secret.txt` in a folder and `C:/Windows/win.ini` were both refused.

### Verified in the app
- Indexed `test-files` (9 added, 1 failed: the scanned PDF, which needs Tesseract) in 0.8 s, and `eval/docs` (11 files) in 1.1 s. Re-indexing: 21 unchanged.
- Word searches take 5–9 ms. With meaning search, 73–90 ms. Turning on meaning search, including the model download and 37 passages, took 19 s.
- "moving out of my apartment" puts lease.pdf first, matched by both words and meaning.
- Ask on the local model ("When do I have to tell my landlord…"): 3 items, each quote verified on lease.pdf p. 1.
- Ask with Claude ("Which of my classes don't accept late work?"): the consent box listed 9 files and 4,105 characters. After "Send to Claude", Sonnet answered in 8.8 s across BIO, MATH, and CS, with every quote verified.
- Clicking a result opens the file in MyFile. "Show on page" for "cumulative final exam" marks exactly that sentence on BIO-200 p. 2. "Show in document" on a local deadline answer marks the quoted sentence on lease.pdf.

**Tests:** 115 unit tests pass (4 ignored live tests). New tests cover chunking, FTS query safety, stop words, snippets, rank fusion, the full index lifecycle (add, search, stemming, any-word fallback, pages, unchanged, edited, deleted, folder removal), vector ranking, gathering passages with real page numbers, span finding with multi-byte text, page choice, and best-sentence selection.

## 2026-10-23: Milestone 8, MyFile becomes a file converter

**Direction from the user.** "This project is a file converter. It can convert to PDF, JPG, etc., any kind of file." They pasted a broad catalog of formats and said the interface was "chaotic": remove the unnecessary things, and depending on the file, offer conversion options. They also reported that the loading spinner was "a long spinning thing" instead of a circle.

**Plan.** A conversion engine where each (format → target) pair has exactly one engine: built in (Rust), ffmpeg, or LibreOffice/pandoc when installed. The interface offers only conversions whose engine is present. The main screen becomes: drop a file, see what it can become, convert, save. The earlier tools move behind one collapsed "More tools for this file". Nothing was deleted (the project has no commits yet, so a copy of `src/` was saved first). The AI's job is now a converter job: turn "small enough to email" into one of this file's conversions plus settings, checked by code.

### Engine (`src/convert/`)
- **`mod.rs`:** the format catalog (75 source formats plus aliases), detection by content (`infer`) with the extension as fallback (e.g. a .docx that sniffs as zip stays docx), routes per format, tool discovery, and the commands. Results live in a temp folder that's deleted when replaced or when the file changes. Saving is atomic (temp sibling, then rename). The original can't be overwritten, and multi-file results go into a chosen folder with "name (2)" instead of overwriting.
- **`media.rs`:** images through the `image` crate (now with TIFF, ICO, TGA, PNM and QOI), or decoded by ffmpeg (HEIC, AVIF, PSD, EXR...). Resize and quality settings apply. Lossy WebP and AVIF are encoded by ffmpeg. Audio and video run through ffmpeg with `-progress` (live percentage), cancel, and ffprobe stream checks: no "just the audio" from a silent video, and no GIF from an audio file. Also handles SRT ↔ VTT and subtitles → text.
- **`docs.rs`:** PDF → PNG/JPG per page with hayro (pure-Rust rendering at 150 dpi, or fit to a size). Markdown, text and HTML go to blocks, then to PDF, HTML, Markdown or text. The text of DOCX, ODT, ODP, PPTX and EPUB is read from their XML (roxmltree). EPUB follows the OPF spine order.
- **`pdfwrite.rs`:** a small PDF writer. It puts images on pages (JPEG, DCTDecode) and lays text out on Letter pages with the standard Helvetica and Courier fonts (word wrap from the AFM widths, page flow, headings, lists, code).
- **`data.rs`:** JSON / YAML / TOML / CSV / TSV / XML conversions and Excel/ODS reading (calamine) and writing (rust_xlsxwriter). CSV cells get natural JSON types, but "02139" keeps its leading zero. Tables render to HTML (escaped), Markdown (pipes escaped), and PDF.
- **`archive.rs`:** ZIP/TAR/TAR.GZ extract and repack. Only plain relative paths are written (`../evil.txt` is skipped), links are skipped, and size and entry count are capped.
- **`office.rs`:** LibreOffice headless with a private profile, and pandoc. Both run with argument lists, a 5-minute limit, and cancel.
- **AI:** `suggest_conversion` sends only the request, the file type and size, and this file's available conversions. `check_suggestion` accepts only an available target from that list, keeps only settings that apply to it, and snaps them to the interface's values.

### Interface
- Home is just the drop zone, "Drop any file to convert it", with one line of supported families. The folder watch, monitored list, search panel and folder button are gone from the main screen.
- File screen: format chips grouped by kind, a "describe it" box, settings that apply to the chosen target (quality, size, bitrate), "Convert to X", progress with Cancel, then a result card (preview, size, "% smaller") and Save. "More tools for this file" holds everything else, collapsed.
- **Spinner:** it was an inline `<span>`, so its width and height were ignored in text, and it stretched in flex rows. It's now an inline-block with a fixed 18px box and `aspect-ratio: 1`.

### Problems found by testing
- JSON → YAML reordered keys alphabetically (serde_json's default map is sorted). A converter must keep key order, so `preserve_order` is now on.
- PDF pages rendered 799 px when 800 was asked for: hayro truncates the scaled size, so the scale now aims half a pixel over.
- A same-format conversion (a smaller JPG, a re-encoded MP4) would have defaulted to saving under the original's name. It now gets "-converted", and MKV → MKV re-encodes instead of copying the streams.
- The PDF writer test compared byte offsets on a lossy UTF-8 string (the binary header shifted them). It now checks raw bytes.

### Verified
- **In the app:**
  - PNG → JPG at 800 px: 44 KB, in 1.0 s.
  - The 2-page sample PDF → PNG: 2 page images, with 50% and 100% progress shown.
  - MP4 → GIF in 2.6 s, with progress.
  - WAV → MP3: 86% smaller, in 0.5 s.
  - Markdown → PDF in 0.25 s.
  - "make it small enough to email" on the photo: the local model (16 s) picked JPG, quality 75, up to 800 px, and the result was 36% smaller.
  - Save: the Windows dialog saved the JPG (verified on disk, then removed).
- The PDF and DOCX screens list "More formats need LibreOffice (Word)" instead of offering them.
- **Not verified:** the LibreOffice and pandoc routes (neither is installed here). The save dialog opens in the last-used folder, not always the original's folder.

**Tests:** 137 unit tests pass. The 22 new converter tests cover:
- detection, and that every route matches its engine's availability;
- 8 image formats with resize, and image → PDF;
- PDF page rendering, checking that real text pixels appear;
- Markdown → HTML and PDF (read back with the PDF text extractor);
- CSV → JSON, and CSV → XLSX → CSV round-trips;
- JSON ↔ YAML ↔ TOML;
- SRT ↔ VTT ↔ text;
- ZIP extract with a traversal entry, and ZIP → TAR.GZ → extract;
- real ffmpeg runs (WAV → MP3 at 96 kbit/s, MP4 → GIF, MP4 → M4A, and audio → GIF refused);
- the AI-suggestion checks.

## 2026-10-25: Wrap-up, release build, and requirements check

**Release build.** Built with the portable CPU settings (`GGML_NATIVE=OFF`, AVX2/FMA/F16C):
- `MyFile_0.1.0_x64_en-US.msi` (13.9 MB)
- `MyFile_0.1.0_x64-setup.exe` (9.9 MB)

**Checked against the assignment:**

| Requirement | Where it's met |
|---|---|
| The model does real work on each run | "Describe it" turns a request into one of the file's conversions; under More tools, the assistant, templates, fact-checking, and search answers all call a real model. Code checks every answer. |
| A log of plans and changes in the repo | This file. |
| What the AI does and what the code does | README: "What it converts" (the AI step) and "What the AI does vs. what the code does". |
| No paid API key | The Claude Code CLI with the existing login, or the free local model (Ollama). |

**Still open:**
- LibreOffice and pandoc routes aren't installed here, so they're untested.
- The macOS build runs in CI once the repo is on GitHub, and hasn't run yet.
- The portfolio card and demo video describe the earlier version and need updating.
- Tesseract (OCR) wasn't installed, so the OCR step itself is untested.
