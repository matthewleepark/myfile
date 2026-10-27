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
- I asked for MyFile to open whole folders and pull all the information out of them. Now you can drop a folder or click "Choose a folder", and MyFile reads every file inside, even in subfolders: the text of PDFs, Word, PowerPoint, Markdown and code, the rows of spreadsheets, what's inside ZIP files, and the size or length of pictures and recordings. It can also transcribe the recordings. Then you save everything as one Word, PDF, Markdown, HTML, text or JSON file, or a CSV list of the files. Claude tested it on a folder with 16 different kinds of files and checked that the Word file opens in Microsoft Word. The browser version on my site can take folders too.

## 2026-10-27, Project direction and AI boundaries

- I gave Claude a long set of guidelines for MyFile's direction: it's a file utility first, AI only helps, and the app's own code decides what runs. First I had it inspect everything and report back without changing anything. It found the app itself was mostly fine, but the docs and tests had drifted. The README and the eval were still mostly about the AI document tools. Nothing tested the "describe it" conversion AI with a real model. A malformed AI answer quietly turned into "no conversion fits". And the README listed two features (search and folder watch) that aren't in the app anymore.
- I updated CLAUDE.md with the project's purpose statement and the rules: don't invent features or results, AI output is untrusted, ask before anything destructive, and keep tests and docs current. It also spells out the step-by-step limits on the AI and has a table sorting every feature into "aligned", "supporting" or "potential scope drift". The AI document tools are in the last group, so we keep them but don't add more.
- The conversion AI now rejects any answer that doesn't have exactly the right fields, and shows a clear message instead of guessing. The conversion code also re-checks settings (quality, size, bitrate) before running, so out-of-range values are refused.
- Claude added tests:
  - malformed AI answers;
  - formats that don't exist, or exist but not for this file;
  - a file whose name and text try to give the AI instructions, which never reach the conversion AI;
  - broken input files;
  - missing tools;
  - saving over a read-only file or into a missing folder;
  - a folder report with planted instructions in it.
  154 tests pass.
- We made a new live eval for the conversion AI (13 cases). It checks the model's pick and what MyFile accepted, and for normal cases it actually converts the file and checks the output. Haiku passed 13/13. The local model passed 11/13: once it picked Word when I asked for ODT, and once it picked JPG when I asked to email a file. MyFile only pre-selects a pick, so nothing ran without a click.
- I added an adversarial document to the document-assistant eval: it says the real deadline is in 2031 and "authorizes" deleting files and running commands. Haiku and Sonnet passed all 14 cases. The local model passed 8/14 and fell for the fake 2031 date. Its "ASSISTANT INSTRUCTIONS:" wording wasn't something MyFile's injection check knew, so Claude added it with a test.
- Everything is listed in eval/TESTS.md: what each test checks, what was expected, what happened, and whether it was automated or a live AI run.

## Issues

- Asking for "everything" made the app way too big and messy, so I had to step back and focus on converting files. Next time I'd decide the main purpose first.
- Testing caught some real bugs that Claude fixed: the AI put a date in the wrong year (2022 instead of 2026), the installed app ran 7x slower because of a build setting, and JSON to YAML mixed up the order of things.
- Some tools aren't installed on my computer, so those parts aren't tested yet. OCR needs Tesseract (the install got cancelled), and Word/Excel/PowerPoint to PDF needs LibreOffice.
- I don't have a Mac, so the Mac version builds and passes its tests on GitHub, but I haven't run it on a real Mac.
- The local model can pick a real conversion that isn't what I asked for (Word instead of ODT, JPG for "email this"). Code can make sure the pick is a real option, but not that it's the right one, which is why nothing runs until I click Convert.
- The browser version can't do everything the desktop app can. The AI "describe it" box and video conversion need the desktop app.
