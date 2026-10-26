//! Folder report: every file in a folder (and its subfolders), its details,
//! and everything MyFile can read from it, saved as one Word, PDF, Markdown,
//! HTML, text or JSON file, or a CSV list of the files.
//!
//! Reading uses the converter's own readers, on this computer; nothing is
//! sent anywhere. Links are never followed, hidden files are skipped, and the
//! files are only read.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use super::pdfwrite::{self, Block};
use super::{archive, data, docs, docx, media, Conversion, ConversionResult, Engine, Family, Job, OutFile, Settings, Tools};
use crate::error::{human_size, AppError};
use crate::{lock, AppState};

/// Files read from one folder; the rest are counted but left out.
pub const MAX_FILES: usize = 1000;
const MAX_DEPTH: usize = 12;
/// Text kept from one file, and from the whole folder.
const MAX_FILE_CHARS: usize = 200_000;
const MAX_TOTAL_CHARS: usize = 5_000_000;
const MAX_TABLE_ROWS: usize = 500;
const MAX_CELL_CHARS: usize = 30;
const MAX_ARCHIVE_LISTED: usize = 300;
/// Text files of unknown type are read up to this size.
const MAX_PLAIN_BYTES: u64 = 2 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Finding the files
// ---------------------------------------------------------------------------

/// Every regular file under `dir` as (path, name relative to `dir` with "/"),
/// sorted by name, and how many more there were past the limit.
pub fn walk(dir: &Path) -> (Vec<(PathBuf, String)>, usize) {
    let mut found = Vec::new();
    let mut over = 0;
    let mut stack = vec![(dir.to_path_buf(), 0usize)];
    while let Some((d, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name.eq_ignore_ascii_case("desktop.ini") || name.eq_ignore_ascii_case("thumbs.db") {
                continue;
            }
            // symlink_metadata: a link is neither a file nor a folder here.
            let Ok(meta) = std::fs::symlink_metadata(e.path()) else { continue };
            if meta.is_dir() {
                if depth < MAX_DEPTH {
                    stack.push((e.path(), depth + 1));
                }
            } else if meta.is_file() {
                if found.len() < MAX_FILES {
                    let rel = e.path().strip_prefix(dir).unwrap_or(&e.path()).to_string_lossy().replace('\\', "/");
                    found.push((e.path(), rel));
                } else {
                    over += 1;
                }
            }
        }
    }
    found.sort_by_key(|f| f.1.to_lowercase());
    (found, over)
}

// ---------------------------------------------------------------------------
// Reading one file
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct Entry {
    pub rel: String,
    pub label: String,
    pub family: Option<Family>,
    pub size: u64,
    pub modified: Option<SystemTime>,
    /// "3 pages", "1920 × 1080 pixels", "4:12 long", ...
    pub details: Vec<String>,
    /// What was read from the file.
    pub blocks: Vec<Block>,
    /// Why nothing (or not everything) was read.
    pub note: Option<String>,
    pub recording: bool,
    pub transcribed: bool,
}

impl Entry {
    fn words(&self) -> usize {
        self.blocks.iter().map(|b| block_text(b).split_whitespace().count()).sum()
    }
}

fn block_text(b: &Block) -> String {
    match b {
        Block::Heading(_, t) | Block::Para(t) | Block::Item(t) => t.clone(),
        Block::Code(lines) => lines.join("\n"),
        Block::Rule => String::new(),
    }
}

fn block_chars(b: &Block) -> usize {
    match b {
        Block::Heading(_, t) | Block::Para(t) | Block::Item(t) => t.chars().count(),
        Block::Code(lines) => lines.iter().map(|l| l.chars().count() + 1).sum(),
        Block::Rule => 0,
    }
}

struct Ctx<'a> {
    tools: &'a Tools,
    ocr: bool,
    /// The speech model, when recordings should be transcribed.
    model: Option<&'a Path>,
    cancel: &'a Arc<AtomicBool>,
}

fn read_entry(path: &Path, rel: &str, ctx: &Ctx) -> Entry {
    let meta = std::fs::metadata(path).ok();
    let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let format = super::detect(path);
    let family = format.and_then(super::family_of);
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let label = match format {
        Some(f) => super::label_of(f).to_string(),
        None if ext.is_empty() => "File".to_string(),
        None => format!("{} file", ext.to_uppercase()),
    };
    let mut e = Entry {
        rel: rel.to_string(),
        label,
        family,
        size,
        modified: meta.and_then(|m| m.modified().ok()),
        details: Vec::new(),
        blocks: Vec::new(),
        note: None,
        recording: matches!(family, Some(Family::Audio | Family::Video)),
        transcribed: false,
    };
    if size == 0 {
        e.note = Some("Empty file.".into());
        return e;
    }
    if size > crate::files::MAX_FILE_BYTES && !e.recording {
        e.note = Some(format!("Too large to read (over {}).", human_size(crate::files::MAX_FILE_BYTES)));
        return e;
    }
    let result = read_contents(path, format, family, ctx, &mut e);
    if format.is_none() && !e.blocks.is_empty() {
        e.label = if ext.is_empty() { "Text file".into() } else { format!("Text file (.{ext})") };
    }
    if let Err(err) = result {
        if err.code == "cancelled" {
            e.note = Some("Not read (cancelled).".into());
        } else {
            e.note = Some(err.message);
        }
        e.blocks.clear();
    }
    cap_blocks(&mut e.blocks, MAX_FILE_CHARS, "this file");
    e
}

fn read_contents(path: &Path, format: Option<&'static str>, family: Option<Family>, ctx: &Ctx, e: &mut Entry) -> Result<(), AppError> {
    let Some(format) = format else {
        // Unknown type: include it if it's text (source code, logs, configs).
        let mut head = Vec::new();
        std::fs::File::open(path).and_then(|f| f.take(8192).read_to_end(&mut head)).map_err(AppError::from_read)?;
        if !crate::files::looks_like_text(&head) {
            e.note = Some("MyFile can't read inside this kind of file.".into());
            return Ok(());
        }
        let mut bytes = Vec::new();
        std::fs::File::open(path).and_then(|f| f.take(MAX_PLAIN_BYTES).read_to_end(&mut bytes)).map_err(AppError::from_read)?;
        let text = String::from_utf8_lossy(&bytes);
        e.details.push(lines_detail(&text));
        e.blocks = vec![Block::Code(text.trim_end().lines().map(|l| l.replace('\t', "    ")).collect())];
        return Ok(());
    };
    match family.expect("known formats have a family") {
        Family::Pdf => match crate::pdf::extract(path) {
            Ok(t) => {
                e.details.push(plural(t.page_count, "page"));
                e.blocks = t.pages.iter().enumerate().flat_map(|(i, p)| docs::pdf_page_blocks(p, i == 0)).collect();
            }
            Err(err) if err.code == "no_text" && ctx.ocr => {
                let pages = crate::ocr::pdf_text(path)?;
                e.details.push(plural(pages.len(), "page"));
                e.details.push("scanned, read with OCR".into());
                e.blocks = pages.iter().flat_map(|p| docs::text_blocks(p)).collect();
            }
            Err(err) if err.code == "no_text" => {
                e.note = Some("Scanned pages with no text layer. Install Tesseract OCR to read them.".into());
            }
            Err(err) => return Err(err),
        },
        Family::Document | Family::Presentation | Family::Ebook if matches!(format, "docx" | "odt" | "odp" | "pptx" | "epub") => {
            e.blocks = docs::file_blocks(path, format)?;
            if format == "pptx" {
                let slides = e.blocks.iter().filter(|b| matches!(b, Block::Heading(2, t) if t.starts_with("Slide "))).count();
                e.details.push(plural(slides, "slide"));
            }
        }
        Family::Text => {
            e.blocks = match format {
                "md" | "txt" | "html" => docs::file_blocks(path, format)?,
                // reStructuredText, LaTeX, Org: their source is readable as is.
                _ => docs::text_blocks(&read_text(path)?),
            };
        }
        Family::Spreadsheet => {
            let sheets = data::read_workbook(path)?;
            e.details.push(plural(sheets.len(), "sheet"));
            for (name, t) in &sheets {
                e.blocks.push(Block::Heading(3, format!("Sheet: {name}")));
                e.blocks.extend(table_blocks(t));
            }
        }
        Family::Data => match format {
            "csv" | "tsv" => {
                let t = data::read_delimited(&read_text(path)?, if format == "csv" { b',' } else { b'\t' })?;
                e.details.push(plural(t.rows.len(), "row"));
                e.blocks = table_blocks(&t);
            }
            _ => {
                let text = read_text(path)?;
                e.details.push(lines_detail(&text));
                e.blocks = vec![Block::Code(text.trim_end().lines().map(|l| l.replace('\t', "    ")).collect())];
            }
        },
        Family::Subtitle => {
            let text = via_route(path, format, "txt", ctx)?;
            e.details.push(lines_detail(&text));
            e.blocks = docs::text_blocks(&text);
        }
        Family::Image => {
            let dims = image::image_dimensions(path).ok().map(|(w, h)| (w as u64, h as u64)).or_else(|| probe(ctx.tools, path).1);
            if let Some((w, h)) = dims {
                e.details.push(format!("{w} × {h} pixels"));
            }
            if ctx.ocr && media::builtin_decodes(format) {
                let text = crate::ocr::image_text(&crate::image_ops::load(path)?)?;
                if !text.trim().is_empty() {
                    e.details.push("text read with OCR".into());
                    e.blocks = docs::text_blocks(&text);
                }
            }
        }
        Family::Audio | Family::Video => {
            let (secs, dims) = probe(ctx.tools, path);
            if let Some(s) = secs.or_else(|| audio_seconds(path)) {
                e.details.push(format!("{} long", crate::transcribe::clock((s * 1000.0) as u64)));
            }
            if let Some((w, h)) = dims {
                e.details.push(format!("{w} × {h} video"));
            }
            if let Some(model) = ctx.model {
                let t = crate::transcribe::transcribe_file(model, path, ctx.cancel.clone(), Arc::new(|_, _| {}))?;
                let text = crate::transcribe::plain_text(&t.segments);
                e.transcribed = true;
                if text.trim().is_empty() {
                    e.note = Some("No speech was heard in this recording.".into());
                } else {
                    e.details.push("speech transcribed".into());
                    e.blocks = docs::text_blocks(&text);
                }
            }
        }
        Family::Archive => match format {
            "zip" | "tar" | "tgz" => {
                let names = archive_entries(path, format, ctx)?;
                e.details.push(format!("{} inside", plural(names.len(), "file")));
                e.blocks = names.iter().take(MAX_ARCHIVE_LISTED).map(|n| Block::Item(n.clone())).collect();
                if names.len() > MAX_ARCHIVE_LISTED {
                    e.blocks.push(Block::Para(format!("…and {} more.", names.len() - MAX_ARCHIVE_LISTED)));
                }
            }
            "gz" => {}
            _ => e.note = Some(format!("MyFile can't open {} archives yet.", format.to_uppercase())),
        },
        // Old .doc, .rtf and .ppt files: LibreOffice turns them into text.
        Family::Document | Family::Presentation | Family::Ebook => {
            let text = via_route(path, format, "txt", ctx)?;
            e.blocks = docs::text_blocks(&text);
        }
    }
    Ok(())
}

fn read_text(path: &Path) -> Result<String, AppError> {
    let bytes = std::fs::read(path).map_err(AppError::from_read)?;
    Ok(String::from_utf8_lossy(&bytes).trim_start_matches('\u{feff}').to_string())
}

fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

fn lines_detail(text: &str) -> String {
    plural(text.trim_end().lines().count(), "line")
}

/// Runs one of the converter's text routes (e.g. old Word to text through
/// LibreOffice) in a private temporary folder and returns the text.
fn via_route(path: &Path, format: &'static str, target: &str, ctx: &Ctx) -> Result<String, AppError> {
    let engine = super::routes(format).into_iter().find(|r| r.0 == target).map(|r| r.4);
    match engine {
        Some(e) if ctx.tools.has(e) => {}
        Some(e) => return Err(AppError::new("needs_tool", format!("Reading this file needs {}, which isn't installed.", e.tool_name()))),
        None => return Err(AppError::new("bad_target", "MyFile can't read the text of this kind of file.")),
    }
    let dir = tempfile::Builder::new()
        .prefix("myfile-report-")
        .tempdir()
        .map_err(|e| AppError::new("convert_failed", format!("Couldn't create a working folder: {e}")))?;
    let settings = Settings::default();
    let job = Job { input: path, format, target, settings: &settings, out_dir: dir.path(), stem: "text", tools: ctx.tools, cancel: ctx.cancel, progress: &|_| {} };
    let mut text = String::new();
    for f in super::run(&job)? {
        text.push_str(&read_text(&dir.path().join(f))?);
        text.push('\n');
    }
    Ok(text)
}

fn archive_entries(path: &Path, format: &'static str, ctx: &Ctx) -> Result<Vec<String>, AppError> {
    let settings = Settings::default();
    let out = std::env::temp_dir();
    let job = Job { input: path, format, target: "extract", settings: &settings, out_dir: &out, stem: "list", tools: ctx.tools, cancel: ctx.cancel, progress: &|_| {} };
    archive::entries(&job)
}

/// (length in seconds, video size) from ffprobe, when it's installed.
fn probe(tools: &Tools, path: &Path) -> (Option<f64>, Option<(u64, u64)>) {
    let Some(ffprobe) = &tools.ffprobe else { return (None, None) };
    let mut cmd = Command::new(ffprobe);
    cmd.args(["-v", "error", "-show_entries", "format=duration:stream=codec_type,width,height", "-of", "json"]).arg(path);
    media::no_window(&mut cmd);
    let Ok(out) = cmd.output() else { return (None, None) };
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else { return (None, None) };
    let secs = v["format"]["duration"].as_str().and_then(|s| s.parse::<f64>().ok()).filter(|s| s.is_finite() && *s > 0.0);
    let dims = v["streams"]
        .as_array()
        .and_then(|s| s.iter().find(|s| s["codec_type"] == "video"))
        .and_then(|s| Some((s["width"].as_u64()?, s["height"].as_u64()?)));
    (secs, dims)
}

/// An audio file's length from its header, without ffmpeg.
fn audio_seconds(path: &Path) -> Option<f64> {
    use symphonia::core::{formats::FormatOptions, io::MediaSourceStream, meta::MetadataOptions, probe::Hint};
    let mss = MediaSourceStream::new(Box::new(std::fs::File::open(path).ok()?), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe().format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default()).ok()?;
    let track = probed.format.tracks().iter().find(|t| t.codec_params.sample_rate.is_some() && t.codec_params.n_frames.is_some())?;
    Some(track.codec_params.n_frames? as f64 / track.codec_params.sample_rate? as f64)
}

/// A table as aligned monospace lines (it reads the same in Word, PDF and
/// Markdown), up to MAX_TABLE_ROWS rows.
fn table_blocks(t: &data::Table) -> Vec<Block> {
    let cell = |c: &str| -> String {
        let c = c.replace(['\n', '\r', '\t'], " ");
        if c.chars().count() > MAX_CELL_CHARS { c.chars().take(MAX_CELL_CHARS - 1).collect::<String>() + "…" } else { c }
    };
    let rows: Vec<Vec<String>> = std::iter::once(&t.headers).chain(t.rows.iter().take(MAX_TABLE_ROWS)).map(|r| r.iter().map(|c| cell(c)).collect()).collect();
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    if cols == 0 {
        return vec![];
    }
    let widths: Vec<usize> = (0..cols).map(|i| rows.iter().map(|r| r.get(i).map_or(0, |c| c.chars().count())).max().unwrap_or(0)).collect();
    let line = |r: &Vec<String>| -> String {
        (0..cols)
            .map(|i| {
                let c = r.get(i).map(String::as_str).unwrap_or("");
                format!("{c}{}", " ".repeat(widths[i] - c.chars().count()))
            })
            .collect::<Vec<_>>()
            .join(" | ")
            .trim_end()
            .to_string()
    };
    let mut lines = vec![line(&rows[0]), widths.iter().map(|w| "-".repeat(*w)).collect::<Vec<_>>().join("-+-")];
    lines.extend(rows[1..].iter().map(line));
    let mut out = vec![Block::Code(lines)];
    if t.rows.len() > MAX_TABLE_ROWS {
        out.push(Block::Para(format!("…and {} more rows.", t.rows.len() - MAX_TABLE_ROWS)));
    }
    out
}

/// Keeps at most `max` characters of blocks, ending with a note when cut.
fn cap_blocks(blocks: &mut Vec<Block>, max: usize, what: &str) -> bool {
    let mut total = 0;
    for (i, b) in blocks.iter().enumerate() {
        total += block_chars(b);
        if total > max {
            blocks.truncate(i);
            blocks.push(Block::Para(format!("(MyFile kept the first {} characters of {what}.)", group(max))));
            return true;
        }
    }
    false
}

fn group(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

// ---------------------------------------------------------------------------
// The whole folder
// ---------------------------------------------------------------------------

pub struct Report {
    pub name: String,
    pub entries: Vec<Entry>,
    pub created: SystemTime,
    pub notes: Vec<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ReportProgress {
    done: usize,
    total: usize,
    current: Option<String>,
}

/// Reads every file. Entries from `previous` are reused when the file hasn't
/// changed (and doesn't still need transcribing).
pub fn build(dir: &Path, name: &str, tools: &Tools, model: Option<&Path>, cancel: &Arc<AtomicBool>, previous: Option<&Report>, progress: &dyn Fn(usize, usize, &str)) -> Result<Report, AppError> {
    let (files, over_limit) = walk(dir);
    if files.is_empty() {
        return Err(AppError::new("empty_folder", "That folder has no files in it."));
    }
    let ocr = crate::ocr::available();
    let ctx = Ctx { tools, ocr, model, cancel };
    let old: HashMap<&str, &Entry> = previous.map(|r| r.entries.iter().map(|e| (e.rel.as_str(), e)).collect()).unwrap_or_default();
    let mut entries = Vec::with_capacity(files.len());
    let mut total_chars = 0;
    for (i, (path, rel)) in files.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err(AppError::new("cancelled", "Reading the folder was cancelled."));
        }
        progress(i, files.len(), rel);
        let meta = std::fs::metadata(path).ok();
        let unchanged = |e: &&&Entry| {
            meta.as_ref().is_some_and(|m| m.len() == e.size && m.modified().ok() == e.modified) && !(e.recording && model.is_some() && !e.transcribed)
        };
        let mut e = match old.get(rel.as_str()).filter(unchanged) {
            Some(e) => (*e).clone(),
            None => read_entry(path, rel, &ctx),
        };
        if cancel.load(Ordering::Relaxed) {
            return Err(AppError::new("cancelled", "Reading the folder was cancelled."));
        }
        let chars: usize = e.blocks.iter().map(block_chars).sum();
        if total_chars + chars > MAX_TOTAL_CHARS && !e.blocks.is_empty() {
            e.blocks.clear();
            e.note = Some(format!("Not included: the report reached its {}-character limit.", group(MAX_TOTAL_CHARS)));
        } else {
            total_chars += chars;
        }
        entries.push(e);
    }
    progress(files.len(), files.len(), "");

    let mut notes = Vec::new();
    if over_limit > 0 {
        notes.push(format!("This folder has {} more files past MyFile's {MAX_FILES}-file limit; they aren't included.", over_limit));
    }
    let images = entries.iter().filter(|e| e.family == Some(Family::Image)).count();
    if !ocr && images > 0 {
        notes.push(format!("{} not read for text: install Tesseract OCR to read text in pictures.", plural(images, "image")));
    }
    let untranscribed = entries.iter().filter(|e| e.recording && !e.transcribed).count();
    if untranscribed > 0 {
        notes.push(format!("{} not transcribed, so only their details are listed.", plural(untranscribed, "recording")));
    }
    Ok(Report { name: name.to_string(), entries, created: SystemTime::now(), notes })
}

/// "2026-10-25" in this computer's time zone.
fn date(t: SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(t).format("%Y-%m-%d").to_string()
}

fn meta_line(e: &Entry) -> String {
    let mut parts = vec![e.label.clone()];
    parts.extend(e.details.iter().cloned());
    parts.push(human_size(e.size));
    if let Some(m) = e.modified {
        parts.push(format!("modified {}", date(m)));
    }
    parts.join(" · ")
}

/// A file's headings go under its own heading in the report: its top level
/// becomes level 3.
fn demoted(blocks: &[Block]) -> Vec<Block> {
    let top = blocks.iter().filter_map(|b| if let Block::Heading(l, _) = b { Some(*l) } else { None }).min().unwrap_or(1);
    blocks
        .iter()
        .map(|b| match b {
            Block::Heading(l, t) => Block::Heading((l - top + 3).min(6), t.clone()),
            other => other.clone(),
        })
        .collect()
}

/// (overview, one part per file).
fn report_blocks(r: &Report) -> (Vec<Block>, Vec<Vec<Block>>) {
    let total: u64 = r.entries.iter().map(|e| e.size).sum();
    let read = r.entries.iter().filter(|e| !e.blocks.is_empty()).count();
    let mut overview = vec![
        Block::Heading(1, r.name.clone()),
        Block::Para(format!(
            "{} ({}). MyFile read the contents of {} of them on {}.",
            plural(r.entries.len(), "file"),
            human_size(total),
            read,
            date(r.created)
        )),
    ];
    overview.extend(r.notes.iter().map(|n| Block::Item(n.clone())));
    overview.push(Block::Heading(2, "Files".into()));
    overview.extend(r.entries.iter().map(|e| Block::Item(format!("{}: {}", e.rel, meta_line(e)))));
    let parts = r
        .entries
        .iter()
        .map(|e| {
            let mut b = vec![Block::Heading(2, e.rel.clone()), Block::Para(meta_line(e))];
            if let Some(n) = &e.note {
                b.push(Block::Para(n.clone()));
            }
            b.extend(demoted(&e.blocks));
            b
        })
        .collect();
    (overview, parts)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonFile<'a> {
    path: &'a str,
    #[serde(rename = "type")]
    kind: &'a str,
    size_bytes: u64,
    modified: Option<String>,
    details: &'a [String],
    words: usize,
    note: Option<&'a str>,
    text: String,
}

/// The report as `target`: docx, pdf, md, html, txt, json, or csv (the file list).
pub fn render(r: &Report, target: &str) -> Result<Vec<u8>, AppError> {
    let (overview, parts) = report_blocks(r);
    let all = || -> Vec<Block> {
        let mut v = overview.clone();
        for p in &parts {
            v.push(Block::Rule);
            v.extend(p.iter().cloned());
        }
        v
    };
    Ok(match target {
        "docx" => docx::write(&[overview.clone(), parts.concat()], &r.name)?,
        "pdf" => pdfwrite::from_blocks(&all(), &r.name).0,
        "md" => docs::blocks_to_markdown(&all()).into_bytes(),
        "html" => docs::html_page(&r.name, &docs::blocks_to_html(&all())).into_bytes(),
        "txt" => docs::blocks_to_text(&all()).into_bytes(),
        "json" => {
            let files: Vec<JsonFile> = r
                .entries
                .iter()
                .map(|e| JsonFile {
                    path: &e.rel,
                    kind: &e.label,
                    size_bytes: e.size,
                    modified: e.modified.map(date),
                    details: &e.details,
                    words: e.words(),
                    note: e.note.as_deref(),
                    text: if e.blocks.is_empty() { String::new() } else { docs::blocks_to_text(&e.blocks) },
                })
                .collect();
            let v = serde_json::json!({ "folder": r.name, "created": date(r.created), "notes": r.notes, "files": files });
            serde_json::to_vec_pretty(&v).map_err(|e| AppError::new("convert_failed", e.to_string()))?
        }
        "csv" => {
            let mut w = csv::Writer::from_writer(Vec::new());
            let fail = |e: csv::Error| AppError::new("convert_failed", e.to_string());
            w.write_record(["path", "type", "size_bytes", "modified", "details", "words", "note"]).map_err(fail)?;
            for e in &r.entries {
                w.write_record([
                    e.rel.as_str(),
                    &e.label,
                    &e.size.to_string(),
                    &e.modified.map(date).unwrap_or_default(),
                    &e.details.join("; "),
                    &e.words().to_string(),
                    e.note.as_deref().unwrap_or(""),
                ])
                .map_err(fail)?;
            }
            w.into_inner().map_err(|e| AppError::new("convert_failed", e.to_string()))?
        }
        _ => return Err(AppError::new("bad_target", "MyFile can't save a folder report in that format.")),
    })
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntrySummary {
    path: String,
    label: String,
    size_bytes: u64,
    details: Vec<String>,
    words: usize,
    note: Option<String>,
    recording: bool,
    transcribed: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportSummary {
    files: Vec<EntrySummary>,
    total_bytes: u64,
    total_words: usize,
    /// Files whose contents were read.
    read: usize,
    notes: Vec<String>,
    elapsed_ms: u64,
}

fn summary(r: &Report, elapsed_ms: u64) -> ReportSummary {
    ReportSummary {
        files: r
            .entries
            .iter()
            .map(|e| EntrySummary {
                path: e.rel.clone(),
                label: e.label.clone(),
                size_bytes: e.size,
                details: e.details.clone(),
                words: e.words(),
                note: e.note.clone(),
                recording: e.recording,
                transcribed: e.transcribed,
            })
            .collect(),
        total_bytes: r.entries.iter().map(|e| e.size).sum(),
        total_words: r.entries.iter().map(Entry::words).sum(),
        read: r.entries.iter().filter(|e| !e.blocks.is_empty()).count(),
        notes: r.notes.clone(),
        elapsed_ms,
    }
}

/// Reads every file in the open folder. With `transcribe`, recordings are
/// also turned into text (the speech model must be downloaded).
#[tauri::command]
pub async fn read_folder_report(selection_id: u64, transcribe: bool, app: AppHandle, state: State<'_, AppState>) -> Result<ReportSummary, AppError> {
    let (name, dir) = crate::folder::folder_path(&state, selection_id)?;
    let model = if transcribe {
        let m = crate::transcribe::model_path(&app)?;
        if !m.is_file() {
            return Err(AppError::new("no_model", "Download the speech model first: open any recording and choose “Transcribe to text”."));
        }
        Some(m)
    } else {
        None
    };
    let previous = lock(&state.folder_report).as_ref().filter(|(id, _)| *id == selection_id).map(|(_, r)| r.clone());
    let cancel = Arc::new(AtomicBool::new(false));
    if let Some(old) = lock(&state.convert_cancel).replace(cancel.clone()) {
        old.store(true, Ordering::Relaxed);
    }
    let started = Instant::now();
    let a = app.clone();
    let c = cancel.clone();
    let result = crate::blocking(move || {
        let tools = Tools::find();
        let progress = |done: usize, total: usize, current: &str| {
            let _ = a.emit("report-progress", ReportProgress { done, total, current: (!current.is_empty()).then(|| current.to_string()) });
        };
        build(&dir, &name, &tools, model.as_deref(), &c, previous.as_deref(), &progress)
    })
    .await;
    {
        let mut slot = lock(&state.convert_cancel);
        if slot.as_ref().is_some_and(|s| Arc::ptr_eq(s, &cancel)) {
            *slot = None;
        }
    }
    let report = result?;
    crate::folder::folder_path(&state, selection_id)?;
    let out = summary(&report, started.elapsed().as_millis() as u64);
    *lock(&state.folder_report) = Some((selection_id, Arc::new(report)));
    Ok(out)
}

/// Writes the folder report as `target`, ready for `save_conversion`.
#[tauri::command]
pub async fn export_folder_report(selection_id: u64, target: String, state: State<'_, AppState>) -> Result<ConversionResult, AppError> {
    crate::folder::folder_path(&state, selection_id)?;
    let report = lock(&state.folder_report)
        .as_ref()
        .filter(|(id, _)| *id == selection_id)
        .map(|(_, r)| r.clone())
        .ok_or_else(|| AppError::new("stale", "Read the folder first."))?;
    let started = Instant::now();
    let (dir, file, size, original) = crate::blocking(move || {
        let bytes = render(&report, &target)?;
        let dir = tempfile::Builder::new()
            .prefix("myfile-report-")
            .tempdir()
            .map_err(|e| AppError::new("convert_failed", format!("Couldn't create a working folder: {e}")))?;
        let safe: String = report.name.chars().map(|c| if r#"\/:*?"<>|"#.contains(c) { '_' } else { c }).collect();
        let what = if target == "csv" { "file list" } else { "everything" };
        let file = PathBuf::from(format!("{safe} - {what}.{target}"));
        std::fs::write(dir.path().join(&file), &bytes).map_err(|e| AppError::new("convert_failed", format!("Couldn't write the report: {e}")))?;
        Ok((dir, file, bytes.len() as u64, report.entries.iter().map(|e| e.size).sum::<u64>()))
    })
    .await?;
    let id = state.new_id();
    let out = ConversionResult {
        conversion_id: id,
        files: vec![OutFile { name: file.to_string_lossy().into_owned(), size_bytes: size }],
        total_bytes: size,
        original_bytes: original,
        preview: None,
        elapsed_ms: started.elapsed().as_millis() as u64,
        engine: Engine::Builtin,
    };
    *lock(&state.conversion) = Some(Conversion { id, selection_id, dir, files: vec![file] });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let p = d.path();
        std::fs::create_dir_all(p.join("notes/deep")).unwrap();
        std::fs::copy("../test-files/sample-syllabus.pdf", p.join("syllabus.pdf")).unwrap();
        std::fs::write(p.join("notes/week1.md"), "# Week 1\n\nCells are **small**.\n\n- nucleus\n- membrane\n").unwrap();
        std::fs::write(p.join("notes/deep/grades.csv"), "name,score\nAda,9.5\nBo,7\n").unwrap();
        std::fs::write(p.join("script.py"), "print('hi')\n").unwrap();
        std::fs::write(p.join("blob.bin"), [0u8, 1, 2, 3, 0, 9]).unwrap();
        std::fs::write(p.join(".hidden.txt"), "secret").unwrap();
        std::fs::copy("../test-files/sample-image.png", p.join("photo.png")).unwrap();
        let docx = docx::write(&[vec![Block::Heading(1, "Lease".into()), Block::Para("Rent is $1,450.".into())]], "lease").unwrap();
        std::fs::write(p.join("notes/lease.docx"), docx).unwrap();
        {
            let mut z = zip::ZipWriter::new(std::fs::File::create(p.join("bundle.zip")).unwrap());
            z.start_file("a/inside.txt", zip::write::SimpleFileOptions::default()).unwrap();
            std::io::Write::write_all(&mut z, b"hello").unwrap();
            z.finish().unwrap();
        }
        d
    }

    fn build_it(dir: &Path) -> Report {
        let cancel = Arc::new(AtomicBool::new(false));
        build(dir, "Class", &Tools::default(), None, &cancel, None, &|_, _, _| {}).unwrap()
    }

    #[test]
    fn walks_every_file_but_hidden_ones() {
        let d = folder();
        let (files, over) = walk(d.path());
        let names: Vec<&str> = files.iter().map(|f| f.1.as_str()).collect();
        assert_eq!(names, ["blob.bin", "bundle.zip", "notes/deep/grades.csv", "notes/lease.docx", "notes/week1.md", "photo.png", "script.py", "syllabus.pdf"]);
        assert_eq!(over, 0);
    }

    #[test]
    fn reads_each_kind_of_file() {
        let d = folder();
        let r = build_it(d.path());
        let get = |n: &str| r.entries.iter().find(|e| e.rel == n).unwrap();
        let pdf = get("syllabus.pdf");
        assert!(pdf.details.contains(&"2 pages".to_string()), "{:?}", pdf.details);
        assert!(pdf.blocks.contains(&Block::Heading(2, "Grading".into())));
        assert_eq!(get("notes/week1.md").blocks[0], Block::Heading(1, "Week 1".into()));
        let csv = get("notes/deep/grades.csv");
        assert_eq!(csv.details, ["2 rows"]);
        assert!(matches!(&csv.blocks[0], Block::Code(l) if l[0] == "name | score" && l[2] == "Ada  | 9.5"), "{:?}", csv.blocks);
        assert!(get("notes/lease.docx").blocks.contains(&Block::Para("Rent is $1,450.".into())));
        assert_eq!(get("script.py").blocks, vec![Block::Code(vec!["print('hi')".into()])]);
        assert_eq!(get("blob.bin").note.as_deref(), Some("MyFile can't read inside this kind of file."));
        let zip = get("bundle.zip");
        assert_eq!(zip.details, ["1 file inside"]);
        assert_eq!(zip.blocks, vec![Block::Item("a/inside.txt".into())]);
        assert!(get("photo.png").details[0].ends_with("pixels"));
    }

    #[test]
    fn renders_every_format() {
        let d = folder();
        let r = build_it(d.path());
        let md = String::from_utf8(render(&r, "md").unwrap()).unwrap();
        assert!(md.starts_with("# Class\n\n8 files"), "{md}");
        assert!(md.contains("## notes/week1.md") && md.contains("### Week 1") && md.contains("- nucleus"), "{md}");
        let json: serde_json::Value = serde_json::from_slice(&render(&r, "json").unwrap()).unwrap();
        assert_eq!(json["files"].as_array().unwrap().len(), 8);
        assert!(json["files"][7]["text"].as_str().unwrap().contains("Grading"));
        let csv = String::from_utf8(render(&r, "csv").unwrap()).unwrap();
        assert!(csv.starts_with("path,type,size_bytes,modified,details,words,note\n"));
        assert_eq!(csv.lines().count(), 9);
        let docx = render(&r, "docx").unwrap();
        let xml = {
            let mut z = zip::ZipArchive::new(std::io::Cursor::new(docx)).unwrap();
            let mut s = String::new();
            z.by_name("word/document.xml").unwrap().read_to_string(&mut s).unwrap();
            s
        };
        assert!(xml.contains("notes/lease.docx") && xml.contains("Rent is $1,450."));
        let pdf = render(&r, "pdf").unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        assert!(render(&r, "exe").is_err());
    }

    #[test]
    fn unchanged_files_are_reused() {
        let d = folder();
        let first = build_it(d.path());
        std::fs::write(d.path().join("script.py"), "print('changed')\n").unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let second = build(d.path(), "Class", &Tools::default(), None, &cancel, Some(&first), &|_, _, _| {}).unwrap();
        let py = second.entries.iter().find(|e| e.rel == "script.py").unwrap();
        assert_eq!(py.blocks, vec![Block::Code(vec!["print('changed')".into()])]);
    }

    #[test]
    fn caps_and_dates() {
        let mut b = vec![Block::Para("a".repeat(10)), Block::Para("b".repeat(10))];
        assert!(cap_blocks(&mut b, 15, "this file"));
        assert_eq!(b, vec![Block::Para("a".repeat(10)), Block::Para("(MyFile kept the first 15 characters of this file.)".into())]);
        assert_eq!(group(5_000_000), "5,000,000");
        assert_eq!(date(SystemTime::now()), chrono::Local::now().format("%Y-%m-%d").to_string());
    }
}
