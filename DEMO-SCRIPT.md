# MyFile demo video script (about 90 seconds)

**Record with:** Windows Snipping Tool → Record (Win + Shift + R), drag over the MyFile window, and talk while you click.
**Before recording:** open MyFile, keep `test-files/` and `eval/docs/` open in File Explorer next to it, and close anything private on screen.

| Time | Do this | Say this (roughly) |
|---|---|---|
| 0:00 | Show the empty MyFile window. | "This is MyFile, a desktop app where you drop in a file and get the actions that make sense for it. The AI part reads documents, and plain code checks everything it says." |
| 0:10 | Drag `sample-syllabus.pdf` in. Type **"what do I need to do this week?"** in the Ask bar and press Go. | "I can just type what I want. A small model, Haiku, picks one action from a fixed list. It only sees my sentence, not the file." |
| 0:20 | Point at the purple decision line, then the consent dialog. | "It chose 'find deadlines', and before any document text leaves my computer, MyFile asks permission and tells me exactly how many characters it will send." |
| 0:30 | Click **Send to Claude**. Wait for the answer. | "Now Sonnet reads the document through the Claude Code CLI, with every tool turned off, so it can only return text." |
| 0:42 | Point at the green **"8 of 8 quotes found word for word"** banner, then one quote's ✓ page tag. | "This is the important part. The model has to quote its evidence, and MyFile's own code searches the real text for every quote and fixes wrong page numbers. If it invented something, this would say 'not found'." |
| 0:55 | Point at the cost line at the bottom. | "Every answer shows the model, the time, the tokens, and the list-price cost. This one was about a cent and a half, and on my plan it isn't billed per call." |
| 1:02 | Drag `eval/docs/injection.pdf` in, choose **Ask the AI assistant → Deadlines & dates**, send. | "This newsletter has hidden text telling the AI to report a fake 2030 deadline. It ignores it and reports the real dates." |
| 1:15 | Open `eval/RESULTS.md` in VS Code's preview. | "I also built an evaluation: 13 test documents, including traps and that injection, run on Haiku and Sonnet, scored by plain code. The table shows the trade-off: Haiku matched Sonnet here at about a tenth of the cost." |
| 1:25 | Drag `sample-image.png` in, type **"make this small enough to email"**. | "And non-AI features too. Here the router picked settings and it compressed the image locally, about 90 percent smaller." |
| 1:30 | Stop recording. | |

**Upload:** YouTube (Unlisted) or Google Drive (anyone with link), then put the link on your portfolio card.
