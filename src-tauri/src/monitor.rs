//! Document change monitor: notices when a document you care about changes,
//! and says *what* changed rather than "file modified".
//!
//! Ordinary code does all the detection: it keeps a text snapshot of each
//! monitored document (in MyFile's app-data folder; the user is told, and
//! "Stop monitoring" deletes it), fingerprints the text, and re-checks when
//! the file changes (while MyFile is open) or at startup. A change that leaves
//! the text identical is reported as metadata-only. Real changes go through
//! the same diff engine as comparison, and the summary (counts and moved
//! dates) is computed by code. The AI is only involved if the user asks for
//! an explanation, through the existing comparison flow.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use notify::{EventKind, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::mpsc;

use crate::compare::{self, StoredComparison};
use crate::error::AppError;
use crate::files::{self, FileKind};
use crate::{lock, AppState};

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    /// FNV-1a fingerprint of the normalized text.
    hash: String,
    pages: Vec<String>,
    has_pages: bool,
    taken_at: u64,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Entry {
    id: u64,
    path: PathBuf,
    name: String,
    snapshot: Snapshot,
    /// File size and modified time at the snapshot, to tell metadata-only changes apart.
    size: u64,
    modified: u64,
}

#[derive(Serialize, Deserialize, Default)]
struct Store {
    next_id: u64,
    entries: Vec<Entry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Unchanged,
    /// The file was touched or re-saved, but its text is the same.
    MetadataOnly,
    TextChanged,
    Missing,
    Unreadable,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MonitorItem {
    id: u64,
    name: String,
    status: Status,
    /// Computed by code, e.g. "Deadline moved Nov 12, 2026 → Nov 17, 2026 · 5 other changes".
    summary: String,
    snapshot_taken_at: u64,
}

/// A stable 64-bit fingerprint of the text (whitespace-normalized), so
/// re-wrapped but identical text counts as unchanged.
pub fn fingerprint(pages: &[String]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for word in pages.iter().flat_map(|p| p.split_whitespace()) {
        for b in word.bytes().chain(std::iter::once(b' ')) {
            h ^= b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    format!("{h:016x}")
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn file_stamp(path: &Path) -> Option<(u64, u64)> {
    let m = std::fs::metadata(path).ok()?;
    let modified = m.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_secs();
    Some((m.len(), modified))
}

fn store_path(app: &AppHandle) -> Result<PathBuf, AppError> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::new("storage", format!("MyFile couldn't find its data folder ({e}).")))?;
    std::fs::create_dir_all(&dir).map_err(|e| AppError::new("storage", format!("MyFile couldn't create its data folder ({e}).")))?;
    Ok(dir.join("monitored-documents.json"))
}

fn load(app: &AppHandle) -> Result<Store, AppError> {
    let p = store_path(app)?;
    match std::fs::read_to_string(&p) {
        Ok(s) => serde_json::from_str(&s).map_err(|_| AppError::new("storage", "MyFile's list of monitored documents is damaged.")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Store::default()),
        Err(e) => Err(AppError::new("storage", format!("Couldn't read the monitored-documents list ({e})."))),
    }
}

fn save(app: &AppHandle, store: &Store) -> Result<(), AppError> {
    let p = store_path(app)?;
    let json = serde_json::to_vec_pretty(store).map_err(|_| AppError::new("storage", "Couldn't save the monitored-documents list."))?;
    crate::write_atomically(&p, &json)
}

fn snapshot_of(path: &Path) -> Result<(Snapshot, FileKind), AppError> {
    let info = files::inspect(path)?;
    if !matches!(info.kind, FileKind::Pdf | FileKind::Text) {
        return Err(AppError::new("wrong_kind", "Only PDFs and text files can be monitored."));
    }
    let doc = crate::load_document(path, info.kind)?;
    Ok((Snapshot { hash: fingerprint(&doc.pages), pages: doc.pages, has_pages: doc.has_pages, taken_at: now() }, info.kind))
}

/// Status and code-computed summary of one entry, re-reading the file.
fn check(entry: &Entry) -> (Status, String, Option<Snapshot>) {
    if !entry.path.is_file() {
        return (Status::Missing, "The file is gone (moved, renamed, or deleted).".into(), None);
    }
    let (current, _) = match snapshot_of(&entry.path) {
        Ok(s) => s,
        Err(e) => return (Status::Unreadable, e.message, None),
    };
    if current.hash == entry.snapshot.hash {
        let touched = file_stamp(&entry.path).is_some_and(|(s, m)| s != entry.size || m != entry.modified);
        return if touched {
            (Status::MetadataOnly, "The file was saved again, but its text didn't change.".into(), Some(current))
        } else {
            (Status::Unchanged, "No changes since the snapshot.".into(), Some(current))
        };
    }
    let d = compare::diff(
        &compare::sentences(&entry.snapshot.pages, entry.snapshot.has_pages),
        &compare::sentences(&current.pages, current.has_pages),
    );
    (Status::TextChanged, summarize(&d), Some(current))
}

/// "Date moved Nov 12 → Nov 17 · 1 added, 2 removed, 3 changed"
pub fn summarize(d: &compare::Diff) -> String {
    let dates: Vec<String> = d.changes.iter().filter_map(|c| c.date_change.clone()).collect();
    let mut parts = Vec::new();
    match dates.len() {
        0 => {}
        1 => parts.push(format!("Date changed: {}", dates[0])),
        n => parts.push(format!("{n} dates changed (first: {})", dates[0])),
    }
    let mut counts = Vec::new();
    for (n, label) in [(d.added, "added"), (d.removed, "removed"), (d.changed, "changed")] {
        if n > 0 {
            counts.push(format!("{n} {label}"));
        }
    }
    if !counts.is_empty() {
        parts.push(format!("{} sentence(s)", counts.join(", ")));
    }
    if parts.is_empty() {
        "The text changed.".into()
    } else {
        parts.join(" · ")
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Starts monitoring the open file (takes a text snapshot).
#[tauri::command]
pub async fn monitor_add(selection_id: u64, app: AppHandle, state: State<'_, AppState>) -> Result<MonitorItem, AppError> {
    crate::require_current(&state, selection_id)?;
    let (_, path, info) = state.current()?;
    let canon = path.canonicalize().map_err(AppError::from_read)?;
    let p = canon.clone();
    let (snapshot, _) = crate::blocking(move || snapshot_of(&p)).await?;
    let (size, modified) = file_stamp(&canon).unwrap_or((0, 0));
    let mut store = load(&app)?;
    if let Some(existing) = store.entries.iter().find(|e| e.path == canon) {
        return Err(AppError::new("exists", format!("“{}” is already being monitored.", existing.name)));
    }
    store.next_id += 1;
    let entry = Entry { id: store.next_id, path: canon, name: info.name, snapshot, size, modified };
    let item = MonitorItem {
        id: entry.id,
        name: entry.name.clone(),
        status: Status::Unchanged,
        summary: "Snapshot taken. MyFile will tell you when the text changes.".into(),
        snapshot_taken_at: entry.snapshot.taken_at,
    };
    store.entries.push(entry);
    save(&app, &store)?;
    restart_watcher(&app);
    Ok(item)
}

/// Every monitored document with its current status (re-checked now).
#[tauri::command]
pub async fn monitor_list(app: AppHandle) -> Result<Vec<MonitorItem>, AppError> {
    let store = load(&app)?;
    crate::blocking(move || {
        Ok(store
            .entries
            .iter()
            .map(|e| {
                let (status, summary, _) = check(e);
                MonitorItem { id: e.id, name: e.name.clone(), status, summary, snapshot_taken_at: e.snapshot.taken_at }
            })
            .collect())
    })
    .await
}

/// Opens the changes for one document as a comparison (snapshot → now),
/// which the comparison view can show and explain.
#[tauri::command]
pub async fn monitor_changes(id: u64, app: AppHandle, state: State<'_, AppState>) -> Result<serde_json::Value, AppError> {
    let entry = load(&app)?.entries.into_iter().find(|e| e.id == id).ok_or_else(|| AppError::new("missing", "That document isn't monitored anymore."))?;
    let e = entry.clone();
    let (status, _, current) = crate::blocking(move || Ok(check(&e))).await?;
    let current = match (status, current) {
        (Status::TextChanged, Some(c)) => c,
        (Status::Missing | Status::Unreadable, _) => return Err(AppError::new("missing", "MyFile can't read the current version of that file.")),
        _ => return Err(AppError::new("no_changes", "The text hasn't changed since the snapshot.")),
    };
    let d = compare::diff(
        &compare::sentences(&entry.snapshot.pages, entry.snapshot.has_pages),
        &compare::sentences(&current.pages, current.has_pages),
    );
    let explain_chars: usize = d
        .changes
        .iter()
        .take(120)
        .map(|c| c.before.as_deref().unwrap_or("").chars().count() + c.after.as_deref().unwrap_or("").chars().count())
        .sum();
    let comparison_id = state.new_id();
    let old_name = format!("{} (snapshot)", entry.name);
    let new_name = format!("{} (now)", entry.name);
    *lock(&state.comparison) = Some(StoredComparison {
        id: comparison_id,
        selection_id: None,
        old_name: old_name.clone(),
        new_name: new_name.clone(),
        diff: d.clone(),
        consent: false,
    });
    Ok(serde_json::json!({
        "comparisonId": comparison_id,
        "oldName": old_name,
        "newName": new_name,
        "currentIsNew": true,
        "diff": d,
        "explainChars": explain_chars,
        "ocr": false,
    }))
}

/// Marks the current version as reviewed: it becomes the new snapshot.
#[tauri::command]
pub async fn monitor_accept(id: u64, app: AppHandle) -> Result<(), AppError> {
    let mut store = load(&app)?;
    let entry = store.entries.iter_mut().find(|e| e.id == id).ok_or_else(|| AppError::new("missing", "That document isn't monitored anymore."))?;
    let p = entry.path.clone();
    let (snapshot, _) = crate::blocking(move || snapshot_of(&p)).await?;
    let (size, modified) = file_stamp(&entry.path).unwrap_or((0, 0));
    entry.snapshot = snapshot;
    entry.size = size;
    entry.modified = modified;
    save(&app, &store)
}

/// Stops monitoring and deletes the stored snapshot.
#[tauri::command]
pub fn monitor_remove(id: u64, app: AppHandle) -> Result<(), AppError> {
    let mut store = load(&app)?;
    store.entries.retain(|e| e.id != id);
    save(&app, &store)?;
    restart_watcher(&app);
    Ok(())
}

// ---------------------------------------------------------------------------
// Live detection while MyFile is open
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct MonitorEvent {
    id: u64,
    name: String,
    status: Status,
    summary: String,
}

/// (Re)builds the file watcher for the current list. Called at startup and
/// whenever the list changes.
pub fn restart_watcher(app: &AppHandle) {
    let state = app.state::<AppState>();
    *lock(&state.monitor_watcher) = None;
    let Ok(store) = load(app) else { return };
    if store.entries.is_empty() {
        return;
    }
    let (tx, rx) = mpsc::unbounded_channel::<PathBuf>();
    let Ok(mut watcher) = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res {
            if matches!(ev.kind, EventKind::Create(_) | EventKind::Modify(_)) {
                for p in ev.paths {
                    let _ = tx.send(p);
                }
            }
        }
    }) else {
        return;
    };
    let mut dirs: Vec<PathBuf> = store.entries.iter().filter_map(|e| e.path.parent().map(Path::to_path_buf)).collect();
    dirs.sort();
    dirs.dedup();
    for d in &dirs {
        let _ = watcher.watch(d, RecursiveMode::NonRecursive);
    }
    *lock(&state.monitor_watcher) = Some(watcher);
    tauri::async_runtime::spawn(handle(app.clone(), rx));
}

async fn handle(app: AppHandle, mut rx: mpsc::UnboundedReceiver<PathBuf>) {
    let mut pending: HashMap<PathBuf, Instant> = HashMap::new();
    let mut tick = tokio::time::interval(Duration::from_millis(700));
    loop {
        tokio::select! {
            p = rx.recv() => match p {
                Some(p) => { pending.insert(p.canonicalize().unwrap_or(p), Instant::now()); }
                None => return,
            },
            _ = tick.tick() => {}
        }
        let ready: Vec<PathBuf> = pending.iter().filter(|(_, t)| t.elapsed() >= Duration::from_secs(2)).map(|(p, _)| p.clone()).collect();
        for path in ready {
            pending.remove(&path);
            let Ok(store) = load(&app) else { continue };
            let Some(entry) = store.entries.into_iter().find(|e| e.path == path) else { continue };
            let e = entry.clone();
            let Ok((status, summary, _)) = crate::blocking(move || Ok(check(&e))).await else { continue };
            if matches!(status, Status::Unchanged) {
                continue;
            }
            let _ = app.emit("monitor-event", MonitorEvent { id: entry.id, name: entry.name.clone(), status, summary: summary.clone() });
            if status == Status::TextChanged {
                let _ = app.notification().builder().title(format!("{} changed", entry.name)).body(&summary).show();
            }
        }
    }
}

/// At startup: tell the user about documents that changed while MyFile was closed.
pub fn check_on_startup(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        restart_watcher(&app);
        let Ok(store) = load(&app) else { return };
        for entry in store.entries {
            let e = entry.clone();
            let Ok((status, summary, _)) = crate::blocking(move || Ok(check(&e))).await else { continue };
            if status == Status::TextChanged {
                let _ = app.emit("monitor-event", MonitorEvent { id: entry.id, name: entry.name.clone(), status, summary: summary.clone() });
                let _ = app.notification().builder().title(format!("{} changed while MyFile was closed", entry.name)).body(&summary).show();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_ignores_rewrapping_but_not_edits() {
        let a = vec!["The deadline is\nNovember 4.".to_string()];
        let b = vec!["The deadline is November 4.".to_string()];
        let c = vec!["The deadline is November 8.".to_string()];
        assert_eq!(fingerprint(&a), fingerprint(&b));
        assert_ne!(fingerprint(&a), fingerprint(&c));
    }

    #[test]
    fn check_tells_metadata_only_from_real_text_changes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("notes.md");
        std::fs::write(&p, "Report due November 4, 2026.\nBring snacks.").unwrap();
        let (snapshot, _) = snapshot_of(&p).unwrap();
        let (size, modified) = file_stamp(&p).unwrap();
        let entry = Entry { id: 1, path: p.clone(), name: "notes.md".into(), snapshot, size, modified };
        assert_eq!(check(&entry).0, Status::Unchanged);
        // Same text, different bytes (extra trailing whitespace): metadata only.
        std::fs::write(&p, "Report due November 4, 2026.\nBring snacks.\n\n").unwrap();
        assert_eq!(check(&entry).0, Status::MetadataOnly);
        // Real edit: the summary names the moved date.
        std::fs::write(&p, "Report due November 8, 2026.\nBring snacks.").unwrap();
        let (status, summary, _) = check(&entry);
        assert_eq!(status, Status::TextChanged);
        assert!(summary.contains("Nov 4, 2026 → Nov 8, 2026"), "{summary}");
        std::fs::remove_file(&p).unwrap();
        assert_eq!(check(&entry).0, Status::Missing);
    }
}
