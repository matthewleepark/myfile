// Makes test-files/versions/: two versions of a syllabus with realistic
// edits (a moved deadline, a rewritten late policy, an added requirement, a
// removed section, and a wording-only change), for trying comparison.
// Usage: node test-files/make-versions.mjs   (needs Edge or Chrome)
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const out = join(dirname(fileURLToPath(import.meta.url)), "versions");
mkdirSync(out, { recursive: true });
const browser = [
  "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe",
  "C:/Program Files/Microsoft/Edge/Application/msedge.exe",
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  "/usr/bin/google-chrome",
].find(existsSync);
if (!browser) throw new Error("Needs Edge or Chrome.");

const PB = `<div style="page-break-before:always"></div>`;
const v1 = `
<h1>ENGL 201: Writing About Science (Fall 2026)</h1>
<p>Instructor: Prof. Lindqvist. Class meets Tuesday and Thursday.</p>
<h2>Assignments</h2>
<p>The research proposal is due October 29, 2026.</p>
<p>The literature review is due November 12, 2026.</p>
<p>The final paper is due December 8, 2026.</p>
${PB}
<h2>Policies</h2>
<p>Late work loses 10% per day, with no limit.</p>
<p>Attendance is recorded but not graded.</p>
<h2>Extra credit</h2>
<p>You may attend two campus science talks for extra credit.</p>`;

const v2 = `
<h1>ENGL 201: Writing About Science (Fall 2026)</h1>
<p>Instructor: Prof. Lindqvist. The class meets on Tuesday and Thursday.</p>
<h2>Assignments</h2>
<p>The research proposal is due October 29, 2026.</p>
<p>The literature review is due November 17, 2026.</p>
<p>The final paper is due December 8, 2026.</p>
<p>A peer review of a classmate's draft is due December 1, 2026.</p>
${PB}
<h2>Policies</h2>
<p>Late work loses 10% per day, up to a maximum of 30%.</p>
<p>Attendance is recorded but not graded.</p>`;

for (const [name, body] of [["ENGL-201-syllabus-v1", v1], ["ENGL-201-syllabus-v2", v2]]) {
  const html = join(out, name + ".html");
  writeFileSync(html, `<!doctype html><html><head><meta charset="utf-8"><style>body{font-family:Georgia,serif;font-size:13pt;line-height:1.5;margin:1in}</style></head><body>${body}</body></html>`);
  execFileSync(browser, ["--headless=new", "--disable-gpu", "--no-pdf-header-footer", `--print-to-pdf=${resolve(join(out, name + ".pdf"))}`, pathToFileURL(html).href], { stdio: "ignore" });
  rmSync(html);
}
console.log("wrote", out);
