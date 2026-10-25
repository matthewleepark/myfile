// Records a captioned demo of the MyFile converter by driving the real app over the
// WebView2 DevTools protocol and capturing its screencast frames.
//
// 1. Start MyFile with debugging enabled, e.g. (PowerShell):
//      $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9223"
//      .\src-tauri\target\release\myfile.exe
// 2. node demo/record.mjs          -> demo/frames/ + demo/frames.txt
// 3. node demo/render.mjs          -> demo/myfile-demo.mp4 (needs ffmpeg)
//
// Captions and the closing card are injected into the page only while
// recording; the app itself is not modified. Waits on the AI are played back
// faster and labelled on screen.
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "..");
const framesDir = join(here, "frames");
rmSync(framesDir, { recursive: true, force: true });
mkdirSync(framesDir, { recursive: true });
const PORT = process.env.MYFILE_DEBUG_PORT ?? "9223";
const docs = (p) => join(root, p).replaceAll("\\", "/");

// --- DevTools connection -------------------------------------------------
const targets = await (await fetch(`http://127.0.0.1:${PORT}/json`)).json();
const page = targets.find((t) => t.type === "page" && /tauri\.localhost|localhost:1420/.test(t.url));
if (!page) throw new Error("MyFile isn't running with remote debugging on port " + PORT);
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((r) => ws.addEventListener("open", r, { once: true }));
let nextId = 1;
const pending = new Map();
const frames = [];
let speed = 1;
ws.addEventListener("message", (m) => {
  const msg = JSON.parse(m.data);
  if (msg.id && pending.has(msg.id)) {
    pending.get(msg.id)(msg);
    pending.delete(msg.id);
  } else if (msg.method === "Page.screencastFrame") {
    const { data, metadata, sessionId } = msg.params;
    const file = `f${String(frames.length).padStart(5, "0")}.jpg`;
    writeFileSync(join(framesDir, file), Buffer.from(data, "base64"));
    frames.push({ file, t: metadata.timestamp, speed });
    send("Page.screencastFrameAck", { sessionId });
  }
});
const send = (method, params = {}) =>
  new Promise((res, rej) => {
    const id = nextId++;
    pending.set(id, (msg) => (msg.error ? rej(new Error(JSON.stringify(msg.error))) : res(msg.result)));
    ws.send(JSON.stringify({ id, method, params }));
  });
const evaluate = async (expr) => {
  const r = await send("Runtime.evaluate", { expression: expr, awaitPromise: true, returnByValue: true });
  if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description ?? r.exceptionDetails.text);
  return r.result.value;
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const waitFor = async (expr, ms = 120000) => {
  const start = Date.now();
  while (Date.now() - start < ms) {
    if (await evaluate(expr)) return;
    await sleep(150);
  }
  throw new Error("Timed out waiting for " + expr);
};

// --- Helpers ---------------------------------------------------------------
async function caption(text, ffwd = false) {
  speed = ffwd ? 4 : 1;
  await evaluate(`(() => {
    let el = document.getElementById("demo-caption");
    if (!el) {
      el = document.createElement("div");
      el.id = "demo-caption";
      el.style.cssText = "position:fixed;left:50%;bottom:22px;transform:translateX(-50%);z-index:9999;max-width:86%;" +
        "padding:12px 20px;border-radius:12px;background:rgba(10,14,24,.92);color:#fff;font:600 17px/1.45 'Segoe UI',sans-serif;" +
        "text-align:center;box-shadow:0 10px 30px rgba(0,0,0,.45);pointer-events:none;transition:opacity .2s";
      document.body.appendChild(el);
    }
    el.innerHTML = ${JSON.stringify(text)} + (${ffwd} ? ' <span style="color:#9bb4ff;font-weight:700">&nbsp;⏩ 4×</span>' : "");
  })()`);
}
async function drop(rel) {
  await evaluate(`window.__TAURI_INTERNALS__.invoke("plugin:event|emit_to", { target: { kind: "Webview", label: "main" },
    event: "tauri://drag-drop", payload: { paths: [${JSON.stringify(docs(rel))}], position: { x: 500, y: 300 } } })`);
}
async function click(selectorOrText) {
  const ok = await evaluate(`(() => {
    const q = ${JSON.stringify(selectorOrText)};
    let el = q.startsWith("css:") ? document.querySelector(q.slice(4)) : null;
    if (!el) el = [...document.querySelectorAll("button")].find(b => b.textContent.trim().includes(q));
    if (!el) return false; el.scrollIntoView({block: "center", behavior: "smooth"}); el.click(); return true; })()`);
  if (!ok) throw new Error("No element for " + selectorOrText);
}
async function typeInto(selector, text) {
  for (let i = 1; i <= text.length; i++) {
    await evaluate(`(() => { const i = document.querySelector(${JSON.stringify(selector)});
      const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set;
      set.call(i, ${JSON.stringify(text)}.slice(0, ${i})); i.dispatchEvent(new Event("input", {bubbles: true})); })()`);
    await sleep(45);
  }
}
async function scrollTo(selector, block = "start") {
  await evaluate(`document.querySelector(${JSON.stringify(selector)})?.scrollIntoView({behavior: "smooth", block: ${JSON.stringify(block)}})`);
  await sleep(1200);
}
async function slowScroll(px, ms) {
  const steps = Math.round(ms / 40);
  for (let i = 0; i < steps; i++) {
    await evaluate(`window.scrollBy(0, ${px / steps})`);
    await sleep(40);
  }
}
// Clicks "Send to Claude" if the consent dialog is still open (a person at
// the keyboard may already have answered it).
async function approveConsent() {
  if (await evaluate(`!!document.querySelector("dialog")?.open`)) await click("Send to Claude");
}
const fileIs = (name) => `document.querySelector(".file-name")?.innerText === ${JSON.stringify(name)}`;

// --- Scenario --------------------------------------------------------------
await evaluate(`document.getElementById("demo-caption")?.remove(); document.getElementById("demo-card")?.remove(); window.scrollTo(0, 0);`);
const text = (sel) => evaluate(`document.querySelector(${JSON.stringify(sel)})?.innerText?.trim() ?? ""`);
const chip = (label) =>
  evaluate(`[...document.querySelectorAll(".format-chip")].find(b => b.querySelector(".format-chip-label").textContent === ${JSON.stringify(label)}).click()`);
async function convertNow() {
  await click("css:.convert-settings .btn-primary");
  await waitFor(`!!document.querySelector(".convert-result, .convert-panel .error-panel")`, 180000);
  await scrollTo(".convert-result, .convert-panel .error-panel", "center");
}
async function openFile(rel, name) {
  await evaluate(`window.scrollTo({top: 0, behavior: "smooth"})`);
  await drop(rel);
  await waitFor(`${fileIs(name)} && !!document.querySelector(".format-chip")`);
  await sleep(500);
}
await send("Page.startScreencast", { format: "jpeg", quality: 85, maxWidth: 1600, maxHeight: 1200, everyNthFrame: 1 });
await sleep(500);

await caption("MyFile is a desktop file converter. Drop in any file and it shows what that file can become.");
await sleep(4000);

await openFile("test-files/whiteboard-photo.png", "whiteboard-photo.png");
await caption("A PNG image. Every format it can turn into is listed, grouped by kind.");
await sleep(3500);
await typeInto(".describe input", "make it small enough to email");
await sleep(400);
await click("Suggest");
await caption("Or just describe what you need. The AI picks one conversion from this file's list. It sees my words and the file type, never the file.", true);
await waitFor(`!!document.querySelector(".describe p, .describe .error-panel")`, 180000);
await caption(`It picked ${await text(".format-chip.is-selected")}, smaller and resized. My code only accepts a choice that's really on the list and keeps settings in range.`);
await sleep(5000);
await convertNow();
await caption(`Converted on this computer: ${await text(".convert-result .success-line")}. The original is never changed.`);
await sleep(4500);

await openFile("test-files/sample-syllabus.pdf", "sample-syllabus.pdf");
await chip("PNG");
await caption("A PDF becomes one image per page, rendered right here in Rust.");
await sleep(1500);
await convertNow();
await caption(`Done: ${await text(".convert-result .success-line")}. Several files save into a folder you pick.`);
await sleep(4000);

await openFile("test-files/audio/bio-200-lecture-recording.mp4", "bio-200-lecture-recording.mp4");
await caption("A lecture video: other video formats, an animated GIF, a still frame, or just the audio, through ffmpeg.");
await sleep(3500);
await chip("MP3");
await sleep(600);
await convertNow();
await caption(`The sound, pulled out as an MP3: ${await text(".convert-result .success-line")}.`);
await sleep(4000);

await openFile("test-files/demo-grades.csv", "demo-grades.csv");
await caption("Data and spreadsheets too: CSV, JSON, YAML, TOML and Excel, keeping key order and leading zeros.");
await sleep(2500);
await chip("Excel");
await sleep(500);
await convertNow();
await sleep(3000);

// Closing card.
await evaluate(`document.getElementById("demo-caption")?.remove()`);
speed = 1;
const card = `<div style="max-width:780px;padding:40px">
  <div style="font-size:15px;letter-spacing:.14em;text-transform:uppercase;color:#9bb4ff;font-weight:700">MyFile · TCSS 490 Artifact 3</div>
  <h1 style="font-size:40px;margin:10px 0 18px">Convert any file. Describe it if you like.</h1>
  <ul style="font-size:20px;line-height:1.8;margin:0;padding-left:22px">
    <li><b>75+</b> input formats: images, PDFs, documents, data, audio, video, subtitles, archives</li>
    <li>The AI picks the conversion; <b>code checks</b> every choice before anything runs</li>
    <li>Runs on a <b>free local model</b> or Claude, never a paid API key</li>
    <li><b>137</b> automated tests · Windows and macOS · Rust + React</li>
  </ul>
  <p style="margin-top:26px;font-size:18px;color:#a3abc2">Try the browser version: <b style="color:#fff">matthewleepark.github.io/myfile/app</b></p>
</div>`;
await evaluate(`(() => {
  const c = document.createElement("div");
  c.id = "demo-card";
  c.style.cssText = "position:fixed;inset:0;z-index:10000;display:grid;place-items:center;background:#0b0d14;color:#eef0f6;font-family:'Segoe UI',sans-serif";
  c.innerHTML = ${JSON.stringify(card)};
  document.body.appendChild(c);
})()`);
await sleep(6500);
const stoppedAt = Date.now() / 1000;
await send("Page.stopScreencast");
await evaluate(`["demo-card", "demo-caption", "demo-style"].forEach(id => document.getElementById(id)?.remove())`);
ws.close();

// Concat list with real timing; fast-forward frames play at their speed.
const lines = [];
for (let i = 0; i < frames.length; i++) {
  const next = frames[i + 1];
  // Frames only arrive when the screen changes, so the last frame lasts
  // until recording stopped (e.g. the static closing card).
  const real = next ? next.t - frames[i].t : Math.max(0.5, stoppedAt - frames[i].t);
  const dur = Math.max(0.02, real / frames[i].speed);
  lines.push(`file 'frames/${frames[i].file}'`, `duration ${dur.toFixed(3)}`);
}
lines.push(`file 'frames/${frames[frames.length - 1].file}'`);
writeFileSync(join(here, "frames.txt"), lines.join("\n") + "\n");
const total = frames.reduce((s, f, i) => s + (frames[i + 1] ? (frames[i + 1].t - f.t) / f.speed : Math.max(0.5, stoppedAt - f.t)), 0);
console.log(`Captured ${frames.length} frames, about ${total.toFixed(0)} s of video.`);
