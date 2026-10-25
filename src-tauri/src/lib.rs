mod actions;
mod agent;
mod compare;
mod convert;
mod duplicates;
mod ai;
mod error;
mod evidence;
mod extract;
mod factcheck;
#[cfg(test)]
mod eval;
mod files;
mod folder;
mod ics;
mod image_ops;
mod local;
mod monitor;
mod ocr;
mod mcp;
mod pdf;
mod route;
mod search;
mod shift;
mod transcribe;
mod watch;

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tokio::sync::oneshot;

use actions::ActionInfo;
use ai::{AiAnswer, AiTask, ClaudeCli, DocumentText};
use error::AppError;
use files::{FileInfo, FileKind};
use image_ops::CompressOptions;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Text extracted from the current document, kept in memory only.
struct Document {
    pages: Vec<String>,
    has_pages: bool,
    /// The text came from OCR (a scan or a photo), so it may contain misreads.
    ocr: bool,
    /// For a transcribed recording: when each piece was said.
    segments: Option<Arc<Vec<transcribe::Segment>>>,
}

/// The file the user has chosen. Every operation acts on this path, so the
/// interface never asks the backend to read an arbitrary path of its choosing.
struct Selected {
    /// Changes with every selection, so slow work started for an old file
    /// can't attach its results to a newer one.
    id: u64,
    path: PathBuf,
    info: FileInfo,
    document: Option<Arc<Document>>,
    /// Set once the user agrees to send this file's text to the AI provider.
    ai_consent: bool,
    /// The finished transcript of a recording, so reopening the panel doesn't redo it.
    transcript: Option<transcribe::TranscriptResult>,
}

/// A generated result waiting to be saved. Held in memory, not in temp files,
/// so extracted text and AI answers never linger on disk.
struct Output {
    bytes: Vec<u8>,
    file_name: String,
    filter_label: &'static str,
    extension: &'static str,
}

#[derive(Default)]
struct AppState {
    selected: Mutex<Option<Selected>>,
    outputs: Mutex<HashMap<u64, Output>>,
    next_id: Mutex<u64>,
    ai_cancel: Mutex<Option<oneshot::Sender<()>>>,
    /// Cancels an in-flight local-model request.
    local_cancel: Mutex<Option<oneshot::Sender<()>>>,
    /// The open folder in "semester brain" mode, if any.
    folder: Mutex<Option<folder::FolderSession>>,
    /// Cancels every in-flight folder request.
    folder_cancel: Mutex<HashMap<u64, oneshot::Sender<()>>>,
    /// The latest validated agent plan, waiting for approval.
    agent_plan: Mutex<Option<agent::StoredPlan>>,
    /// The latest document comparison, kept so it can be explained.
    comparison: Mutex<Option<compare::StoredComparison>>,
    /// Watches monitored documents for changes while MyFile is open.
    monitor_watcher: Mutex<Option<notify::RecommendedWatcher>>,
    /// The watched folder, if any. Dropping it stops watching.
    watch: Mutex<Option<watch::WatchSession>>,
    /// Stops the running search-index update; Some while one runs.
    index_cancel: Mutex<Option<Arc<std::sync::atomic::AtomicBool>>>,
    /// The passages chosen for the latest "ask your files" question.
    search_ask: Mutex<Option<search::StoredAsk>>,
    /// Stops the running conversion.
    convert_cancel: Mutex<Option<Arc<std::sync::atomic::AtomicBool>>>,
    /// The latest conversion result, waiting to be saved.
    conversion: Mutex<Option<convert::Conversion>>,
    /// Cancels the running transcription or speech-model download.
    speech_cancel: Mutex<Option<Arc<std::sync::atomic::AtomicBool>>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl AppState {
    fn new_id(&self) -> u64 {
        let mut n = lock(&self.next_id);
        *n += 1;
        *n
    }

    /// The current file's id, path, and info.
    fn current(&self) -> Result<(u64, PathBuf, FileInfo), AppError> {
        lock(&self.selected)
            .as_ref()
            .map(|s| (s.id, s.path.clone(), s.info.clone()))
            .ok_or_else(|| AppError::new("no_file", "Choose a file first."))
    }

    fn store_output(&self, output: Output) -> u64 {
        let id = self.new_id();
        lock(&self.outputs).insert(id, output);
        id
    }
}

/// Fails unless `selection_id` is still the file the user has open.
fn require_current(state: &AppState, selection_id: u64) -> Result<(), AppError> {
    match lock(&state.selected).as_ref() {
        Some(s) if s.id == selection_id => Ok(()),
        Some(_) => Err(AppError::new("stale", "You switched files, so that request was cancelled.")),
        None => Err(AppError::new("no_file", "Choose a file first.")),
    }
}

fn require_kind(info: &FileInfo, kinds: &[FileKind], what: &str) -> Result<(), AppError> {
    if kinds.contains(&info.kind) {
        Ok(())
    } else {
        Err(AppError::new("wrong_kind", format!("{what} isn't available for this type of file.")))
    }
}

/// Runs CPU-heavy or blocking work off the async runtime so the UI stays responsive.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, AppError> + Send + 'static,
) -> Result<T, AppError> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|_| AppError::new("internal", "The operation stopped unexpectedly."))?
}

fn stem(name: &str) -> String {
    Path::new(name)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "output".to_string())
}

// ---------------------------------------------------------------------------
// File selection
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SelectedFile {
    /// Identifies this selection. Consent, AI requests, and routing must name
    /// it, so nothing meant for one file can act on another.
    selection_id: u64,
    #[serde(flatten)]
    info: FileInfo,
    actions: Vec<ActionInfo>,
}

/// Validates a file the user dropped or picked and makes it the current file.
#[tauri::command]
fn select_file(path: String, state: State<AppState>) -> Result<SelectedFile, AppError> {
    let path = PathBuf::from(path);
    let result = files::inspect(&path);
    let id = state.new_id();
    lock(&state.outputs).clear();
    // Opening a file leaves folder mode and ends any comparison.
    *lock(&state.folder) = None;
    *lock(&state.comparison) = None;
    // A new file makes any in-flight AI request for the old one pointless.
    for slot in [&state.ai_cancel, &state.local_cancel] {
        if let Some(tx) = lock(slot).take() {
            let _ = tx.send(());
        }
    }
    *lock(&state.conversion) = None;
    if let Some(flag) = lock(&state.convert_cancel).take() {
        flag.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    if let Some(flag) = lock(&state.speech_cancel).take() {
        flag.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    let mut selected = lock(&state.selected);
    match result {
        Ok(info) => {
            *selected = Some(Selected { id, path, info: info.clone(), document: None, ai_consent: false, transcript: None });
            let actions = actions::for_kind(info.kind, ocr::available());
            Ok(SelectedFile { selection_id: id, info, actions })
        }
        Err(e) => {
            *selected = None;
            Err(e)
        }
    }
}

#[tauri::command]
fn clear_file(state: State<AppState>) {
    *lock(&state.selected) = None;
    lock(&state.outputs).clear();
}

/// True if Shift is held right now. Called when a file is dropped.
#[tauri::command]
fn shift_held() -> bool {
    shift::shift_held()
}

// ---------------------------------------------------------------------------
// Text preview
// ---------------------------------------------------------------------------

const PREVIEW_BYTES: usize = 64 * 1024;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TextPreview {
    text: String,
    truncated: bool,
}

#[tauri::command]
async fn preview_text(state: State<'_, AppState>) -> Result<TextPreview, AppError> {
    let (_, path, info) = state.current()?;
    require_kind(&info, &[FileKind::Text], "Text preview")?;
    blocking(move || {
        let mut bytes = Vec::with_capacity(PREVIEW_BYTES + 1);
        File::open(&path)
            .and_then(|f| f.take(PREVIEW_BYTES as u64 + 1).read_to_end(&mut bytes))
            .map_err(AppError::from_read)?;
        let truncated = bytes.len() > PREVIEW_BYTES;
        bytes.truncate(PREVIEW_BYTES);
        // Trim a trailing partial character rather than showing a replacement glyph.
        let text = match std::str::from_utf8(&bytes) {
            Ok(s) => s.to_string(),
            Err(e) if e.error_len().is_none() => {
                String::from_utf8_lossy(&bytes[..e.valid_up_to()]).into_owned()
            }
            Err(_) => String::from_utf8_lossy(&bytes).into_owned(),
        };
        Ok(TextPreview { text, truncated })
    })
    .await
}

// ---------------------------------------------------------------------------
// Documents (PDF extraction, and the text the AI reads)
// ---------------------------------------------------------------------------

fn load_document(path: &Path, kind: FileKind) -> Result<Document, AppError> {
    match kind {
        FileKind::Pdf => match pdf::extract(path) {
            Ok(text) => Ok(Document { pages: text.pages, has_pages: true, ocr: false, segments: None }),
            // A scan has no text layer: read the page images with OCR instead.
            Err(e) if e.code == "no_text" => {
                let pages = ocr::pdf_text(path)?;
                if pages.iter().all(|p| p.trim().is_empty()) {
                    return Err(AppError::new("no_text", "OCR couldn't find readable text in this scanned PDF."));
                }
                Ok(Document { pages, has_pages: true, ocr: true, segments: None })
            }
            Err(e) => Err(e),
        },
        FileKind::Image => {
            let text = ocr::image_text(&image_ops::load(path)?)?;
            if text.trim().is_empty() {
                return Err(AppError::new("no_text", "OCR didn't find any readable text in this image."));
            }
            Ok(Document { pages: vec![text], has_pages: false, ocr: true, segments: None })
        }
        FileKind::Text => {
            let bytes = std::fs::read(path).map_err(AppError::from_read)?;
            let text = String::from_utf8_lossy(&bytes);
            let text = text.strip_prefix('\u{FEFF}').unwrap_or(&text).to_string();
            Ok(Document { pages: vec![text], has_pages: false, ocr: false, segments: None })
        }
        // Transcribing is slow, so it only happens when asked for (transcribe.rs).
        FileKind::Audio | FileKind::Video => Err(AppError::new("needs_transcript", "Transcribe this recording first, then the AI can read it.")),
        FileKind::Unsupported => Err(AppError::new("wrong_kind", "MyFile can only read text from PDFs, text files, and images.")),
    }
}

/// Returns the current file's text, extracting it once and caching it in memory.
async fn ensure_document(state: &AppState) -> Result<(u64, FileInfo, Arc<Document>), AppError> {
    let cached = {
        let guard = lock(&state.selected);
        let s = guard.as_ref().ok_or_else(|| AppError::new("no_file", "Choose a file first."))?;
        (s.id, s.path.clone(), s.info.clone(), s.document.clone())
    };
    let (id, path, info, document) = cached;
    if let Some(doc) = document {
        return Ok((id, info, doc));
    }
    let kind = info.kind;
    let doc = Arc::new(blocking(move || load_document(&path, kind)).await?);
    if let Some(s) = lock(&state.selected).as_mut().filter(|s| s.id == id) {
        s.document = Some(doc.clone());
    }
    Ok((id, info, doc))
}

/// Text sent to the interface for display. The saved file always has everything.
const MAX_DISPLAY_CHARS: usize = 400_000;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExtractedText {
    text: String,
    truncated_for_display: bool,
    page_count: usize,
    empty_pages: usize,
    char_count: usize,
    output_id: u64,
    /// Read with OCR, so it may contain misreads.
    ocr: bool,
}

#[tauri::command]
async fn extract_pdf_text(state: State<'_, AppState>) -> Result<ExtractedText, AppError> {
    let (_, _, info) = state.current()?;
    require_kind(&info, &[FileKind::Pdf, FileKind::Image], "Text extraction")?;
    let (_, info, doc) = ensure_document(&state).await?;

    let text = pdf::PdfText { pages: doc.pages.clone(), page_count: doc.pages.len() };
    let joined = text.joined();
    let char_count = text.char_count();
    let empty_pages = doc.pages.iter().filter(|p| p.trim().is_empty()).count();
    let truncated_for_display = joined.chars().count() > MAX_DISPLAY_CHARS;
    let shown: String = if truncated_for_display { joined.chars().take(MAX_DISPLAY_CHARS).collect() } else { joined.clone() };

    let output_id = state.store_output(Output {
        bytes: joined.into_bytes(),
        file_name: format!("{}.txt", stem(&info.name)),
        filter_label: "Text file",
        extension: "txt",
    });
    Ok(ExtractedText {
        text: shown,
        truncated_for_display,
        page_count: text.page_count,
        empty_pages,
        char_count,
        output_id,
        ocr: doc.ocr,
    })
}

// ---------------------------------------------------------------------------
// Images
// ---------------------------------------------------------------------------

#[tauri::command]
async fn image_thumbnail(state: State<'_, AppState>) -> Result<String, AppError> {
    let (_, path, info) = state.current()?;
    require_kind(&info, &[FileKind::Image], "Image preview")?;
    blocking(move || image_ops::thumbnail_data_url(&image_ops::load(&path)?, 480)).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CompressResult {
    output_id: u64,
    file_name: String,
    original_bytes: u64,
    output_bytes: u64,
    original_width: u32,
    original_height: u32,
    width: u32,
    height: u32,
    preview_data_url: String,
}

#[tauri::command]
async fn compress_image(
    options: CompressOptions,
    state: State<'_, AppState>,
) -> Result<CompressResult, AppError> {
    let (id, path, info) = state.current()?;
    require_kind(&info, &[FileKind::Image], "Image compression")?;
    let opts = options.clone();
    let out = blocking(move || image_ops::compress(&path, &opts)).await?;

    if state.current().ok().map(|(cur, _, _)| cur) != Some(id) {
        return Err(AppError::new("stale", "The file changed while this was running."));
    }
    let file_name = format!("{}-compressed.{}", stem(&info.name), options.format.extension());
    let output_bytes = out.bytes.len() as u64;
    let output_id = state.store_output(Output {
        bytes: out.bytes,
        file_name: file_name.clone(),
        filter_label: if options.format == image_ops::OutputFormat::Jpeg { "JPEG image" } else { "PNG image" },
        extension: options.format.extension(),
    });
    Ok(CompressResult {
        output_id,
        file_name,
        original_bytes: info.size_bytes,
        output_bytes,
        original_width: out.original_width,
        original_height: out.original_height,
        width: out.width,
        height: out.height,
        preview_data_url: out.preview_data_url,
    })
}

// ---------------------------------------------------------------------------
// AI document assistant
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AiStatus {
    available: bool,
    provider: &'static str,
    model: Option<String>,
    /// Characters of the current document that would be sent, once known.
    max_chars: usize,
}

#[tauri::command]
fn ai_status(app: AppHandle) -> AiStatus {
    let cli = ClaudeCli::locate(app.path().home_dir().ok().as_deref());
    AiStatus {
        available: cli.is_some(),
        provider: "Claude Code CLI",
        model: cli.map(|c| c.model),
        max_chars: ai::MAX_DOCUMENT_CHARS,
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DocumentStats {
    chars: usize,
    /// Page count for PDFs; None for text files.
    pages: Option<usize>,
    ocr: bool,
    /// The text is a machine transcript of a recording.
    transcript: bool,
}

/// Extracts the current document locally (if not done yet) and reports how
/// much text there is, so the consent prompt can say exactly what would be sent.
#[tauri::command]
async fn document_stats(selection_id: u64, state: State<'_, AppState>) -> Result<DocumentStats, AppError> {
    require_current(&state, selection_id)?;
    let (_, _, info) = state.current()?;
    require_kind(&info, &[FileKind::Pdf, FileKind::Text, FileKind::Image, FileKind::Audio, FileKind::Video], "The AI assistant")?;
    let (id, _, doc) = ensure_document(&state).await?;
    if id != selection_id {
        return Err(AppError::new("stale", "You switched files, so that request was cancelled."));
    }
    Ok(DocumentStats {
        chars: doc.pages.iter().map(|p| p.chars().count()).sum(),
        pages: doc.has_pages.then_some(doc.pages.len()),
        ocr: doc.ocr,
        transcript: doc.segments.is_some(),
    })
}

/// Records that the user agreed to send the current file's text to the AI.
#[tauri::command]
fn grant_ai_consent(selection_id: u64, state: State<AppState>) -> Result<(), AppError> {
    let mut guard = lock(&state.selected);
    let s = guard.as_mut().ok_or_else(|| AppError::new("no_file", "Choose a file first."))?;
    if s.id != selection_id {
        return Err(AppError::new("stale", "You switched files, so that permission wasn't applied."));
    }
    s.ai_consent = true;
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AiResult {
    #[serde(flatten)]
    answer: AiAnswer,
    /// For recordings: when each item's quote was said (ms), by item index.
    item_times: Option<Vec<Option<u64>>>,
    output_id: u64,
    /// A ready-to-save .ics file for deadline answers with checked dates.
    calendar: Option<CalendarExport>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CalendarExport {
    output_id: u64,
    events: usize,
    /// Dated items left out because their exact calendar date wasn't certain.
    skipped: usize,
}

#[tauri::command]
async fn run_ai(
    selection_id: u64,
    task: AiTask,
    question: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AiResult, AppError> {
    let (_, _, info) = state.current()?;
    require_kind(&info, &[FileKind::Pdf, FileKind::Text, FileKind::Image, FileKind::Audio, FileKind::Video], "The AI assistant")?;

    // Ordinary code enforces consent, for this exact selection; the
    // interface asking isn't enough. Checked again right before sending.
    let has_consent = |state: &AppState| {
        lock(&state.selected).as_ref().is_some_and(|s| s.id == selection_id && s.ai_consent)
    };
    require_current(&state, selection_id)?;
    if !has_consent(&state) {
        return Err(AppError::new("ai_consent", "MyFile needs your permission before sending this document to the AI."));
    }

    let question = check_question(task, question)?;

    let cli = ClaudeCli::locate(app.path().home_dir().ok().as_deref()).ok_or_else(|| {
        AppError::new(
            "ai_unavailable",
            "The AI assistant needs the Claude Code CLI, and MyFile couldn't find it on this computer. \
             Install Claude Code and sign in, then try again.",
        )
    })?;

    let (id, info, doc) = prepared_document(&state, selection_id).await?;
    let doc_text = DocumentText { name: &info.name, pages: &doc.pages, has_pages: doc.has_pages };

    let (cancel_tx, cancel_rx) = oneshot::channel();
    {
        let mut slot = lock(&state.ai_cancel);
        if slot.as_ref().is_some_and(|tx| !tx.is_closed()) {
            return Err(AppError::new("ai_busy", "The AI is already working on a request."));
        }
        *slot = Some(cancel_tx);
    }

    // Last check before any document text leaves the computer.
    if !has_consent(&state) {
        *lock(&state.ai_cancel) = None;
        return Err(AppError::new("stale", "You switched files, so nothing was sent."));
    }
    let started = Instant::now();
    let prompt = ai::build_prompt(task, question.as_deref(), &doc_text);
    let reply = cli.run(prompt, cancel_rx).await;
    *lock(&state.ai_cancel) = None;
    let answer = ai::validate(task, reply?, &doc_text, started)?;

    if state.current().ok().map(|(cur, _, _)| cur) != Some(id) {
        return Err(AppError::new("stale", "The file changed while the AI was working."));
    }
    Ok(finish_answer(&state, &info, &doc, task, answer))
}

/// Checks a question is present (for the Question task) and not too long.
fn check_question(task: AiTask, question: Option<String>) -> Result<Option<String>, AppError> {
    let question = question.map(|q| q.trim().to_string()).filter(|q| !q.is_empty());
    if task == AiTask::Question {
        match &question {
            None => return Err(AppError::new("bad_question", "Type a question first.")),
            Some(q) if q.chars().count() > ai::MAX_QUESTION_CHARS => {
                return Err(AppError::new(
                    "bad_question",
                    format!("Questions can be up to {} characters.", ai::MAX_QUESTION_CHARS),
                ))
            }
            _ => {}
        }
    }
    Ok(question)
}

/// The current document's text, for this exact selection, checked for being
/// non-empty and within the size the assistant handles.
async fn prepared_document(state: &AppState, selection_id: u64) -> Result<(u64, FileInfo, Arc<Document>), AppError> {
    let (id, info, doc) = ensure_document(state).await?;
    if id != selection_id {
        return Err(AppError::new("stale", "You switched files, so that request was cancelled."));
    }
    let chars: usize = doc.pages.iter().map(|p| p.chars().count()).sum();
    if doc.pages.iter().all(|p| p.trim().is_empty()) {
        return Err(AppError::new("no_text", "This document has no text for the AI to read."));
    }
    if chars > ai::MAX_DOCUMENT_CHARS {
        return Err(AppError::new(
            "too_long",
            format!(
                "This document has about {} characters of text. The AI assistant handles up to {}.",
                chars,
                ai::MAX_DOCUMENT_CHARS
            ),
        ));
    }
    Ok((id, info, doc))
}

/// Stores the answer (and, for deadlines, a calendar) for Copy/Save.
fn finish_answer(state: &AppState, info: &FileInfo, doc: &Document, task: AiTask, answer: AiAnswer) -> AiResult {
    let output_id = state.store_output(Output {
        bytes: ai::to_plain_text(&info.name, &answer).into_bytes(),
        file_name: format!("{}-ai-{}.txt", stem(&info.name), task_slug(task)),
        filter_label: "Text file",
        extension: "txt",
    });
    let calendar = (task == AiTask::Deadlines && ics::event_count(&answer.items) > 0).then(|| {
        let events = ics::event_count(&answer.items);
        let skipped = answer.items.iter().filter(|i| i.date.is_some() && i.iso_date.is_none()).count();
        let output_id = state.store_output(Output {
            bytes: ics::build(&info.name, &answer.items, std::time::SystemTime::now()).into_bytes(),
            file_name: format!("{}-deadlines.ics", stem(&info.name)),
            filter_label: "Calendar file",
            extension: "ics",
        });
        CalendarExport { output_id, events, skipped }
    });
    let item_times = doc.segments.as_ref().map(|segs| {
        answer
            .items
            .iter()
            .map(|i| match (&i.quote, i.quote_check) {
                (Some(q), Some(ai::QuoteCheck::Verified)) => transcribe::locate(q, segs),
                _ => None,
            })
            .collect()
    });
    AiResult { answer, item_times, output_id, calendar }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LocalStatus {
    available: bool,
    model: String,
    message: Option<String>,
}

/// Whether a local model (Ollama) is ready to use.
#[tauri::command]
async fn local_status() -> LocalStatus {
    let client = local::OllamaClient::from_env();
    let model = client.model.clone();
    match blocking(move || client.status()).await {
        Ok(()) => LocalStatus { available: true, model, message: None },
        Err(e) => LocalStatus { available: false, model, message: Some(e.message) },
    }
}

/// Runs the same document task on the local model. No consent is needed:
/// the text goes to Ollama on 127.0.0.1 and never leaves this computer.
/// Validation is identical to the Claude path.
#[tauri::command]
async fn run_ai_local(
    selection_id: u64,
    task: AiTask,
    question: Option<String>,
    state: State<'_, AppState>,
) -> Result<AiResult, AppError> {
    let (_, _, info) = state.current()?;
    require_kind(&info, &[FileKind::Pdf, FileKind::Text, FileKind::Image, FileKind::Audio, FileKind::Video], "The AI assistant")?;
    require_current(&state, selection_id)?;
    let question = check_question(task, question)?;
    let client = local::OllamaClient::from_env();
    {
        let c = client.clone();
        blocking(move || c.status()).await?;
    }
    let (id, info, doc) = prepared_document(&state, selection_id).await?;
    let doc_text = DocumentText { name: &info.name, pages: &doc.pages, has_pages: doc.has_pages };

    let (cancel_tx, cancel_rx) = oneshot::channel();
    {
        let mut slot = lock(&state.local_cancel);
        if slot.as_ref().is_some_and(|tx| !tx.is_closed()) {
            return Err(AppError::new("ai_busy", "The local model is already working on a request."));
        }
        *slot = Some(cancel_tx);
    }
    let started = Instant::now();
    let prompt = ai::build_prompt(task, question.as_deref(), &doc_text);
    let reply = client.run_with(ai::system_prompt(), ai::response_schema(), prompt, cancel_rx).await;
    *lock(&state.local_cancel) = None;
    let answer = ai::validate(task, reply?, &doc_text, started)?;
    if state.current().ok().map(|(cur, _, _)| cur) != Some(id) {
        return Err(AppError::new("stale", "The file changed while the local model was working."));
    }
    Ok(finish_answer(&state, &info, &doc, task, answer))
}

#[tauri::command]
fn cancel_local(state: State<AppState>) {
    if let Some(tx) = lock(&state.local_cancel).take() {
        let _ = tx.send(());
    }
}

fn task_slug(task: AiTask) -> &'static str {
    match task {
        AiTask::Summarize => "summary",
        AiTask::Deadlines => "deadlines",
        AiTask::Requirements => "requirements",
        AiTask::Decisions => "decisions",
        AiTask::Simplify => "explained",
        AiTask::Question => "answer",
    }
}

/// Turns a typed request into one allowlisted action for the current file.
/// Only the request text and the file's type are sent, never its contents.
#[tauri::command]
async fn route_request(
    selection_id: u64,
    request: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<route::RouteDecision, AppError> {
    require_current(&state, selection_id)?;
    let (_, _, info) = state.current()?;
    let request = request.trim().to_string();
    if request.is_empty() {
        return Err(AppError::new("bad_request", "Type what you'd like to do with this file."));
    }
    if request.chars().count() > route::MAX_REQUEST_CHARS {
        return Err(AppError::new(
            "bad_request",
            format!("Keep requests under {} characters.", route::MAX_REQUEST_CHARS),
        ));
    }
    if info.kind == FileKind::Unsupported {
        return Err(AppError::new("wrong_kind", "MyFile has no actions for this type of file yet."));
    }

    let mut cli = ClaudeCli::locate(app.path().home_dir().ok().as_deref()).ok_or_else(|| {
        AppError::new(
            "ai_unavailable",
            "Understanding typed requests needs the Claude Code CLI, which MyFile couldn't find. \
             The action buttons still work.",
        )
    })?;
    cli.model = std::env::var("MYFILE_ROUTER_MODEL")
        .ok()
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| route::DEFAULT_ROUTER_MODEL.to_string());

    let schema = route::schema(info.kind);
    let prompt = route::build_prompt(info.kind, &info.type_label, &request);
    let (_keep_open, cancel) = oneshot::channel();
    let reply = cli.run_with(route::system_prompt(), &schema, prompt, cancel).await?;
    // The user may have opened another file while the model was choosing.
    require_current(&state, selection_id)?;
    route::validate(info.kind, reply)
}

#[tauri::command]
fn cancel_ai(state: State<AppState>) {
    if let Some(tx) = lock(&state.ai_cancel).take() {
        let _ = tx.send(());
    }
}

// ---------------------------------------------------------------------------
// Saving
// ---------------------------------------------------------------------------

/// Copies a generated result to the clipboard-friendly text, if it is text.
#[tauri::command]
fn output_text(output_id: u64, state: State<AppState>) -> Result<String, AppError> {
    let outputs = lock(&state.outputs);
    let out = outputs.get(&output_id).ok_or_else(missing_output)?;
    String::from_utf8(out.bytes.clone()).map_err(|_| AppError::new("not_text", "This result isn't text."))
}

fn missing_output() -> AppError {
    AppError::new("no_output", "That result is no longer available. Run the action again.")
}

/// Asks where to save a result (with a native dialog opened from Rust, so the
/// interface never supplies a write path) and writes it there.
#[tauri::command]
async fn save_output(
    output_id: u64,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<String>, AppError> {
    let (bytes, file_name, filter_label, extension) = {
        let outputs = lock(&state.outputs);
        let o = outputs.get(&output_id).ok_or_else(missing_output)?;
        (o.bytes.clone(), o.file_name.clone(), o.filter_label, o.extension)
    };
    let original = state.current().ok().map(|(_, p, _)| p);
    save_bytes(&app, bytes, &file_name, filter_label, extension, original).await
}

/// Shows a native save dialog (opened from Rust, so the interface never
/// supplies a write path), refuses to overwrite `original`, and writes the
/// bytes atomically. Returns the saved file name, or None if cancelled.
pub(crate) async fn save_bytes(
    app: &AppHandle,
    bytes: Vec<u8>,
    file_name: &str,
    filter_label: &str,
    extension: &str,
    original: Option<PathBuf>,
) -> Result<Option<String>, AppError> {
    let (tx, rx) = oneshot::channel();
    let mut dialog = app
        .dialog()
        .file()
        .set_title("Save result")
        .set_file_name(file_name)
        .add_filter(filter_label, &[extension]);
    if let Some(dir) = original.as_ref().and_then(|p| p.parent()) {
        dialog = dialog.set_directory(dir);
    }
    // The OS dialog itself asks for confirmation before replacing an existing file.
    dialog.save_file(move |picked| {
        let _ = tx.send(picked);
    });
    let Some(picked) = rx.await.ok().flatten() else {
        return Ok(None); // the user cancelled
    };
    let dest = picked
        .into_path()
        .map_err(|_| AppError::new("bad_destination", "That location can't be saved to."))?;

    if let Some(orig) = &original {
        if same_file(orig, &dest) {
            return Err(AppError::new(
                "would_overwrite_original",
                "That's the original file. Choose a different name so your original stays unchanged.",
            ));
        }
    }

    let saved_name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    blocking(move || write_atomically(&dest, &bytes)).await?;
    Ok(Some(saved_name))
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Writes to a temporary sibling, then renames it over the destination, so a
/// failure part-way never leaves a half-written file under the chosen name.
fn write_atomically(dest: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = dest.with_file_name(format!(".{name}.myfile-partial"));
    let result = std::fs::write(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, dest));
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(AppError::new("save_failed", format!("MyFile couldn't save the file ({e}).")));
    }
    Ok(())
}

/// Entry point for the `myfile-mcp` binary: serves MyFile's tools over MCP (stdio).
pub fn run_mcp(root: PathBuf) -> std::io::Result<()> {
    mcp::run_stdio(root)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .manage(AppState::default())
        .setup(|app| {
            // Report documents that changed while MyFile was closed, then keep watching.
            monitor::check_on_startup(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            select_file,
            clear_file,
            shift_held,
            preview_text,
            extract_pdf_text,
            image_thumbnail,
            compress_image,
            ai_status,
            document_stats,
            grant_ai_consent,
            run_ai,
            cancel_ai,
            route_request,
            local_status,
            run_ai_local,
            cancel_local,
            folder::open_folder,
            folder::close_folder,
            folder::grant_folder_consent,
            folder::cancel_folder,
            folder::analyze_folder,
            folder::ask_folder,
            agent::plan_agent,
            agent::run_agent,
            agent::discard_plan,
            duplicates::find_duplicates,
            convert::conversion_choices,
            convert::convert_file,
            convert::cancel_convert,
            convert::save_conversion,
            convert::suggest_conversion,
            evidence::evidence_current,
            evidence::evidence_folder,
            evidence::evidence_indexed,
            evidence::evidence_passage,
            search::search_status,
            search::add_search_folder,
            search::remove_search_folder,
            search::reindex,
            search::stop_indexing,
            search::delete_search_index,
            search::enable_meaning_search,
            search::search_files,
            search::prepare_ask,
            search::ask_files,
            transcribe::speech_status,
            transcribe::download_speech_model,
            transcribe::transcribe,
            transcribe::cancel_speech,
            transcribe::current_transcript,
            extract::list_templates,
            extract::save_template,
            extract::delete_template,
            extract::extract_file,
            extract::extract_folder,
            factcheck::fact_check_file,
            factcheck::fact_check_folder,
            compare::compare_with,
            compare::grant_comparison_consent,
            compare::explain_comparison,
            monitor::monitor_add,
            monitor::monitor_list,
            monitor::monitor_changes,
            monitor::monitor_accept,
            monitor::monitor_remove,
            watch::start_watch,
            watch::stop_watch,
            watch::save_watch_calendar,
            output_text,
            save_output,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out.txt");
        std::fs::write(&dest, b"old").unwrap();
        write_atomically(&dest, b"new").unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"new");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn same_file_detects_the_original() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        std::fs::write(&a, b"x").unwrap();
        assert!(same_file(&a, &dir.path().join(".").join("a.txt")));
        assert!(!same_file(&a, &dir.path().join("b.txt")));
    }

    #[test]
    fn sample_pdf_extracts_two_pages() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../test-files/sample-syllabus.pdf");
        let doc = load_document(&path, FileKind::Pdf).unwrap();
        assert_eq!(doc.pages.len(), 2);
        assert!(doc.pages[0].contains("October 25, 2026"));
        assert!(doc.pages[1].contains("December 10, 2026"));
    }

    /// Calls the real Claude Code CLI (no mocks). Skipped by default because
    /// it needs the CLI installed and signed in. Run with:
    /// cargo test live_ -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn live_ai_finds_deadlines_in_sample_pdf() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../test-files/sample-syllabus.pdf");
        let doc = load_document(&path, FileKind::Pdf).unwrap();
        let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from);
        let cli = ClaudeCli::locate(home.as_deref()).expect("Claude Code CLI not found");
        let text = DocumentText { name: "sample-syllabus.pdf", pages: &doc.pages, has_pages: true };
        let (_keep_open, cancel) = oneshot::channel();
        let started = Instant::now();
        let reply = cli.run(ai::build_prompt(AiTask::Deadlines, None, &text), cancel).await.unwrap();
        let answer = ai::validate(AiTask::Deadlines, reply, &text, started).unwrap();
        println!("{}\n({} ms)", ai::to_plain_text("sample-syllabus.pdf", &answer), answer.duration_ms);
        assert!(answer.found);
        assert!(answer.items.iter().any(|i| i.date.as_deref().is_some_and(|d| d.contains("25"))));
    }

    #[test]
    fn requests_for_an_old_selection_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, b"hello").unwrap();
        let state = AppState::default();
        assert_eq!(require_current(&state, 1).unwrap_err().code, "no_file");
        let info = files::inspect(&path).unwrap();
        *lock(&state.selected) = Some(Selected { id: 7, path, info, document: None, ai_consent: true, transcript: None });
        assert!(require_current(&state, 7).is_ok());
        assert_eq!(require_current(&state, 6).unwrap_err().code, "stale");
    }

    #[test]
    fn stem_handles_odd_names() {
        assert_eq!(stem("report.final.pdf"), "report.final");
        assert_eq!(stem(""), "output");
    }
}
