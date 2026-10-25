# Build Log, Artifact 3

Tool: Claude Code in VS Code (mostly Opus 5.5, sometimes Haiku). Dates: 2026-10-21 to 2026-10-25

- I pasted the Artifact 3 assignment and my plan for MyFile into Claude Code and asked it to build the app.
- Claude checked my computer first. Rust and the C++ build tools were missing, so I approved installing them.
- We used Claude Code in headless mode as the AI, so I didn't need a paid API key.
- Claude built a desktop app with Tauri (Rust backend, React frontend): drop a file in and see what you can do with it.
- I asked for features that were more eye-opening and kept asking for more, so Claude added a lot over several rounds.
- Near the end I realized the app had become mostly about summarizing documents. I told Claude it's supposed to be a file converter, pasted a list of file formats, and asked it to remove the unnecessary things.

## Changes I made

- First version: an AI document assistant that summarizes and finds deadlines. My code checks every quote the AI gives against the real document and asks permission before anything is sent.
- Added a test with trap documents and a hidden prompt injection. It showed Haiku did about as well as Sonnet for about a tenth of the cost.
- Made a free local AI model (Ollama) the default so files stay on my computer. Claude is optional.
- Added folder mode for a semester of syllabi, OCR, comparing two versions of a file, fact-checking, fill-in templates, a duplicate finder, speech to text, and search across my files.
- Changed direction: MyFile is now a file converter. Drop a file and it shows what it can become: images, PDF pages, Word/Excel/PowerPoint text, data files (CSV, JSON, YAML), audio, video, subtitles, and ZIP files.
- Added a "describe it" box. I can type "small enough to email" and the AI picks the conversion and settings, but the code only accepts a choice from the list for that file.
- Cleaned up the main screen and moved the older tools into "More tools for this file."
- Fixed the loading spinner, which was a stretched bar instead of a circle.

## Issues

- Asking for "everything" made the app too big and messy, so I had to step back and focus it on converting files. Next time I would decide the main purpose first.
- Testing caught real bugs that Claude then fixed: the AI put a date in the wrong year (2022 instead of 2026), the installed version ran 7x slower because of a build setting, and JSON to YAML mixed up the order of the keys.
- Some tools aren't installed on my computer, so those parts aren't tested yet: OCR needs Tesseract (the install got cancelled), and Word/Excel/PowerPoint to PDF needs LibreOffice.
- The Mac version only builds after I push the code to GitHub, so it isn't verified yet.
