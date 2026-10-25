// Makes test-files/scanned-syllabus.pdf: a 2-page PDF whose pages are only
// pictures of text (no text layer), like a scanner produces. Also writes
// test-files/whiteboard-photo.png, a photo-style image of text.
// Usage: node test-files/make-scanned.mjs   (needs Edge or Chrome)
import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const browser = [
  "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe",
  "C:/Program Files/Microsoft/Edge/Application/msedge.exe",
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  "/usr/bin/google-chrome",
].find(existsSync);
if (!browser) throw new Error("Needs Edge or Chrome.");
const work = mkdtempSync(join(tmpdir(), "myfile-scan-"));

function shot(name, html, w, h) {
  const file = join(work, name + ".html");
  writeFileSync(file, `<!doctype html><html><body style="margin:0;background:#fbfaf6">${html}</body></html>`);
  const png = join(work, name + ".png");
  execFileSync(browser, ["--headless=new", "--disable-gpu", "--hide-scrollbars", `--window-size=${w},${h}`, `--screenshot=${png}`, pathToFileURL(file).href], { stdio: "ignore" });
  return png;
}

const page = (body) =>
  `<div style="width:1240px;height:1754px;padding:110px;box-sizing:border-box;font:30px/1.6 Georgia,serif;color:#1d1d1d;transform:rotate(0.4deg)">${body}</div>`;

const p1 = shot("p1", page(`<h1 style="font-size:52px">PHYS 110: Physics of Everyday Life</h1>
<p>Instructor: Dr. Okafor. Office hours Thursdays 1:00 to 2:30 PM.</p>
<h2 style="font-size:40px">Lab schedule</h2>
<p>Lab report 1 is due October 30, 2026.</p>
<p>Lab report 2 is due November 13, 2026.</p>
<p>Late lab reports lose 20% per day.</p>`), 1240, 1754);
const p2 = shot("p2", page(`<h2 style="font-size:40px">Exams</h2>
<p>The midterm exam is on November 4, 2026.</p>
<p>The final exam is on December 14, 2026 at 10:30 AM.</p>
<p>Bring a calculator and one page of notes.</p>`), 1240, 1754);

// A PDF made only of the two pictures: no selectable text at all.
const b64 = (p) => readFileSync(p).toString("base64");
const html = join(work, "scan.html");
writeFileSync(html, `<!doctype html><html><head><style>@page{size:A4;margin:0}body{margin:0}img{display:block;width:210mm;height:297mm}</style></head><body>
<img src="data:image/png;base64,${b64(p1)}"><img src="data:image/png;base64,${b64(p2)}"></body></html>`);
execFileSync(browser, ["--headless=new", "--disable-gpu", "--no-pdf-header-footer", `--print-to-pdf=${resolve(join(here, "scanned-syllabus.pdf"))}`, pathToFileURL(html).href], { stdio: "ignore" });

// A whiteboard-style photo of a to-do list.
const wb = shot("wb", `<div style="width:1400px;height:900px;padding:80px;box-sizing:border-box;background:#f4f6f3;font:44px/1.7 'Segoe UI',sans-serif;color:#1f3b8f">
<p style="font-size:60px;font-weight:700;margin:0 0 20px">Group project: next steps</p>
<p style="margin:0">Maya: finish the survey by November 9, 2026</p>
<p style="margin:0">Leo: draft the slides by November 12, 2026</p>
<p style="margin:0">Everyone: rehearse on November 16, 2026</p></div>`, 1400, 900);
writeFileSync(join(here, "whiteboard-photo.png"), readFileSync(wb));
rmSync(work, { recursive: true, force: true });
console.log("wrote scanned-syllabus.pdf and whiteboard-photo.png");
