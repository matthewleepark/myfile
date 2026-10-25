// Generates test-files/semester/: four classes whose deadlines deliberately
// collide, for trying MyFile's semester brain.
// Usage: node test-files/make-semester.mjs   (needs Edge or Chrome for PDFs)
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const out = join(dirname(fileURLToPath(import.meta.url)), "semester");
mkdirSync(out, { recursive: true });
const browser = [
  "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe",
  "C:/Program Files/Microsoft/Edge/Application/msedge.exe",
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  "/usr/bin/google-chrome",
].find(existsSync);
if (!browser) throw new Error("Needs Edge or Chrome to print PDFs.");

const PAGE_BREAK = `<div style="page-break-before:always"></div>`;

const PDFS = {
  "CS-101-syllabus": `
<h1>CS 101: Introduction to Programming (Fall 2026)</h1>
<p>Instructor: Prof. Rivera. Lectures Monday and Wednesday.</p>
<h2>Schedule</h2>
<p>Project 1 (text adventure) is due Friday, October 30, 2026.</p>
<p>The midterm exam is on Wednesday, November 4, 2026, in class.</p>
<p>Project 2 (data dashboard) is due Wednesday, November 18, 2026.</p>
<p>The final exam is on Wednesday, December 9, 2026 at 8:30 AM.</p>
<h2>Late work</h2>
<p>Projects lose 10% per day late, up to three days. After three days they are not accepted.</p>`,

  "BIO-200-syllabus": `
<h1>BIO 200: Cell Biology</h1>
<p>Lab sections meet Tuesday and Thursday afternoons.</p>
<h2>Lab reports</h2>
<p>Lab report 1 (microscopy) is due November 4, 2026.</p>
<p>Lab report 2 (enzyme kinetics) is due November 6, 2026.</p>
${PAGE_BREAK}
<h2>Exams</h2>
<p>Exam 2 will be held on November 20, 2026.</p>
<p>The cumulative final exam is on December 11, 2026.</p>
<h2>Late work</h2>
<p>Late lab reports are not accepted for any reason. Plan ahead.</p>`,

  "HIST-150-syllabus": `
<h1>HIST 150: The Modern World</h1>
<p>This course surveys global history from 1750 to the present through primary sources.</p>
<p>Weekly reading responses are due before each Monday discussion.</p>
<h2>Major assignments</h2>
<p>The source analysis essay is due November 5, 2026.</p>
<p>Group presentations take place on December 2, 2026.</p>
<h2>Late work</h2>
<p>Essays lose 5% per day late. Contact the instructor before the deadline if you need an extension.</p>`,
};

const MD = {
  "MATH-221-course-info.md": `# MATH 221: Linear Algebra, course info

Problem sets are due every Friday at 5 PM.

## Quizzes and exams

- Quiz 3 is on November 18, 2026.
- The final exam is on December 9, 2026.

## Late work

Problem sets may be turned in one class late for half credit. Quizzes cannot be made up.
`,
  "club-meeting-notes.md": `# Chess club notes

We played a blitz tournament and voted on new club shirts (navy won).
Snacks rotate alphabetically. Nobody has homework here.
`,
};

for (const [name, body] of Object.entries(PDFS)) {
  const html = join(out, `${name}.html`);
  writeFileSync(
    html,
    `<!doctype html><html><head><meta charset="utf-8"><style>body{font-family:Georgia,serif;font-size:13pt;line-height:1.5;margin:1in}</style></head><body>${body}</body></html>`,
  );
  execFileSync(browser, ["--headless=new", "--disable-gpu", "--no-pdf-header-footer", `--print-to-pdf=${resolve(join(out, name + ".pdf"))}`, pathToFileURL(html).href], { stdio: "ignore" });
  rmSync(html);
}
for (const [name, text] of Object.entries(MD)) writeFileSync(join(out, name), text);
console.log("wrote", Object.keys(PDFS).length + Object.keys(MD).length, "files to", out);
