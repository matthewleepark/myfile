//! Folder watch: notices new documents (e.g. a syllabus landing in Downloads)
//! and, if the user turned it on, reads them for deadlines automatically.
//!
//! Ordinary code does everything except the reading: it watches one folder
//! (not subfolders), ignores in-progress downloads, waits for a file to stop
//! changing, checks the real file type, skips files it already handled, and
//! sends a system notification. Standing permission is chosen explicitly when
//! watching starts, shown the whole time, and ends when watching stops or the
//! app closes. Nothing about the watch is saved to disk.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use notify::{EventKind, RecursiveMode, Watcher};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::{mpsc, oneshot};

use crate::ai::{self, AiTask, ClaudeCli, DocumentText};
use crate::error::AppError;
use crate::files::{self, FileKind};
use crate::{lock, AppState};

pub struct WatchSession {
    pub id: u64,
    pub auto_read: bool,
    /// Calendar files found for new documents, by event number. Kept here
    /// (not with per-file results, which clear when you switch files) and
    /// dropped when watching stops.
    calendars: HashMap<u64, (String, Vec<u8>)>,
    /// In-flight automatic reads; all are cancelled when watching stops.
    cancels: Vec<oneshot::Sender<()>>,
    // Dropping the watcher stops watching.
    _watcher: notify::RecommendedWatcher,
}

impl Drop for WatchSession {
    /// Stopping (or replacing) a watch also stops any read it started, so
    /// nothing is sent after the user withdrew permission.
    fn drop(&mut self) {
        for tx in self.cancels.drain(..) {
            let _ = tx.send(());
        }
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct WatchInfo {
    watch_id: u64,
    folder: String,
    auto_read: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct WatchEvent {
    watch_id: u64,
    seq: u64,
    name: String,
    /// Full path, so the user can open it in MyFile with one click.
    path: String,
    /// "new" (detected, not read), "reading", "done", or "error".
    status: &'static str,
    message: String,
    items: usize,
    /// Dated items available as a calendar file (save with save_watch_calendar).
    calendar_events: usize,
}

/// Names of files still being written by a browser or app.
pub fn is_partial_download(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.starts_with("~$")
        || lower.starts_with('.')
        || [".crdownload", ".part", ".partial", ".tmp", ".download", ".opdownload"].iter().any(|e| lower.ends_with(e))
}

#[tauri::command]
pub fn start_watch(path: String, auto_read: bool, app: AppHandle, state: State<AppState>) -> Result<WatchInfo, AppError> {
    let dir = PathBuf::from(&path);
    if !dir.is_dir() {
        return Err(AppError::new("not_a_folder", "That isn't a folder."));
    }
    // Only one watched folder at a time.
    *lock(&state.watch) = None;
    let id = state.new_id();

    let (tx, rx) = mpsc::unbounded_channel::<PathBuf>();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(event) = res {
            if matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_)) {
                for p in event.paths {
                    let _ = tx.send(p);
                }
            }
        }
    })
    .map_err(|e| AppError::new("watch_failed", format!("MyFile couldn't watch that folder ({e}).")))?;
    watcher
        .watch(&dir, RecursiveMode::NonRecursive)
        .map_err(|e| AppError::new("watch_failed", format!("MyFile couldn't watch that folder ({e}).")))?;

    let folder = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(path);
    let canonical = dir.canonicalize().map_err(AppError::from_read)?;
    *lock(&state.watch) =
        Some(WatchSession { id, auto_read, calendars: HashMap::new(), cancels: Vec::new(), _watcher: watcher });
    tauri::async_runtime::spawn(handle_events(app, id, canonical, rx));
    Ok(WatchInfo { watch_id: id, folder, auto_read })
}

#[tauri::command]
pub fn stop_watch(state: State<AppState>) {
    *lock(&state.watch) = None;
}

/// Saves the calendar found for one new document.
#[tauri::command]
pub async fn save_watch_calendar(watch_id: u64, seq: u64, app: AppHandle, state: State<'_, AppState>) -> Result<Option<String>, AppError> {
    let (file_name, bytes) = {
        let guard = lock(&state.watch);
        let w = guard
            .as_ref()
            .filter(|w| w.id == watch_id)
            .ok_or_else(|| AppError::new("stale", "Watching has stopped, so that calendar is gone."))?;
        let (name, bytes) = w.calendars.get(&seq).cloned().ok_or_else(|| AppError::new("no_output", "That calendar isn't available."))?;
        (name, bytes)
    };
    // Not defaulting to the watched folder keeps saved calendars out of the watch.
    crate::save_bytes(&app, bytes, &file_name, "Calendar file", "ics", None).await
}

/// Debounces raw file-system events into one "new document" per file.
async fn handle_events(app: AppHandle, watch_id: u64, folder: PathBuf, mut rx: mpsc::UnboundedReceiver<PathBuf>) {
    let mut pending: HashMap<PathBuf, Instant> = HashMap::new();
    let mut handled: HashMap<PathBuf, (u64, Option<SystemTime>)> = HashMap::new();
    let mut seq = 0u64;
    let mut tick = tokio::time::interval(Duration::from_millis(500));
    loop {
        tokio::select! {
            p = rx.recv() => match p {
                Some(p) => { pending.insert(p, Instant::now()); }
                None => return, // watcher dropped
            },
            _ = tick.tick() => {}
        }
        if !still_watching(&app, watch_id) {
            return;
        }
        // A file is ready once nothing has touched it for 1.5 s.
        let ready: Vec<PathBuf> = pending
            .iter()
            .filter(|(_, t)| t.elapsed() >= Duration::from_millis(1500))
            .map(|(p, _)| p.clone())
            .collect();
        for path in ready {
            pending.remove(&path);
            let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
            // Skip MyFile's own calendar files, so saving one here doesn't trigger another read.
            if is_partial_download(&name) || name.to_lowercase().ends_with(".ics") {
                continue;
            }
            // Never follow links: a symlink named syllabus.md could point at a private file
            // elsewhere. The file must really live directly in the watched folder.
            let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
            if !meta.file_type().is_file() {
                continue;
            }
            let in_folder = path
                .canonicalize()
                .ok()
                .and_then(|c| c.parent().map(|p| p == folder.as_path()))
                .unwrap_or(false);
            if !in_folder {
                continue;
            }
            let fingerprint = (meta.len(), meta.modified().ok());
            if meta.len() == 0 || handled.get(&path) == Some(&fingerprint) {
                continue;
            }
            let Ok(info) = files::inspect(&path) else { continue };
            if !matches!(info.kind, FileKind::Pdf | FileKind::Text) {
                continue;
            }
            handled.insert(path.clone(), fingerprint);
            seq += 1;
            let app = app.clone();
            tauri::async_runtime::spawn(process(app, watch_id, seq, path, name, info.kind));
        }
    }
}

fn still_watching(app: &AppHandle, watch_id: u64) -> bool {
    let state = app.state::<AppState>();
    let watching = lock(&state.watch).as_ref().is_some_and(|w| w.id == watch_id);
    watching
}

fn auto_read(app: &AppHandle, watch_id: u64) -> bool {
    let state = app.state::<AppState>();
    let auto = lock(&state.watch).as_ref().is_some_and(|w| w.id == watch_id && w.auto_read);
    auto
}

fn notify_user(app: &AppHandle, title: &str, body: &str) {
    let _ = app.notification().builder().title(title).body(body).show();
}

async fn process(app: AppHandle, watch_id: u64, seq: u64, path: PathBuf, name: String, kind: FileKind) {
    let base = WatchEvent {
        watch_id,
        seq,
        name: name.clone(),
        path: path.to_string_lossy().into_owned(),
        status: "new",
        message: String::new(),
        items: 0,
        calendar_events: 0,
    };
    if !auto_read(&app, watch_id) {
        let _ = app.emit("watch-event", WatchEvent { message: "New document. Open it in MyFile to work with it.".into(), ..base });
        notify_user(&app, "New document", &format!("{name} arrived. Open MyFile to read it."));
        return;
    }
    let _ = app.emit("watch-event", WatchEvent { status: "reading", message: "Reading for deadlines…".into(), ..base.clone() });
    let result = read_deadlines(&app, watch_id, &path, &name, kind).await;
    // The user may have stopped watching while this ran.
    if !still_watching(&app, watch_id) {
        return;
    }
    match result {
        Ok((items, dated, verified, checked, calendar)) => {
            if let Some(bytes) = calendar {
                let state = app.state::<AppState>();
                if let Some(w) = lock(&state.watch).as_mut().filter(|w| w.id == watch_id) {
                    w.calendars.insert(seq, (format!("{}-deadlines.ics", crate::stem(&name)), bytes));
                }
            }
            let message = if items == 0 {
                "No deadlines found.".to_string()
            } else {
                format!("{items} item(s), {dated} with a calendar date; {verified}/{checked} quotes verified.")
            };
            let _ = app.emit(
                "watch-event",
                WatchEvent { status: "done", message: message.clone(), items, calendar_events: dated, ..base },
            );
            if items > 0 {
                notify_user(&app, &format!("MyFile found {items} deadline{} in {name}", if items == 1 { "" } else { "s" }), &message);
            }
        }
        Err(e) => {
            let _ = app.emit("watch-event", WatchEvent { status: "error", message: e.message, ..base });
        }
    }
}

/// Items, dated items, verified quotes, checked quotes, and the .ics bytes.
type ReadOutcome = (usize, usize, usize, usize, Option<Vec<u8>>);

async fn read_deadlines(app: &AppHandle, watch_id: u64, path: &Path, name: &str, kind: FileKind) -> Result<ReadOutcome, AppError> {
    let p = path.to_path_buf();
    let doc = crate::blocking(move || crate::load_document(&p, kind)).await?;
    let chars: usize = doc.pages.iter().map(|p| p.chars().count()).sum();
    if chars == 0 || chars > ai::MAX_DOCUMENT_CHARS {
        return Err(AppError::new("too_long", "This document is empty or too long to read automatically."));
    }
    let mut cli = ClaudeCli::locate(app.path().home_dir().ok().as_deref())
        .ok_or_else(|| AppError::new("ai_unavailable", "Automatic reading needs the Claude Code CLI."))?;
    cli.model = std::env::var("MYFILE_FOLDER_MODEL").ok().filter(|m| !m.trim().is_empty()).unwrap_or_else(|| "haiku".into());
    let text = DocumentText { name, pages: &doc.pages, has_pages: doc.has_pages };
    // Extraction can take a while; check the standing permission again right
    // before anything is sent, and register so Stop can cancel the request.
    let (tx, rx) = oneshot::channel::<()>();
    {
        let state = app.state::<AppState>();
        let mut guard = lock(&state.watch);
        match guard.as_mut().filter(|w| w.id == watch_id && w.auto_read) {
            Some(w) => w.cancels.push(tx),
            None => return Err(AppError::new("watch_stopped", "Watching stopped before this document was sent.")),
        }
    }
    let started = Instant::now();
    let reply = cli.run(ai::build_prompt(AiTask::Deadlines, None, &text), rx).await?;
    let answer = ai::validate(AiTask::Deadlines, reply, &text, started)?;
    let dated = crate::ics::event_count(&answer.items);
    let calendar = (dated > 0).then(|| crate::ics::build(name, &answer.items, SystemTime::now()).into_bytes());
    Ok((answer.items.len(), dated, answer.checks.quotes_verified, answer.checks.quotes_checked, calendar))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_files_still_downloading() {
        for n in ["syllabus.pdf.crdownload", "a.part", "Unconfirmed 1234.crdownload", "x.tmp", "~$report.docx", ".DS_Store", "f.opdownload"] {
            assert!(is_partial_download(n), "{n}");
        }
        for n in ["syllabus.pdf", "notes.md", "partial-report.pdf"] {
            assert!(!is_partial_download(n), "{n}");
        }
    }
}
