//! MyFile as an MCP server: lets Claude Code (or any MCP client) call
//! MyFile's local abilities as tools.
//!
//! The split is the same as in the app. The tools are deterministic local
//! code; the model calling them does the reasoning. Notably, `verify_quote`
//! lets the model check its own quotes against the real document text.
//!
//! Safety:
//! - Every path must resolve inside `root` (the user's home folder unless
//!   `--root` says otherwise).
//! - Writes only ever create new files; nothing is overwritten or deleted.
//! - Calendar dates are validated by code before an .ics is written.
//!
//! Transport: JSON-RPC 2.0, one message per line on stdin/stdout. Nothing
//! else may be printed to stdout.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::{json, Value};

use crate::ai::{self, AiItem, Basis};
use crate::error::{human_size, AppError};
use crate::files::{self, FileKind};
use crate::image_ops::{self, CompressOptions, OutputFormat};

const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
const DEFAULT_READ_CHARS: usize = 60_000;
const MAX_READ_CHARS: usize = 300_000;
const MAX_LISTED: usize = 200;

pub struct Server {
    root: PathBuf,
}

/// Runs the server until stdin closes.
pub fn run_stdio(root: PathBuf) -> std::io::Result<()> {
    let root = root.canonicalize()?;
    let server = Server::new(root);
    eprintln!("myfile-mcp: serving files under {}", server.root.display());
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = server.handle_line(&line) {
            writeln!(stdout, "{reply}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

impl Server {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Handles one JSON-RPC message; returns the reply line, if any.
    pub fn handle_line(&self, line: &str) -> Option<String> {
        let msg: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => return Some(rpc_error(Value::Null, -32700, "Parse error").to_string()),
        };
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        // Notifications (no id) get no reply.
        let id = id?;
        let result = match method {
            "initialize" => Ok(self.initialize(&params)),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tool_definitions() })),
            "tools/call" => Ok(self.call_tool(&params)),
            _ => Err((-32601, format!("Method not found: {method}"))),
        };
        Some(match result {
            Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }).to_string(),
            Err((code, m)) => rpc_error(id, code, &m).to_string(),
        })
    }

    fn initialize(&self, params: &Value) -> Value {
        let asked = params.get("protocolVersion").and_then(|v| v.as_str()).unwrap_or("");
        let version = if PROTOCOL_VERSIONS.contains(&asked) { asked } else { PROTOCOL_VERSIONS[0] };
        json!({
            "protocolVersion": version,
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": "myfile", "version": env!("CARGO_PKG_VERSION") },
            "instructions": "Local file tools from MyFile. Read documents with read_document; before \
                stating a fact from a document, confirm the exact supporting sentence with verify_quote. \
                Files are never modified: compress_image and export_calendar only create new files."
        })
    }

    fn call_tool(&self, params: &Value) -> Value {
        let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let args = params.get("arguments").cloned().unwrap_or(json!({}));
        let outcome = match name {
            "list_documents" => self.list_documents(&args),
            "inspect_file" => self.inspect_file(&args),
            "read_document" => self.read_document(&args),
            "verify_quote" => self.verify_quote(&args),
            "compress_image" => self.compress_image(&args),
            "export_calendar" => self.export_calendar(&args),
            _ => Err(AppError::new("unknown_tool", format!("There is no tool named “{name}”."))),
        };
        match outcome {
            Ok(text) => json!({ "content": [{ "type": "text", "text": text }], "isError": false }),
            Err(e) => json!({ "content": [{ "type": "text", "text": e.message }], "isError": true }),
        }
    }

    // --- Paths ---------------------------------------------------------------

    /// Resolves an existing path and checks it is inside the root.
    fn existing(&self, raw: &str) -> Result<PathBuf, AppError> {
        let p = self.join(raw)?;
        let canon = p.canonicalize().map_err(AppError::from_read)?;
        self.inside(&canon)?;
        Ok(canon)
    }

    /// Resolves a path for a new file: its folder must exist inside the root
    /// and the file itself must not exist yet.
    fn new_file(&self, raw: &str, extension: &str) -> Result<PathBuf, AppError> {
        let p = self.join(raw)?;
        let name = p
            .file_name()
            .ok_or_else(|| AppError::new("bad_path", "The output path needs a file name."))?
            .to_os_string();
        // A ':' in the name would write a hidden NTFS stream onto another
        // file ("notes.txt:x.ics"), modifying it; control characters are never valid.
        if name.to_string_lossy().chars().any(|c| c == ':' || c.is_control()) {
            return Err(AppError::new("bad_path", "The output file name can't contain ':' or control characters."));
        }
        if p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()) != Some(extension.into()) {
            return Err(AppError::new("bad_path", format!("The output file must end in .{extension}.")));
        }
        let parent = p.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(&self.root);
        let dir = parent.canonicalize().map_err(AppError::from_read)?;
        self.inside(&dir)?;
        Ok(dir.join(name))
    }

    fn join(&self, raw: &str) -> Result<PathBuf, AppError> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Err(AppError::new("bad_path", "A path is required."));
        }
        let p = Path::new(raw);
        Ok(if p.is_absolute() { p.to_path_buf() } else { self.root.join(p) })
    }

    fn inside(&self, canon: &Path) -> Result<(), AppError> {
        if canon.starts_with(&self.root) {
            Ok(())
        } else {
            Err(AppError::new(
                "outside_root",
                format!("That path is outside the folder MyFile is allowed to use ({}).", self.root.display()),
            ))
        }
    }

    fn shown(&self, p: &Path) -> String {
        p.strip_prefix(&self.root).unwrap_or(p).display().to_string()
    }

    // --- Tools -----------------------------------------------------------------

    fn list_documents(&self, args: &Value) -> Result<String, AppError> {
        let folder = self.existing(str_arg(args, "folder").unwrap_or("."))?;
        if !folder.is_dir() {
            return Err(AppError::new("not_a_folder", "That path isn't a folder."));
        }
        let recursive = args.get("recursive").and_then(|v| v.as_bool()).unwrap_or(false);
        let mut found = Vec::new();
        let mut stack = vec![(folder.clone(), 0usize)];
        let mut truncated = false;
        while let Some((dir, depth)) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            let mut entries: Vec<_> = entries.flatten().collect();
            entries.sort_by_key(|e| e.file_name());
            for entry in entries {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    continue;
                }
                // Never follow links: a junction to C:\Windows would list files outside the root.
                let Ok(ft) = entry.file_type() else { continue };
                if ft.is_symlink() || !path.canonicalize().is_ok_and(|c| c.starts_with(&self.root)) {
                    continue;
                }
                if ft.is_dir() {
                    if recursive && depth < 3 {
                        stack.push((path, depth + 1));
                    }
                    continue;
                }
                let Ok(info) = files::inspect(&path) else { continue };
                if info.kind == FileKind::Unsupported {
                    continue;
                }
                if found.len() >= MAX_LISTED {
                    truncated = true;
                    break;
                }
                found.push(format!("- {} ({}, {})", self.shown(&path), info.type_label, human_size(info.size_bytes)));
            }
        }
        let mut out = format!("{} supported file(s) in {}:\n", found.len(), self.shown(&folder));
        out.push_str(&found.join("\n"));
        if truncated {
            out.push_str(&format!("\n(Stopped at {MAX_LISTED} files.)"));
        }
        Ok(out)
    }

    fn inspect_file(&self, args: &Value) -> Result<String, AppError> {
        let path = self.existing(required(args, "path")?)?;
        let info = files::inspect(&path)?;
        Ok(format!(
            "{}: {} ({}){}",
            info.name,
            info.type_label,
            human_size(info.size_bytes),
            if info.extension_mismatch { ". Note: the extension doesn't match the contents." } else { "" }
        ))
    }

    fn read_document(&self, args: &Value) -> Result<String, AppError> {
        let path = self.existing(required(args, "path")?)?;
        let info = files::inspect(&path)?;
        let doc = crate::load_document(&path, info.kind)?;
        let limit = args
            .get("max_chars")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).clamp(1_000, MAX_READ_CHARS))
            .unwrap_or(DEFAULT_READ_CHARS);
        let mut text = String::new();
        for (i, page) in doc.pages.iter().enumerate() {
            if doc.has_pages {
                text.push_str(&format!("[Page {}]\n", i + 1));
            }
            text.push_str(page);
            text.push('\n');
        }
        let total = text.chars().count();
        let mut out = format!(
            "{} ({}{}), {} characters.\n\n",
            info.name,
            info.type_label,
            if doc.has_pages { format!(", {} pages", doc.pages.len()) } else { String::new() },
            total
        );
        out.extend(text.chars().take(limit));
        if total > limit {
            out.push_str(&format!("\n\n[Truncated at {limit} of {total} characters. Pass a larger max_chars to read more.]"));
        }
        Ok(out)
    }

    fn verify_quote(&self, args: &Value) -> Result<String, AppError> {
        let path = self.existing(required(args, "path")?)?;
        let quote = required(args, "quote")?;
        let info = files::inspect(&path)?;
        let doc = crate::load_document(&path, info.kind)?;
        let needle = ai::normalize(quote);
        if needle.is_empty() {
            return Err(AppError::new("bad_quote", "The quote is empty."));
        }
        let pages: Vec<usize> = doc
            .pages
            .iter()
            .enumerate()
            .filter(|(_, p)| ai::normalize(p).contains(&needle))
            .map(|(i, _)| i + 1)
            .collect();
        Ok(if pages.is_empty() {
            "NOT FOUND: this exact passage does not appear in the document (ignoring case, spacing, and quote/dash style). \
             Do not present it as a quote."
                .to_string()
        } else if doc.has_pages {
            format!("FOUND on page(s) {}.", pages.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", "))
        } else {
            "FOUND in the document.".to_string()
        })
    }

    fn compress_image(&self, args: &Value) -> Result<String, AppError> {
        let path = self.existing(required(args, "path")?)?;
        let info = files::inspect(&path)?;
        if info.kind != FileKind::Image {
            return Err(AppError::new("wrong_kind", "That file isn't an image MyFile can compress."));
        }
        let format = match str_arg(args, "format").unwrap_or("jpeg") {
            "png" => OutputFormat::Png,
            _ => OutputFormat::Jpeg,
        };
        let quality = args.get("quality").and_then(|v| v.as_u64()).unwrap_or(75).clamp(30, 95) as u8;
        let max_dimension = args.get("max_dimension").and_then(|v| v.as_u64()).map(|d| d.clamp(16, 10_000) as u32);
        let opts = CompressOptions { format, quality, max_dimension };
        let out = image_ops::compress(&path, &opts)?;

        // Always a new file next to the original: name-compressed.jpg, then name-compressed-2.jpg, ...
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "image".into());
        let dir = path.parent().unwrap_or(&self.root);
        let mut dest = None;
        for n in 1..1000 {
            let suffix = if n == 1 { String::new() } else { format!("-{n}") };
            let candidate = dir.join(format!("{stem}-compressed{suffix}.{}", format.extension()));
            match write_new(&candidate, &out.bytes) {
                Ok(()) => {
                    dest = Some(candidate);
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(AppError::new("save_failed", format!("Couldn't write the new image ({e})."))),
            }
        }
        let dest = dest.ok_or_else(|| AppError::new("exists", "Couldn't find a free file name."))?;
        let before = info.size_bytes as f64;
        let after = out.bytes.len() as f64;
        Ok(format!(
            "Wrote {} ({} → {}, {}; {}×{} px). The original was not changed.",
            self.shown(&dest),
            human_size(info.size_bytes),
            human_size(out.bytes.len() as u64),
            if after < before { format!("{:.1}% smaller", (1.0 - after / before) * 100.0) } else { "not smaller".into() },
            out.width,
            out.height
        ))
    }

    fn export_calendar(&self, args: &Value) -> Result<String, AppError> {
        let dest = self.new_file(required(args, "output_path")?, "ics")?;
        let events = args
            .get("events")
            .and_then(|v| v.as_array())
            .ok_or_else(|| AppError::new("bad_events", "events must be a list."))?;
        let mut items = Vec::new();
        let mut rejected = Vec::new();
        for e in events.iter().take(200) {
            let title = e.get("title").and_then(|v| v.as_str()).unwrap_or("").trim();
            let date = e.get("date").and_then(|v| v.as_str()).unwrap_or("").trim();
            if title.is_empty() || ai::parse_iso_date(date).is_none() {
                rejected.push(format!("{title:?} on {date:?}"));
                continue;
            }
            items.push(AiItem {
                title: title.chars().take(300).collect(),
                detail: e.get("details").and_then(|v| v.as_str()).unwrap_or("").chars().take(2000).collect(),
                category: None,
                date: Some(date.into()),
                iso_date: Some(date.into()),
                quote: None,
                quote_check: None,
                date_check: None,
                suspicious: false,
                page: None,
                basis: Basis::Explicit,
            });
        }
        if items.is_empty() {
            return Err(AppError::new("bad_events", "No valid events. Each needs a title and a real date as YYYY-MM-DD."));
        }
        let source = str_arg(args, "source").unwrap_or("documents");
        write_new(&dest, crate::ics::build(source, &items, SystemTime::now()).as_bytes()).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                AppError::new("exists", format!("{} already exists. MyFile never overwrites files; choose another name.", dest.display()))
            } else {
                AppError::new("save_failed", format!("Couldn't write the calendar ({e})."))
            }
        })?;
        let mut out = format!("Wrote {} with {} event(s).", self.shown(&dest), items.len());
        if !rejected.is_empty() {
            out.push_str(&format!(" Skipped {} invalid event(s): {}.", rejected.len(), rejected.join(", ")));
        }
        Ok(out)
    }
}

/// Creates a new file and fails if anything already exists at that path,
/// including a symlink, so a write can never land outside the root or
/// replace an existing file.
fn write_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
    f.write_all(bytes)
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|v| v.as_str())
}

fn required<'a>(args: &'a Value, key: &str) -> Result<&'a str, AppError> {
    str_arg(args, key).ok_or_else(|| AppError::new("missing_argument", format!("“{key}” is required.")))
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn tool_definitions() -> Value {
    json!([
        {
            "name": "list_documents",
            "title": "List documents",
            "description": "List the PDFs, text files, and images MyFile supports in a folder (detected by content, not extension).",
            "inputSchema": { "type": "object", "properties": {
                "folder": { "type": "string", "description": "Folder path, absolute or relative to the root. Defaults to the root." },
                "recursive": { "type": "boolean", "description": "Also look in subfolders (up to 3 levels)." }
            } }
        },
        {
            "name": "inspect_file",
            "title": "Inspect a file",
            "description": "Report a file's real type (from its contents) and size.",
            "inputSchema": { "type": "object", "required": ["path"], "properties": {
                "path": { "type": "string" }
            } }
        },
        {
            "name": "read_document",
            "title": "Read a document",
            "description": "Extract the text of a PDF or text file locally. PDFs include [Page N] markers.",
            "inputSchema": { "type": "object", "required": ["path"], "properties": {
                "path": { "type": "string" },
                "max_chars": { "type": "integer", "description": "Default 60000, maximum 300000." }
            } }
        },
        {
            "name": "verify_quote",
            "title": "Verify a quote",
            "description": "Check that an exact passage appears in a document and on which page(s). Use it before presenting any quote as evidence.",
            "inputSchema": { "type": "object", "required": ["path", "quote"], "properties": {
                "path": { "type": "string" },
                "quote": { "type": "string" }
            } }
        },
        {
            "name": "compress_image",
            "title": "Compress an image",
            "description": "Write a smaller copy of an image next to the original (never overwrites). JPEG for photos, PNG to keep transparency.",
            "inputSchema": { "type": "object", "required": ["path"], "properties": {
                "path": { "type": "string" },
                "format": { "type": "string", "enum": ["jpeg", "png"] },
                "quality": { "type": "integer", "description": "JPEG quality 30-95, default 75." },
                "max_dimension": { "type": "integer", "description": "Scale down so the longest side is at most this many pixels." }
            } }
        },
        {
            "name": "export_calendar",
            "title": "Export a calendar",
            "description": "Write an .ics file of all-day events. Dates must be real YYYY-MM-DD dates; invalid ones are skipped. Never overwrites.",
            "inputSchema": { "type": "object", "required": ["events", "output_path"], "properties": {
                "output_path": { "type": "string", "description": "New .ics file path inside the root." },
                "source": { "type": "string", "description": "Where the dates came from, noted in each event." },
                "events": { "type": "array", "items": { "type": "object", "required": ["title", "date"], "properties": {
                    "title": { "type": "string" },
                    "date": { "type": "string", "description": "YYYY-MM-DD" },
                    "details": { "type": "string" }
                } } }
            } }
        }
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server() -> (tempfile::TempDir, Server) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.txt"), "Project kickoff.\nThe report is due Friday, November 6, 2026.").unwrap();
        let root = dir.path().canonicalize().unwrap();
        (dir, Server::new(root))
    }

    fn call(s: &Server, name: &str, args: Value) -> Value {
        let line = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":args}}).to_string();
        serde_json::from_str::<Value>(&s.handle_line(&line).unwrap()).unwrap()["result"].clone()
    }

    fn text(v: &Value) -> String {
        v["content"][0]["text"].as_str().unwrap().to_string()
    }

    #[test]
    fn initialize_and_list_tools() {
        let (_d, s) = server();
        let init = s.handle_line(r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#).unwrap();
        assert!(init.contains("\"protocolVersion\":\"2025-06-18\"") && init.contains("\"name\":\"myfile\""));
        assert!(s.handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());
        let tools = s.handle_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#).unwrap();
        for t in ["list_documents", "read_document", "verify_quote", "compress_image", "export_calendar"] {
            assert!(tools.contains(t), "missing {t}");
        }
        let unknown = s.handle_line(r#"{"jsonrpc":"2.0","id":3,"method":"nope"}"#).unwrap();
        assert!(unknown.contains("-32601"));
    }

    #[test]
    fn reads_and_verifies_quotes() {
        let (_d, s) = server();
        assert!(text(&call(&s, "read_document", json!({"path":"notes.txt"}))).contains("November 6, 2026"));
        assert!(text(&call(&s, "verify_quote", json!({"path":"notes.txt","quote":"the REPORT is due  Friday, November 6, 2026."}))).starts_with("FOUND"));
        assert!(text(&call(&s, "verify_quote", json!({"path":"notes.txt","quote":"The report is due Monday."}))).starts_with("NOT FOUND"));
    }

    #[test]
    fn refuses_paths_outside_the_root() {
        let (_d, s) = server();
        let r = call(&s, "read_document", json!({"path":"../../../../Windows/win.ini"}));
        assert_eq!(r["isError"], true);
        let outside = std::env::temp_dir().canonicalize().unwrap();
        if !outside.starts_with(&s.root) {
            let r = call(&s, "list_documents", json!({"folder": outside.to_string_lossy()}));
            assert_eq!(r["isError"], true);
        }
    }

    #[test]
    fn calendar_validates_dates_and_never_overwrites() {
        let (d, s) = server();
        let r = call(&s, "export_calendar", json!({"output_path":"due.ics","events":[
            {"title":"Report","date":"2026-11-06"}, {"title":"Bogus","date":"2026-02-30"}]}));
        assert!(text(&r).contains("1 event(s)") && text(&r).contains("Skipped 1"), "{}", text(&r));
        assert!(std::fs::read_to_string(d.path().join("due.ics")).unwrap().contains("DTSTART;VALUE=DATE:20261106"));
        let again = call(&s, "export_calendar", json!({"output_path":"due.ics","events":[{"title":"x","date":"2026-11-06"}]}));
        assert_eq!(again["isError"], true);
        let wrong_ext = call(&s, "export_calendar", json!({"output_path":"due.txt","events":[{"title":"x","date":"2026-11-06"}]}));
        assert_eq!(wrong_ext["isError"], true);
    }

    #[test]
    fn output_names_cannot_hide_a_stream_in_another_file() {
        let (d, s) = server();
        let r = call(&s, "export_calendar", json!({"output_path":"notes.txt:x.ics","events":[{"title":"x","date":"2026-11-06"}]}));
        assert_eq!(r["isError"], true);
        let unchanged = "Project kickoff.\nThe report is due Friday, November 6, 2026.";
        assert_eq!(std::fs::read_to_string(d.path().join("notes.txt")).unwrap(), unchanged);
    }

    #[test]
    fn compress_writes_a_new_file_next_to_the_original() {
        let (d, s) = server();
        let img = image::RgbaImage::from_pixel(64, 32, image::Rgba([200, 30, 30, 255]));
        img.save(d.path().join("red.png")).unwrap();
        let r = call(&s, "compress_image", json!({"path":"red.png","format":"jpeg","max_dimension":32}));
        assert!(text(&r).contains("red-compressed.jpg"), "{}", text(&r));
        let r2 = call(&s, "compress_image", json!({"path":"red.png"}));
        assert!(text(&r2).contains("red-compressed-2.jpg"));
        assert!(d.path().join("red.png").exists());
    }
}
