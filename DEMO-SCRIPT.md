# MyFile demo script (about 80 seconds)

The recorded demo (`demo/myfile-demo.mp4`) follows this script. `demo/record.mjs` drives the running app and adds the captions; see the comments at the top of that file for how to run it.

1. **Home.** "MyFile is a desktop file converter. Drop in any file and it shows what that file can become."
2. **A photo** (`test-files/whiteboard-photo.png`). Every format it can become appears, grouped by kind.
3. **Describe it.** Type "make it small enough to email" and press Suggest. The AI picks one conversion from this file's list (JPG, quality 75, up to 800 px). It sees only the words and the file type, and code checks the pick before anything runs.
4. **Convert.** The JPG is about a third smaller, and the original is unchanged.
5. **A PDF** (`test-files/sample-syllabus.pdf`) → PNG: one image per page, rendered on the computer.
6. **A video** (`test-files/audio/bio-200-lecture-recording.mp4`) → MP3: just the audio, through ffmpeg.
7. **A CSV** (`test-files/demo-grades.csv`) → Excel.
8. **Closing card:** 74 formats, the AI picks and code checks, a free local model or Claude, 137 tests, and the browser version's address.
