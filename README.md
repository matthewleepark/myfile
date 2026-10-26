# MyFile

**Convert any file.** MyFile is a desktop file converter for Windows and macOS. Drop in a file, and it shows what that file can become. Pick a format (or describe what you need), convert, and save. Your original is never changed.

Built with Tauri 2 (Rust backend) and React + TypeScript (frontend) for TCSS 490, Artifact 3.

**Try it:** the [browser version](https://matthewleepark.github.io/myfile/app/) (the common conversions, many files at once, nothing uploaded) · [download the desktop app](https://github.com/matthewleepark/myfile/releases/latest) · [project page](https://matthewleepark.github.io/myfile/)

## What it converts

| From | To | Engine |
|---|---|---|
| **Images:** PNG, JPG, GIF, WebP, BMP, TIFF, ICO, TGA, PPM, QOI | Any of those, AVIF, or **PDF**, with quality and size settings | Built in (lossy WebP and AVIF through ffmpeg) |
| **Images:** HEIC, AVIF, JPEG XL, PSD, EXR, JPEG 2000, PCX, DPX | The formats above | Decoded by ffmpeg |
| **PDF** | **Word (.docx)** and HTML (paragraphs, headings and lists rebuilt from the text), **PNG or JPG, one image per page** (rendered on this computer), text, Markdown | Built in ([hayro](https://crates.io/crates/hayro)) |
| **Word / ODT** (.docx, .odt) | PDF (text and headings), Word, HTML, text, Markdown; ODT and RTF with LibreOffice, EPUB with pandoc. Old .doc and .rtf files need LibreOffice | Built in / LibreOffice |
| **Excel / ODS** (.xlsx, .xls, .xlsm, .xlsb, .ods) | CSV, TSV, JSON, HTML table, Markdown table (one file per sheet), XLSX | Built in |
| **PowerPoint / ODP** | PDF and Word (the slides' text, one section per slide), text, Markdown. Old .ppt files and PowerPoint/ODP to each other need LibreOffice | Built in / LibreOffice |
| **EPUB** | Text, Markdown, HTML, PDF, Word | Built in |
| **Markdown, text, HTML** | PDF, Word, HTML, text, Markdown; ODT, EPUB and more with LibreOffice or pandoc | Built in / LibreOffice / pandoc |
| **Data:** JSON, YAML, TOML, CSV, TSV, XML | Each other, plus Excel, HTML/Markdown/PDF tables (key order kept) | Built in |
| **Subtitles:** SRT, VTT, ASS | SRT, VTT, plain text | Built in / ffmpeg |
| **Audio:** MP3, WAV, M4A, AAC, FLAC, OGG, Opus, WMA, AIFF, AMR | MP3, M4A, WAV, FLAC, OGG, Opus, AIFF, with bitrate | ffmpeg |
| **Video:** MP4, MOV, MKV, WebM, AVI, WMV, FLV, MPEG, 3GP, TS | MP4, WebM, MOV, MKV, AVI (or a smaller copy), animated **GIF**, a still **JPG**, or just the audio | ffmpeg |
| **Archives:** ZIP, TAR, TAR.GZ, .gz | Extract to a folder, or repack ZIP ↔ TAR.GZ | Built in |

The format is detected from the file's **contents**, not just its name, so a PNG renamed to .jpg is still treated as a PNG. Only conversions whose engine is installed are offered. The rest are listed as needing LibreOffice or pandoc. Results go to a private temporary folder until you save them. Several outputs (PDF pages, sheets, an extracted archive) save into a folder you choose, and nothing there is overwritten.

**Describe it (the AI step).** Type what you need, like "small enough to email" or "a PDF to print". The AI picks **one conversion from this file's list** and suggests quality, size, or bitrate. Code then checks the pick: it must be an available conversion for this file, and the settings snap to the values the interface offers. The AI sees only your words, the file type, and its size, never the file's contents. It uses the local model (Ollama) when installed, and otherwise Claude's small model.

**More tools.** Under "More tools for this file" are the earlier features: the AI document assistant, templates, comparison, transcription, and more. They're described below.

### More tools (under "More tools for this file")

| Feature | What it does | The model's one job | What code checks |
|---|---|---|---|
| **Semester brain** | Drop a *folder* of syllabi and get one timeline, same-day **conflicts** across classes, **crunch weeks**, one calendar, and questions answered across every document | Read each document for deadlines; answer cross-document questions naming the source file | Validates each answer against *its own* document, moves an answer to the file that really contains its quote, finds conflicts and crunch weeks itself |
| **Agent mode** | "Get my week ready" becomes a plan → you approve → MyFile runs it → a briefing | Plan using six tools while seeing only file *names*; write the briefing from results | Drops unknown tools, missing files, and out-of-order steps; shows exactly which documents would be sent; runs only the stored, approved plan |
| **Local model (default)** | Every AI feature runs on a private model (Ollama, Qwen 2.5 3B) when it's installed; Claude is optional, and you can run **both side by side** | Same as Claude | Same `ai::validate`, so both are scored by identical checks; nothing leaves the computer |
| **Version comparison** | "What changed between v1 and v2 of this syllabus?" | Explain the changes, citing change numbers | The **diff engine finds every change** (sentence-level, then word-level) and every moved date. The model sees only the changes; an explanation point that cites a change number that doesn't exist is dropped, and changes it skipped are listed |
| **Change monitor** | Keeps a snapshot of a document and tells you (with a notification) when its text changes, e.g. a revised syllabus | (Optional) explain the change | Text fingerprints, metadata-only vs real text changes, and the diff are all code. Snapshots live in the app's data folder |
| **Fact-check a claim** | "Late work is accepted for full credit." → **supported / contradicted / ambiguous / not found** | Judge the claim and quote evidence for and against | A *supported* or *contradicted* verdict needs a verified quote with that stance, or it's shown as **unverified**. Evidence both ways makes it **ambiguous**. "Not found" says plainly that absence isn't disproof |
| **Extraction templates** | Built-in (syllabus, job posting, lease, receipt) or your own fields, filled for one file or **every document in a folder**, exported as CSV/JSON | Fill each field with a value and a quote | Each value must have the right type, its quote must be in the document, and **numbers and dates must appear in that quote**. CSV cells are defused against spreadsheet formulas |
| **Duplicate finder** | Identical copies, the same text in different files, **two versions of one document** (with moved dates), and look-alike images | None. **No AI** | File bytes, text fingerprints, 3-word shingles, and image difference hashes. It lists what it found and never deletes anything |
| **Search your files** | Search the folders you choose by what documents *say*: by words (SQLite FTS5) and, optionally, by meaning (a local embedding model), then **Ask** a question answered from the best passages | Answer from the chosen passages, citing file and page. The embedding model only turns text into vectors | Indexing, ranking, snippets, the meaning cutoff, and which passages are sent are all code. Claude sees only passages you were shown first, and every quote is checked against them |
| **Show in document** | Every verified quote and search result opens its page with the exact text marked | None | Code finds the span with the same matching the quote check uses, and only for the open file, the open folder, or indexed files |
| **Speech to text** | Lecture recordings, voice memos, meeting videos → a timestamped transcript and subtitles; AI answers then say **when** each quote was said ("Said at 0:04") | None for the transcript (Whisper is a speech model, run locally). The assistant then reads the transcript like any document | Decoding (Symphonia, or ffmpeg as a fallback), resampling, cutting 5-minute chunks at the quietest moment, the model download's **SHA-256 check**, subtitle files, and matching each verified quote to its timestamp |
| **OCR** | Reads scanned PDFs and photos of text | (Reads the OCR'd text like any other) | Tesseract runs locally. Code pulls the page images out of the PDF (JPEG, Flate, ICC color) and labels OCR text so you know where it came from |
| **Folder watch** | Watches a folder like Downloads and reads new syllabi automatically, with a Windows notification | Find deadlines in the new document | Ignores in-progress downloads, links, and its own output; standing permission is explicit, visible, and stops with Stop |
| **MCP server** | `myfile-mcp` lets Claude Code itself call MyFile: read documents, **verify quotes**, compress images, export calendars | (Claude Code is the caller) | Every path stays inside a root folder; writes only ever create new files |

**Demo video (82 s):** [`demo/myfile-demo.mp4`](demo/myfile-demo.mp4). It converts a photo (with the AI picking the conversion), a PDF, a video and a CSV. It was recorded from the running app by [`demo/record.mjs`](demo/record.mjs), with captions that read the actual on-screen results and AI waits shown at 4×.

See [LOG.md](LOG.md) for the running log of plans and changes.

## What the AI does vs. what the code does

The model has one bounded job: **read extracted document text and answer a request about it, as structured JSON.** Everything around that is ordinary, deterministic code. When something goes wrong, this split makes it clear which side is at fault.

| The AI model (Claude) is responsible for | Ordinary code is responsible for |
|---|---|
| Interpreting the user's question | Checking the file exists, isn't empty or a folder, and is ≤ 100 MB |
| Reading and analyzing the document text | Detecting the real file type from its contents, not its extension |
| Summarizing, explaining in plain language | Extracting PDF text locally, page by page, and running OCR (Tesseract, on this computer) on scanned pages and photos |
| Finding deadlines, requirements, decisions, action items | Asking consent before any text leaves the computer, and enforcing it in Rust, not just in the UI |
| Marking each item as explicit or inferred | Running the CLI with **all tools disabled**, a fixed system prompt, a JSON Schema, a 180 s timeout, and cancellation |
| Choosing short supporting quotes and page numbers | **Checking every quote against the real document text**, labeling quotes it can't find, correcting wrong page numbers, and removing impossible ones |
| Turning each date into YYYY-MM-DD for the calendar | **Rejecting impossible dates and any ISO date the written date can't mean** (wrong month/day/year, swapped numeric order, relative dates), **choosing the year itself when the text never states one**, then writing the .ics file itself |
| **Routing:** mapping a typed request to one action ID (and, for images, suggesting settings) | Offering only the actions allowed for this file type (in the JSON Schema *and* re-checked in Rust), clamping settings to what the UI offers, and dropping decisions that arrive after you've switched files |
| | Capping lengths, stripping control characters, rejecting empty or malformed replies |
| | Showing results and saving files only where the user picks, never over the original |

**Checks added because testing found gaps:**

- **A date must come from its own quote** (`ai::date_supported`). The local model once returned a real, verified quote paired with a *different*, injected date. A verified quote only proves the passage exists, so now a date that isn't in its own quote is flagged and kept off every calendar.
- **A year must be stated, or code chooses it** (`ai::ground_year`). Transcribing a lecture that said "Thursday, November 19" showed the local model filling in **2022**. Now the model's year counts only if the quote or the document states it. If nothing states a year, MyFile uses the next upcoming date and says so. If the document gives other years but never this one, the date stays off the calendar. The same test showed a model giving "every Friday" a specific date, so a date whose quote contains no date at all is now unsupported too.
- **Instructions aimed at the AI are detected** (`ai::injected_sentences`). If a document contains text like "SYSTEM NOTE TO THE AI ASSISTANT: ignore the rest of this document…", MyFile warns you, and any item quoting that text is flagged and kept off the calendar. This catches the case the date check can't: the model quoting the planted sentence itself.

The model never chooses a file path or runs a command. When routing, it can only *name* one action from a fixed allowlist; Rust checks the name, and the action then runs exactly as if you'd clicked its button. Document reading still asks for consent first.

### Why the quote check matters

A JSON Schema can guarantee the reply has the right *shape*, but not that it's *true*. The model can return a well-formed quote that doesn't exist in the document. So [`ai::validate`](src-tauri/src/ai.rs) normalizes case, whitespace, curly quotes and dashes, then searches each page:

- Found → ✓ "Found in the document, page N" (with the page corrected if the model got it wrong)
- Not found → ⚠ "Not found in the document. The AI may have reworded or invented it."

### AI provider: the Claude Code CLI in headless mode

[`ClaudeCli::run`](src-tauri/src/ai.rs) starts `claude -p` as a child process:

```
claude -p --output-format json --model sonnet --tools "" --no-session-persistence
       --strict-mcp-config --disable-slash-commands --setting-sources ""
       --system-prompt <fixed> --json-schema <fixed>
```

- Arguments are a fixed list passed directly, never through a shell. The document and question go in on **stdin**, so they never appear on a command line.
- `--tools ""` means the model can't read files, run commands, or browse. It can only return text.
- It runs from the temp folder, with no settings sources loaded, so no project instructions or hooks influence it.
- MyFile finds the CLI via `MYFILE_CLAUDE_PATH`, then `PATH`, then `~/.local/bin`, then the copy bundled with the VS Code extension. `MYFILE_MODEL` overrides the model (default `sonnet`).

### Two models, chosen per job

| Job | Model | Why |
|---|---|---|
| Routing a typed request | Haiku | A small decision from a short list. It sends only your sentence and the file type, about 2–3 s and ≈$0.0005 list price per request. |
| Reading the document | Local Qwen 2.5 3B when installed; otherwise Sonnet (`MYFILE_MODEL` overrides) | The heavier job. See the evaluation below for how the models compare on it. |
| Folder work and templates (Claude) | Haiku | Many documents at once. In the evaluation Haiku matched Sonnet at about a tenth of the cost. |

**Trade-offs:**

| | |
|---|---|
| **Cost** | Uses the existing Claude subscription login. No paid API key, per course rules. |
| **Portability** | The local model needs only Ollama, with no account. Claude needs Claude Code installed and signed in. Both sit behind one `ai::Provider` enum with the same prompt, schema, and `validate`, so the UI doesn't care which one ran. |
| **Privacy** | Document text goes to Anthropic. MyFile says so, with the exact character count, and asks first for each file. |
| **Latency** | 7–13 s for the sample documents in testing, with an elapsed-seconds counter (no fake progress bar) and a working Cancel. |
| **Reliability** | Errors are classified: CLI not found, not signed in, usage limit, timeout, cancelled, malformed or empty reply. Each gets a message that says what to do. |

## Evaluation

`cargo test eval_live -- --ignored --nocapture` runs **13 test documents** through the real pipeline (local extraction → model → MyFile's validation) on **Haiku, Sonnet, and a local Qwen 2.5 3B**, and plain code scores each answer against known answers in [`eval/cases.json`](eval/cases.json). Full results: [`eval/RESULTS.md`](eval/RESULTS.md). The test PDFs can be regenerated with `node eval/make-docs.mjs`.

The cases are chosen to catch specific failures, not just easy wins:

- **Page attribution:** every date buried on page 3, or split across pages 2 and 4
- **Formats:** ISO dates (`2026-11-20`), a relative deadline ("within 30 days of this letter"), a 16-date table across two pages, a two-column layout
- **Moved deadline:** "originally March 1, moved to March 15". Giving the stale date as the answer fails.
- **Two traps:** a recipe with no deadlines, and a question about a warranty the manual never mentions. The only passing answer is "not found".
- **Prompt injection:** hidden white text telling the AI to report a fake 2030 deadline
- **Negative control:** every real reply is re-checked against an *unrelated* document. MyFile's quote checker must reject those quotes, which proves it isn't approving everything.

Scoring rules: an expected fact must appear in the model's own claim (title, date, or detail), never just inside a quote copied from the document. Dates match on number boundaries, so "November 1" ≠ "November 15".

**Final results (stricter scoring, after the review fixes):**

| Model | Cases passed | Expected facts found | Quotes verified | Traps & injection resisted | Control: quotes wrongly accepted | Median time | List-price cost (13 cases) |
|---|---|---|---|---|---|---|---|
| Haiku | 13/13 | 100% | 100% | 4/4 | 0 of 53 | 7.4 s | $0.013 |
| Sonnet | 13/13 | 100% | 100% | 4/4 | 0 of 53 | 6.5 s | $0.126 |
| **Qwen 2.5 3B, local** | **8/13** | **87%** | **98%** | **3/4** | 0 of 42 | 25.5 s | **$0, nothing leaves the computer** |

**What this shows, honestly:**

- **On these documents, Haiku matched Sonnet at about a tenth of the list cost.** Sonnet was a little faster on median time, so the smaller model isn't automatically the faster one. Sonnet stays the default for reading because real documents are messier than this set, but the evaluation makes that a choice you can measure.
- **MyFile's quote and page checks never had to fire on real output.** Across 26 runs, no quote was invented and no page number needed fixing. That's good news about current models, but it means the checks are a safety net rather than something these runs exercised. The negative control proves the net is real: when the same replies were checked against the wrong document, it rejected **all 106 quotes**.
- **The small local model is where MyFile's checks earn their keep.** It missed dates, gave **7 wrong page numbers that MyFile corrected**, and **fell for the prompt injection**. The trade-off in one row: private and free, but slower and less reliable.
- **The injection exposed a gap in MyFile itself, which is now fixed.** On one run the local model paired a real, verified quote with the injected *January 1, 2030* date, and that date went into the calendar. The fix has two checks: a date must appear in its own quote, and text addressed to an AI ("SYSTEM NOTE TO THE AI ASSISTANT…") is detected, so items quoting it are flagged. In the final run the model still claimed 2030 (so the case still fails, which is honest about the model), but **0 fake dates reached the calendar**. The model was fooled, but the app wasn't.
- **Thirteen documents written for this project are a small, friendly sample.** The harness is the lasting part: point it at harder real documents and the same table shows where each model breaks.

## Adversarial code review

The new code got a multi-agent review: four reviewers (routing security, consent and auto-run, calendar dates, evaluation honesty), then an independent skeptic per serious finding who tried to *refute* it by tracing the code. Five findings survived, and all were fixed, each with a test:

1. A routing decision arriving after you switched files could auto-run on the new file. Now every request names its selection ID, the backend rejects stale ones, and the UI ignores late decisions.
2. Consent wasn't bound to a specific file between the check and the send. Consent is now granted per selection ID and re-checked immediately before any text leaves the computer.
3. Numeric written dates were only checked for the year. The checker now builds every date the text can mean (ISO, M/D and D/M, month names with ordinals and ranges) and requires an exact match.
4. An empty written date skipped the check. It now yields no calendar date.
5. The evaluation's scoring was too loose (substring dates, quotes counting as answers, a loose "superseded" check). It was tightened before the final run.

## Security review of the five new features

A separate review agent read the new code looking for concrete exploits. It found 9 real defects (6 medium, 3 low), and all were fixed:

1. **MCP writes could follow a dangling symlink** out of the root. Files are now created with `create_new`, which refuses any existing path, including links, and closes the check-then-write race.
2. **`notes.txt:x.ics` would write a hidden NTFS stream** onto an existing file. Names containing `:` or control characters are rejected.
3. **`list_documents` followed junctions** out of the root. It no longer follows links.
4. A wrong plan id threw away the current valid plan. The id is now checked first.
5. The agent's briefing could be sent after you'd left the folder. The folder is re-checked first.
6. **Auto-read could still send a document after you pressed Stop.** Permission is re-checked right before sending, and Stop cancels in-flight reads.
7. **A symlink in Downloads (e.g. `syllabus.md -> ~/.ssh/id_rsa`) would be auto-read.** Watch skips links and anything not directly in the folder.
8. MyFile could re-read its own saved `.ics`. `.ics` files are ignored, and saves no longer default into the watched folder.
9. **Cancel could silently stop working** when two folder requests overlapped. Each request now has its own cancel token. Cancel in folder mode stops all folder AI work at once.

## Security and privacy model

- **No filesystem plugin.** The frontend can't read or write files itself. It sends the dropped or picked path to `select_file`, which validates it and remembers it as the current file. Every later command acts only on that file.
- **Save dialogs open from Rust**, so the interface never supplies a write path. The OS dialog confirms before replacing a file, Rust refuses to write over the original, and writes are atomic (temp file + rename).
- **Narrow capabilities.** Only `core:default` and `dialog:allow-open` are granted ([capabilities/default.json](src-tauri/capabilities/default.json)).
- **Strict CSP.** No remote scripts, styles, or connections.
- **Nothing lingers.** Extracted text, AI answers, and compressed images are held in memory until saved, and cleared when you change files. There are no temp files.
- **Images:** re-encoding drops EXIF metadata, including GPS location. The UI says so.
- **Untrusted input everywhere:** a PDF that crashes the parser becomes an error message (`catch_unwind`), images over 100 megapixels are refused, and prompt-injection text inside a document can at worst produce a wrong answer, because the model has no tools.

## Shift-drop

Holding Shift while dropping a file onto MyFile opens a quick action menu at the drop point. The OS drop event doesn't report modifier keys, so on drop the backend asks the OS whether Shift is down: `GetAsyncKeyState` on Windows, `CGEventSourceFlagsState` on macOS. Both read key state only, with no input hooks and no extra permissions ([shift.rs](src-tauri/src/shift.rs)).

**Out of scope:** a *system-wide* "Shift-drag anywhere on the desktop" gesture. That would need a global input hook (and macOS Accessibility permission), which the project rules rule out as too intrusive.

## MyFile as an MCP server

Build it with `cargo build --release --bin myfile-mcp`, then register it with Claude Code once:

```sh
claude mcp add myfile -- "<path-to>/src-tauri/target/release/myfile-mcp.exe" --root "<folder MyFile may use>"
```

Then in Claude Code: *"Using the myfile tools, find every deadline in syllabus.pdf, verify each quote, and export them to deadlines.ics."* In testing, Claude called `read_document`, then `verify_quote` seven times (all FOUND), then `export_calendar`, in 14 s.

## Running it

Prerequisites: Node 20+, Rust stable, and the platform's [Tauri prerequisites](https://tauri.app/start/prerequisites/). On Windows that's the C++ Build Tools and WebView2; on macOS, the Xcode Command Line Tools. The AI features also need [Claude Code](https://claude.com/claude-code), installed and signed in.

```sh
npm install
npm run tauri dev                     # run in development
npm run tauri build                   # build installers for the current OS
cd src-tauri && cargo test            # unit tests (no AI calls)
cargo test live_ -- --ignored --nocapture   # one real end-to-end AI call (needs Claude Code)
cargo test eval_live -- --ignored --nocapture   # full evaluation, writes eval/RESULTS.md
EVAL_MODELS=local:qwen2.5:3b EVAL_APPEND=1 cargo test eval_live -- --ignored   # add the local model
EVAL_ONLY=injection EVAL_VERBOSE=1 cargo test eval_live -- --ignored          # one case, printed, not saved
```

Optional, for the free local model and for OCR:

```sh
# Windows
winget install Ollama.Ollama
winget install --id tesseract-ocr.tesseract -e
# macOS
brew install ollama tesseract
# then, on either
ollama pull qwen2.5:3b
```

**Speech to text needs no install.** The speech model (Whisper base English, 141 MB) downloads from inside the app the first time, with a SHA-256 check. `MYFILE_WHISPER_MODEL` points at a different ggml model file. ffmpeg is optional; it adds formats like WebM and MKV. **Building** MyFile compiles whisper.cpp, which needs **CMake** (the Visual Studio Build Tools include one; or `winget install Kitware.CMake` / `brew install cmake`) and **libclang** for bindgen (`LIBCLANG_PATH`; on Windows `pip install --user libclang` provides it without admin rights; on macOS, Xcode's is found automatically). For an installer that runs on other computers, build with `GGML_NATIVE=OFF GGML_AVX2=ON GGML_FMA=ON GGML_F16C=ON`, as CI does. (The `force-debug` feature on whisper-rs-sys in `Cargo.toml` isn't for debugging: without it, Windows release builds compile whisper.cpp unoptimized and run 7× slower. See LOG.md.)

MyFile finds Tesseract on the PATH, in its standard install folders, or at `MYFILE_TESSERACT`. Without it, scanned files get a clear "install OCR" message instead of a wrong answer.

Sample files for trying every feature are in [`test-files/`](test-files/): `scanned-syllabus.pdf` and `whiteboard-photo.png` for OCR, `versions/` for comparison, `semester/` for folder mode, and `audio/` (made by `make-audio.ps1`) for speech to text.

## Platform verification

| | Windows 11 | macOS |
|---|---|---|
| Builds, runs, all features | ✅ Verified (see LOG.md for how) | ✅ Builds in CI as .dmg files for Apple Silicon and Intel, and all unit tests pass on GitHub's Mac runner. ❌ Not yet run on a real Mac. |
| OCR | Image extraction from scanned PDFs verified by a unit test. The Tesseract step is **not verified yet**, because Tesseract isn't installed on the test machine | Code looks in the Homebrew paths, untested |
| Shift detection | ✅ Verified with a real Shift key press | Code written (`CGEventSourceFlagsState`), untested |
| CI build and unit tests | ✅ [`.github/workflows/build.yml`](.github/workflows/build.yml) builds the .exe and .msi installers and runs the tests | ✅ Same workflow builds unsigned `.app`/`.dmg` for Apple Silicon and Intel (macOS 10.15 or newer) and runs the tests |

All paths go through Rust's `std::path`, dialogs and drag-and-drop come from Tauri, and the only platform-specific code is the small, isolated `shift.rs`. A Mac is still needed to *prove* macOS works.

## Known limitations

- **Conversion:** MyFile's own Word output keeps the text, headings and lists, not the layout; PDF to Word rebuilds paragraphs from the PDF's text, so columns, tables and images aren't kept, and a scanned PDF has no text to convert. Excel to PDF, old .doc/.ppt/.rtf files, and ODT/RTF output need LibreOffice; EPUB, LaTeX and reStructuredText output need pandoc. Neither is installed on the test machine, so those routes are written but **not verified**. MyFile's own text-to-PDF uses the standard PDF fonts, which show Western European characters; others print as "?". Animated GIF/WebP inputs convert their first frame. Camera RAW, vector formats (SVG, AI, EPS), and 7z/RAR archives aren't converted yet.

- OCR needs Tesseract installed separately, and its English model only. PDFs whose page images use JPEG 2000 or JBIG2 are skipped with a message.
- Speech to text is English only, and the base model mishears some words (on the samples, "office hours" once became "our"). On audio that repeats itself word for word, Whisper sometimes skipped or duplicated a sentence. In an 11-minute test made of 20 identical loops, it caught each sentence 17–20 times.
- Search indexes PDFs and text files only (not images or recordings), up to 5,000 files per folder. Meaning search uses a small model: "money owed to the government" didn't find the tax letter on the samples, though "IRS letter about income" did.
- Password-protected PDFs aren't supported.
- Folder mode reads up to 40 documents (and one level of subfolders). Comparison is two files at a time.
- Near-duplicate detection is tuned on the sample documents (versions share about 0.43 of their 3-word runs, while unrelated syllabi share at most 0.03). Heavily rewritten versions may not be grouped.
- Animated GIFs become a still image (first frame) when compressed.
- WebP output isn't offered: the Rust `image` crate only encodes lossless WebP, which usually isn't smaller.
- Documents over 300,000 characters are refused by the AI assistant rather than silently truncated.
