# Build Log, Artifact 3

Tool: Claude Code in VS Code (mostly Opus 5.5, sometimes Haiku)

## 2026-10-21, Environment check and AI provider decision

- I pasted the Artifact 3 assignment and my plan for MyFile into Claude Code and asked it to build the app.
- Claude checked my computer first. Rust and the C++ build tools were missing, so I let it install them.
- We went with Claude Code in headless mode for the AI, so I didn't need a paid API key.

## 2026-10-21, First version

- Claude built the app with Tauri (Rust backend, React frontend). You drop a file in and it shows what you can do with it.
- The main feature was an AI assistant that summarizes documents and finds deadlines.
- My code checks every quote the AI gives against the real document, and it asks me before sending anything to Claude.
- Claude made a test with trick documents and a hidden prompt injection. Haiku did about as well as Sonnet for way cheaper.

## 2026-10-22, Adding a lot more

- I wanted the project to be more eye-opening, so I kept asking for more features.
- I made a free local AI model (Ollama) the default so my files stay on my computer. Claude is still an option.
- Claude added folder mode for a whole semester of syllabi, OCR, comparing two versions of a file, fact-checking, templates, a duplicate finder, speech to text, and search across my files.

## 2026-10-23, Switching to a file converter

- I realized the app turned into mostly a summarizer, but I wanted a file converter. I told Claude that and pasted a big list of file formats.
- Now when you drop a file, it shows what it can be converted to: images, PDF pages, Word/Excel/PowerPoint text, CSV/JSON/YAML, audio, video, subtitles, and ZIP files.
- I added a "describe it" box. I can type something like "small enough to email" and the AI picks the conversion and settings. The code only lets it pick from the list for that file.
- I told Claude the screen was too chaotic, so it cleaned up the main page and moved the old tools into "More tools for this file."
- I also had it fix the loading spinner because it was a long stretched bar instead of a circle.

## 2026-10-25, Wrapping up

- Claude built the Windows installers and checked the project against the assignment requirements.
- I had it shorten this log so it matches my other logs.
- I pushed everything to GitHub. The Mac build failed at first because the speech-to-text part needs a newer macOS, so Claude set the minimum to macOS 10.15. Then it failed again while packaging, so it split the Mac build into separate Apple Silicon and Intel apps. Now Windows and both Mac builds pass.
- I felt like the project looked like no effort went into it, even though a lot was built. So I had Claude make it look better: a browser version of the converter on my portfolio (it converts many files at once and has a before/after slider), a new project page with real screenshots, and a new demo video of the converter.
- Testing the browser version caught that PDF and MP3 conversions stalled when the tab wasn't visible, so Claude fixed that.
- I published a release on GitHub so people can download the app for Windows or Mac.
- I wanted a PDF to give me options like Word, not just images. So Claude added a built-in Word (.docx) writer: a PDF now converts to Word, HTML, text, Markdown, or images (one per page). It rebuilds the paragraphs, headings and bullet points from the PDF's text. Markdown, text and PowerPoint can now become Word files too, and Word files can become PDFs without LibreOffice. I opened the results in Microsoft Word to check them, in the app and in the browser version.

## Issues

- Asking for "everything" made the app way too big and messy, so I had to step back and focus on converting files. Next time I'd decide the main purpose first.
- Testing caught some real bugs that Claude fixed: the AI put a date in the wrong year (2022 instead of 2026), the installed app ran 7x slower because of a build setting, and JSON to YAML mixed up the order of things.
- Some tools aren't installed on my computer, so those parts aren't tested yet. OCR needs Tesseract (the install got cancelled), and Word/Excel/PowerPoint to PDF needs LibreOffice.
- I don't have a Mac, so the Mac version builds and passes its tests on GitHub, but I haven't run it on a real Mac.
- The browser version can't do everything the desktop app can. The AI "describe it" box and video conversion need the desktop app.
