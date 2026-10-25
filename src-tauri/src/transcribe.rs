//! Speech to text for audio and video files, entirely on this computer.
//!
//! - Decoding: Symphonia (pure Rust: WAV, MP3, AAC/M4A, MP4/MOV audio, FLAC,
//!   Ogg Vorbis, ALAC). Anything else goes to ffmpeg if it's installed.
//! - Recognition: whisper.cpp (through whisper-rs) with the Whisper base
//!   English model, downloaded once with permission and checked by SHA-256.
//!
//! Audio is streamed: it's downmixed, resampled to 16 kHz, and transcribed in
//! chunks of about five minutes, cut at the quietest moment so words aren't
//! split. Memory stays flat however long the recording is.

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Once};
use std::time::Instant;

use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager, State};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState};

use crate::actions::{self, ActionInfo};
use crate::error::AppError;
use crate::files::FileKind;
use crate::{lock, AppState, Document, Output};

const MODEL_FILE: &str = "ggml-base.en.bin";
const MODEL_LABEL: &str = "Whisper base (English)";
const MODEL_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin";
const MODEL_SHA256: &str = "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002";
const MODEL_BYTES: u64 = 147_964_211;

pub const RATE: usize = 16_000;
/// Transcribe in pieces of about this long.
const CHUNK_SECS: usize = 300;
/// Look this far back from a chunk's end for a quiet place to cut.
const CUT_SEARCH_SECS: usize = 10;
/// Recordings longer than this are refused.
const MAX_SECS: usize = 4 * 3600;
/// A pause this long starts a new paragraph in the transcript text.
const PARAGRAPH_GAP_MS: u64 = 2_000;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Segment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

// ---------------------------------------------------------------------------
// Resampling and chunking (pure code, unit tested)
// ---------------------------------------------------------------------------

/// Streaming mono resampler to 16 kHz. Downsampling averages each output
/// sample's span of input (a simple anti-aliasing filter); upsampling
/// interpolates linearly.
pub struct Resampler {
    /// Input samples per output sample.
    step: f64,
    down: bool,
    // Downsampling state.
    edge: f64,
    acc: f32,
    n: u32,
    // Shared: index of the next input sample.
    i: u64,
    // Upsampling state.
    t: f64,
    prev: f32,
}

impl Resampler {
    pub fn new(source_rate: u32) -> Self {
        let step = source_rate as f64 / RATE as f64;
        Resampler { step, down: step > 1.0, edge: step, acc: 0.0, n: 0, i: 0, t: 0.0, prev: 0.0 }
    }

    pub fn push(&mut self, input: &[f32], out: &mut Vec<f32>) {
        for &x in input {
            if self.down {
                self.acc += x;
                self.n += 1;
                self.i += 1;
                if self.i as f64 >= self.edge {
                    out.push(self.acc / self.n as f32);
                    self.acc = 0.0;
                    self.n = 0;
                    self.edge += self.step;
                }
            } else {
                let i = self.i as f64;
                if self.i == 0 {
                    self.prev = x;
                }
                while self.t <= i {
                    let frac = (self.t - (i - 1.0)).clamp(0.0, 1.0) as f32;
                    out.push(self.prev + (x - self.prev) * frac);
                    self.t += self.step;
                }
                self.prev = x;
                self.i += 1;
            }
        }
    }
}

/// Where to end a chunk: the middle of the quietest 100 ms window in the
/// last few seconds, so a cut is unlikely to land inside a word.
pub fn quiet_cut(samples: &[f32], chunk: usize, search: usize) -> usize {
    let win = RATE / 10;
    let end = chunk.min(samples.len());
    let start = end.saturating_sub(search);
    let mut best = (f32::MAX, end);
    let mut w = start;
    while w + win <= end {
        let energy: f32 = samples[w..w + win].iter().map(|s| s * s).sum();
        if energy < best.0 {
            best = (energy, w + win / 2);
        }
        w += win;
    }
    best.1
}

/// Collects 16 kHz samples and hands out chunks cut at quiet moments.
pub struct Chunker {
    buf: Vec<f32>,
    /// Samples already handed out.
    pub offset: usize,
    chunk: usize,
    search: usize,
}

impl Chunker {
    pub fn new(chunk_secs: usize, search_secs: usize) -> Self {
        Chunker { buf: Vec::new(), offset: 0, chunk: chunk_secs * RATE, search: search_secs * RATE }
    }

    /// Adds samples; returns any full chunks as (offset in samples, samples).
    pub fn push(&mut self, samples: &[f32]) -> Vec<(usize, Vec<f32>)> {
        self.buf.extend_from_slice(samples);
        let mut out = Vec::new();
        while self.buf.len() >= self.chunk {
            let cut = quiet_cut(&self.buf, self.chunk, self.search).max(1);
            let rest = self.buf.split_off(cut);
            out.push((self.offset, std::mem::replace(&mut self.buf, rest)));
            self.offset += cut;
        }
        out
    }

    pub fn finish(&mut self) -> Option<(usize, Vec<f32>)> {
        if self.buf.is_empty() {
            return None;
        }
        let b = std::mem::take(&mut self.buf);
        let at = self.offset;
        self.offset += b.len();
        Some((at, b))
    }
}

// ---------------------------------------------------------------------------
// Transcript text formats
// ---------------------------------------------------------------------------

/// Whisper marks non-speech like "[BLANK_AUDIO]" or "(music)"; those aren't words.
pub fn is_non_speech(text: &str) -> bool {
    let t = text.trim();
    t.is_empty()
        || ((t.starts_with('[') && t.ends_with(']')) || (t.starts_with('(') && t.ends_with(')')) || (t.starts_with('*') && t.ends_with('*')))
}

/// whisper.cpp sometimes joins sentences inside a segment without a space
/// ("start.The midterm"). Adds one after . ! ? between a lowercase letter or
/// digit and a capital letter; abbreviations like "U.S.A." are left alone.
pub fn fix_spacing(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len() + 8);
    for (i, &c) in chars.iter().enumerate() {
        out.push(c);
        if matches!(c, '.' | '!' | '?')
            && i > 0
            && (chars[i - 1].is_lowercase() || chars[i - 1].is_ascii_digit())
            && chars.get(i + 1).is_some_and(|n| n.is_uppercase())
        {
            out.push(' ');
        }
    }
    out
}

/// The text the AI reads: plain paragraphs, split at long pauses.
pub fn plain_text(segments: &[Segment]) -> String {
    let mut out = String::new();
    let mut last_end = None;
    for s in segments {
        if let Some(end) = last_end {
            out.push_str(if s.start_ms.saturating_sub(end) >= PARAGRAPH_GAP_MS { "\n\n" } else { " " });
        }
        out.push_str(s.text.trim());
        last_end = Some(s.end_ms);
    }
    out
}

pub fn clock(ms: u64) -> String {
    let s = ms / 1000;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

fn subtitle_time(ms: u64, sep: char) -> String {
    format!("{:02}:{:02}:{:02}{sep}{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
}

pub fn timestamped(name: &str, segments: &[Segment]) -> String {
    let mut out = format!("Transcript of {name}\n(Machine transcription by {MODEL_LABEL} on this computer. Check names and numbers against the recording.)\n\n");
    for s in segments {
        out.push_str(&format!("[{}] {}\n", clock(s.start_ms), s.text.trim()));
    }
    out
}

pub fn srt(segments: &[Segment]) -> String {
    segments
        .iter()
        .enumerate()
        .map(|(i, s)| format!("{}\n{} --> {}\n{}\n", i + 1, subtitle_time(s.start_ms, ','), subtitle_time(s.end_ms, ','), s.text.trim()))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn vtt(segments: &[Segment]) -> String {
    let mut out = String::from("WEBVTT\n\n");
    for s in segments {
        out.push_str(&format!("{} --> {}\n{}\n\n", subtitle_time(s.start_ms, '.'), subtitle_time(s.end_ms, '.'), s.text.trim()));
    }
    out
}

/// When in the recording a (verified) quote was said: the start of the
/// segment where it begins.
pub fn locate(quote: &str, segments: &[Segment]) -> Option<u64> {
    let needle = crate::ai::normalize(quote);
    if needle.is_empty() {
        return None;
    }
    let mut hay = String::new();
    let mut starts = Vec::with_capacity(segments.len());
    for s in segments {
        if !hay.is_empty() {
            hay.push(' ');
        }
        starts.push((hay.len(), s.start_ms));
        hay.push_str(&crate::ai::normalize(&s.text));
    }
    let at = hay.find(&needle)?;
    starts.iter().rev().find(|(pos, _)| *pos <= at).map(|(_, ms)| *ms)
}

// ---------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Decoder {
    /// Symphonia, compiled into MyFile.
    Builtin,
    Ffmpeg,
}

fn cancelled() -> AppError {
    AppError::new("cancelled", "Transcription cancelled.")
}

/// Decodes the first audio track to mono f32 at its own sample rate,
/// passing (rate, samples) to `sink`. Returns the track's length in seconds
/// when the file says.
fn decode_builtin(
    path: &Path,
    cancel: &AtomicBool,
    started: &mut bool,
    sink: &mut dyn FnMut(u32, &[f32]) -> Result<(), AppError>,
    on_duration: &mut dyn FnMut(Option<f64>),
) -> Result<(), AppError> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
    use symphonia::core::errors::Error;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let unsupported = |why: String| AppError::new("unsupported_audio", why);
    let file = File::open(path).map_err(AppError::from_read)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| unsupported(format!("The built-in decoder can't open this file ({e}).")))?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL && t.codec_params.sample_rate.is_some())
        .ok_or_else(|| unsupported("No audio track the built-in decoder understands.".into()))?;
    let track_id = track.id;
    let rate = track.codec_params.sample_rate.unwrap_or(RATE as u32);
    on_duration(track.codec_params.n_frames.map(|n| n as f64 / rate as f64));
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| unsupported(format!("The built-in decoder doesn't support this audio codec ({e}).")))?;

    let mut mono = Vec::new();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(cancelled());
        }
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(Error::ResetRequired) => break,
            Err(e) if !*started => return Err(unsupported(format!("Couldn't read the audio ({e})."))),
            Err(_) => break,
        };
        if packet.track_id() != track_id {
            continue;
        }
        let buf = match decoder.decode(&packet) {
            Ok(b) => b,
            // A damaged packet: skip it, like media players do.
            Err(Error::DecodeError(_)) => continue,
            Err(e) if !*started => return Err(unsupported(format!("Couldn't decode the audio ({e})."))),
            Err(_) => break,
        };
        let spec = *buf.spec();
        let channels = spec.channels.count().max(1);
        let mut sb = SampleBuffer::<f32>::new(buf.capacity() as u64, spec);
        sb.copy_interleaved_ref(buf);
        mono.clear();
        mono.extend(sb.samples().chunks(channels).map(|f| f.iter().sum::<f32>() / channels as f32));
        *started = true;
        sink(spec.rate, &mono)?;
    }
    Ok(())
}

pub fn find_ffmpeg() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("MYFILE_FFMPEG") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    let exe = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"].map(PathBuf::from));
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        dirs.push(PathBuf::from(local).join("Microsoft").join("WinGet").join("Links"));
    }
    dirs.into_iter().map(|d| d.join(exe)).find(|p| p.is_file())
}

/// ffmpeg converts anything it can read to 16 kHz mono f32 on stdout.
fn decode_ffmpeg(ffmpeg: &Path, path: &Path, cancel: &AtomicBool, sink: &mut dyn FnMut(u32, &[f32]) -> Result<(), AppError>) -> Result<(), AppError> {
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-nostdin", "-v", "error", "-i"])
        .arg(path)
        .args(["-vn", "-ac", "1", "-ar", "16000", "-f", "f32le", "pipe:1"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().map_err(|e| AppError::new("ffmpeg_failed", format!("Couldn't start ffmpeg: {e}")))?;
    let mut stdout = child.stdout.take().expect("piped");
    let mut bytes = vec![0u8; 64 * 1024];
    let mut carry: Vec<u8> = Vec::new();
    let mut samples = Vec::new();
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(cancelled());
        }
        let n = stdout.read(&mut bytes).map_err(AppError::from_read)?;
        if n == 0 {
            break;
        }
        carry.extend_from_slice(&bytes[..n]);
        let whole = carry.len() / 4 * 4;
        samples.clear();
        samples.extend(carry[..whole].chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])));
        carry.drain(..whole);
        if let Err(e) = sink(RATE as u32, &samples) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e);
        }
    }
    let mut err = String::new();
    if let Some(mut e) = child.stderr.take() {
        let _ = e.read_to_string(&mut err);
    }
    let status = child.wait().map_err(AppError::from_read)?;
    if !status.success() {
        let first = err.lines().next().unwrap_or("unknown error").trim().to_string();
        return Err(AppError::new("unsupported_audio", format!("ffmpeg couldn't read this file: {first}")));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The speech model
// ---------------------------------------------------------------------------

fn model_path(app: &AppHandle) -> Result<PathBuf, AppError> {
    if let Ok(p) = std::env::var("MYFILE_WHISPER_MODEL") {
        return Ok(PathBuf::from(p));
    }
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::new("no_data_dir", format!("Couldn't find the app's data folder: {e}")))?;
    Ok(dir.join("models").join(MODEL_FILE))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechStatus {
    model_installed: bool,
    model_label: &'static str,
    download_bytes: u64,
    ffmpeg: bool,
}

#[tauri::command]
pub fn speech_status(app: AppHandle) -> Result<SpeechStatus, AppError> {
    Ok(SpeechStatus {
        model_installed: model_path(&app)?.is_file(),
        model_label: MODEL_LABEL,
        download_bytes: MODEL_BYTES,
        ffmpeg: find_ffmpeg().is_some(),
    })
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Progress {
    phase: &'static str,
    done: f64,
    total: Option<f64>,
}

fn emit(app: &AppHandle, phase: &'static str, done: f64, total: Option<f64>) {
    let _ = app.emit("speech-progress", Progress { phase, done, total });
}

/// Registers a cancel flag for this run, replacing (and cancelling) any other.
fn cancel_flag(state: &AppState) -> Arc<AtomicBool> {
    let flag = Arc::new(AtomicBool::new(false));
    if let Some(old) = lock(&state.speech_cancel).replace(flag.clone()) {
        old.store(true, Ordering::Relaxed);
    }
    flag
}

#[tauri::command]
pub fn cancel_speech(state: State<AppState>) {
    if let Some(f) = lock(&state.speech_cancel).take() {
        f.store(true, Ordering::Relaxed);
    }
}

fn save_err(e: std::io::Error) -> AppError {
    AppError::new("model_save_failed", format!("Couldn't save the speech model: {e}"))
}

/// Downloads the speech model once, into the app's data folder. The file is
/// written under a temporary name and only kept if its SHA-256 matches.
#[tauri::command]
pub async fn download_speech_model(app: AppHandle, state: State<'_, AppState>) -> Result<(), AppError> {
    let dest = model_path(&app)?;
    if dest.is_file() {
        return Ok(());
    }
    let cancel = cancel_flag(&state);
    let a = app.clone();
    crate::blocking(move || {
        let dir = dest.parent().ok_or_else(|| AppError::new("no_data_dir", "No folder for the model."))?;
        std::fs::create_dir_all(dir).map_err(save_err)?;
        let resp = ureq::get(MODEL_URL)
            .timeout(std::time::Duration::from_secs(60 * 30))
            .call()
            .map_err(|e| AppError::new("download_failed", format!("Couldn't download the speech model: {e}")))?;
        let mut tmp = tempfile::NamedTempFile::new_in(dir).map_err(save_err)?;
        let mut reader = resp.into_reader();
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 256 * 1024];
        let mut done: u64 = 0;
        let mut last_emit = Instant::now();
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(AppError::new("cancelled", "Download cancelled."));
            }
            let n = reader.read(&mut buf).map_err(|e| AppError::new("download_failed", format!("The download stopped: {e}")))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            tmp.write_all(&buf[..n]).map_err(save_err)?;
            done += n as u64;
            if done > MODEL_BYTES + 1024 {
                return Err(AppError::new("download_failed", "The download was larger than the speech model should be."));
            }
            if last_emit.elapsed().as_millis() > 200 {
                emit(&a, "download", done as f64, Some(MODEL_BYTES as f64));
                last_emit = Instant::now();
            }
        }
        let digest: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if digest != MODEL_SHA256 {
            return Err(AppError::new(
                "download_failed",
                "The downloaded model didn't match its expected checksum, so MyFile deleted it. Try again later.",
            ));
        }
        tmp.as_file().sync_all().map_err(save_err)?;
        tmp.persist(&dest).map_err(|e| save_err(e.error))?;
        emit(&a, "download", MODEL_BYTES as f64, Some(MODEL_BYTES as f64));
        Ok(())
    })
    .await
}

// ---------------------------------------------------------------------------
// Transcribing
// ---------------------------------------------------------------------------

fn whisper_err(e: impl std::fmt::Display) -> AppError {
    AppError::new("transcribe_failed", format!("Speech recognition failed: {e}"))
}

fn run_chunk(
    state: &mut WhisperState,
    samples: &[f32],
    offset: usize,
    prompt: &str,
    cancel: &Arc<AtomicBool>,
    on_progress: impl FnMut(i32) + 'static,
) -> Result<Vec<Segment>, AppError> {
    // Beam search: in testing, greedy decoding dropped a whole phrase
    // ("Hall 210. OK,") from one sample.
    let mut p = FullParams::new(SamplingStrategy::BeamSearch { beam_size: 5, patience: -1.0 });
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(8);
    p.set_n_threads(threads as i32);
    p.set_language(Some("en"));
    p.set_print_progress(false);
    p.set_print_realtime(false);
    p.set_print_special(false);
    p.set_print_timestamps(false);
    p.set_suppress_blank(true);
    p.set_suppress_nst(true);
    if !prompt.is_empty() {
        // Context from the previous chunk keeps names and spelling consistent.
        p.set_initial_prompt(prompt);
    }
    let c = cancel.clone();
    // whisper-rs 0.16 boxes the closure as Box<dyn FnMut() -> bool> but its C
    // trampoline casts the pointer back to the closure's own type. Passing an
    // already-boxed closure makes those the same type; a plain closure reads
    // garbage and whisper aborts with error -6.
    let abort: Box<dyn FnMut() -> bool> = Box::new(move || c.load(Ordering::Relaxed));
    p.set_abort_callback_safe(abort);
    p.set_progress_callback_safe(on_progress);
    let result = state.full(p, samples);
    if cancel.load(Ordering::Relaxed) {
        return Err(cancelled());
    }
    result.map_err(whisper_err)?;
    let offset_ms = offset as u64 * 1000 / RATE as u64;
    let mut out = Vec::new();
    for i in 0..state.full_n_segments() {
        let Some(seg) = state.get_segment(i) else { continue };
        let text = fix_spacing(seg.to_str_lossy().map_err(whisper_err)?.trim());
        if is_non_speech(&text) {
            continue;
        }
        // Whisper timestamps are in hundredths of a second.
        out.push(Segment {
            start_ms: offset_ms + seg.start_timestamp().max(0) as u64 * 10,
            end_ms: offset_ms + seg.end_timestamp().max(0) as u64 * 10,
            text,
        });
    }
    Ok(out)
}

pub struct Transcription {
    pub segments: Vec<Segment>,
    pub duration_ms: u64,
    pub decoder: Decoder,
}

/// Decodes and transcribes `path` with the model at `model`. Progress is
/// reported as (seconds done, total seconds if known).
pub fn transcribe_file(
    model: &Path,
    path: &Path,
    cancel: Arc<AtomicBool>,
    progress: Arc<dyn Fn(f64, Option<f64>) + Send + Sync>,
) -> Result<Transcription, AppError> {
    static HOOKS: Once = Once::new();
    // Route whisper.cpp's console logging into the (silent) log facade.
    HOOKS.call_once(whisper_rs::install_logging_hooks);
    let ctx = WhisperContext::new_with_params(model, WhisperContextParameters::default())
        .map_err(|e| AppError::new("model_failed", format!("Couldn't load the speech model: {e}")))?;
    let mut wstate = ctx.create_state().map_err(whisper_err)?;

    let mut segments: Vec<Segment> = Vec::new();
    let mut chunker = Chunker::new(CHUNK_SECS, CUT_SEARCH_SECS);
    let mut resampler: Option<(u32, Resampler)> = None;
    let mut resampled = Vec::new();
    let total: Arc<std::sync::Mutex<Option<f64>>> = Arc::default();

    let mut transcribe_chunk = |segments: &mut Vec<Segment>, offset: usize, samples: Vec<f32>| -> Result<(), AppError> {
        let prompt: String = segments.iter().rev().take(3).map(|s| s.text.as_str()).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(" ");
        let len = samples.len();
        let (p, t) = (progress.clone(), total.clone());
        let on_progress = move |pct: i32| {
            let done = (offset as f64 + len as f64 * pct.clamp(0, 100) as f64 / 100.0) / RATE as f64;
            p(done, *t.lock().unwrap_or_else(|e| e.into_inner()));
        };
        segments.extend(run_chunk(&mut wstate, &samples, offset, &prompt, &cancel, on_progress)?);
        Ok(())
    };

    let mut sink = |rate: u32, mono: &[f32]| -> Result<(), AppError> {
        let r = match &mut resampler {
            Some((r0, r)) if *r0 == rate => r,
            _ => &mut resampler.insert((rate, Resampler::new(rate))).1,
        };
        resampled.clear();
        r.push(mono, &mut resampled);
        if chunker.offset + resampled.len() > MAX_SECS * RATE {
            return Err(AppError::new("too_long", format!("MyFile transcribes recordings up to {} hours.", MAX_SECS / 3600)));
        }
        for (offset, chunk) in chunker.push(&resampled) {
            transcribe_chunk(&mut segments, offset, chunk)?;
        }
        Ok(())
    };

    let mut started = false;
    let t2 = total.clone();
    let mut on_duration = move |d: Option<f64>| *t2.lock().unwrap_or_else(|e| e.into_inner()) = d;
    let decoder = match decode_builtin(path, &cancel, &mut started, &mut sink, &mut on_duration) {
        Ok(()) => Decoder::Builtin,
        // Fall back to ffmpeg only if nothing was decoded yet, so no audio is transcribed twice.
        Err(e) if e.code == "unsupported_audio" && !started => {
            let ffmpeg = find_ffmpeg().ok_or_else(|| {
                AppError::new(
                    "needs_ffmpeg",
                    format!("{} Installing ffmpeg lets MyFile read almost any audio or video format.", e.message),
                )
            })?;
            decode_ffmpeg(&ffmpeg, path, &cancel, &mut sink)?;
            Decoder::Ffmpeg
        }
        Err(e) => return Err(e),
    };
    drop(sink);
    if let Some((offset, chunk)) = chunker.finish() {
        // Whisper needs at least a second of audio.
        if chunk.len() >= RATE {
            transcribe_chunk(&mut segments, offset, chunk)?;
        }
    }
    let duration_ms = chunker.offset as u64 * 1000 / RATE as u64;
    Ok(Transcription { segments, duration_ms, decoder })
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptResult {
    segments: Vec<Segment>,
    duration_ms: u64,
    elapsed_ms: u64,
    words: usize,
    model: &'static str,
    decoder: Decoder,
    text_output_id: u64,
    srt_output_id: u64,
    vtt_output_id: u64,
    /// Actions that work now that the recording has text.
    actions: Vec<ActionInfo>,
}

#[tauri::command]
pub async fn transcribe(selection_id: u64, app: AppHandle, state: State<'_, AppState>) -> Result<TranscriptResult, AppError> {
    crate::require_current(&state, selection_id)?;
    let (_, path, info) = state.current()?;
    crate::require_kind(&info, &[FileKind::Audio, FileKind::Video], "Transcription")?;
    let model = model_path(&app)?;
    if !model.is_file() {
        return Err(AppError::new("needs_model", "Download the speech model first."));
    }
    let cancel = cancel_flag(&state);
    let a = app.clone();
    let progress: Arc<dyn Fn(f64, Option<f64>) + Send + Sync> = Arc::new(move |done, total| emit(&a, "transcribe", done, total));
    let started = Instant::now();
    emit(&app, "transcribe", 0.0, None);
    let t = crate::blocking(move || transcribe_file(&model, &path, cancel, progress)).await?;
    if t.segments.is_empty() {
        return Err(AppError::new("no_speech", "MyFile didn't hear any speech in this recording."));
    }

    let stem = crate::stem(&info.name);
    let text_output_id = state.store_output(Output {
        bytes: timestamped(&info.name, &t.segments).into_bytes(),
        file_name: format!("{stem}-transcript.txt"),
        filter_label: "Text file",
        extension: "txt",
    });
    let srt_output_id = state.store_output(Output {
        bytes: srt(&t.segments).into_bytes(),
        file_name: format!("{stem}.srt"),
        filter_label: "SubRip subtitles",
        extension: "srt",
    });
    let vtt_output_id = state.store_output(Output {
        bytes: vtt(&t.segments).into_bytes(),
        file_name: format!("{stem}.vtt"),
        filter_label: "WebVTT subtitles",
        extension: "vtt",
    });

    // The transcript becomes this file's text, so the assistant, templates,
    // and fact-checking can read it.
    let text = plain_text(&t.segments);
    let words = text.split_whitespace().count();
    let doc = Arc::new(Document { pages: vec![text], has_pages: false, ocr: false, segments: Some(Arc::new(t.segments.clone())) });
    let result = TranscriptResult {
        segments: t.segments,
        duration_ms: t.duration_ms,
        elapsed_ms: started.elapsed().as_millis() as u64,
        words,
        model: MODEL_LABEL,
        decoder: t.decoder,
        text_output_id,
        srt_output_id,
        vtt_output_id,
        actions: actions::after_transcript(),
    };
    let mut sel = lock(&state.selected);
    let s = sel
        .as_mut()
        .filter(|s| s.id == selection_id)
        .ok_or_else(|| AppError::new("stale", "You switched files, so that transcript was discarded."))?;
    s.document = Some(doc);
    s.transcript = Some(result.clone());
    Ok(result)
}

/// The transcript already made for this file, if any.
#[tauri::command]
pub fn current_transcript(selection_id: u64, state: State<AppState>) -> Result<Option<TranscriptResult>, AppError> {
    crate::require_current(&state, selection_id)?;
    Ok(lock(&state.selected).as_ref().and_then(|s| s.transcript.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: u32, secs: f64, hz: f64) -> Vec<f32> {
        (0..(rate as f64 * secs) as usize).map(|i| (2.0 * std::f64::consts::PI * hz * i as f64 / rate as f64).sin() as f32 * 0.5).collect()
    }

    fn resample_all(rate: u32, input: &[f32]) -> Vec<f32> {
        let mut r = Resampler::new(rate);
        let mut out = Vec::new();
        // Feed in odd-sized pieces, as a decoder would.
        for piece in input.chunks(997) {
            r.push(piece, &mut out);
        }
        out
    }

    #[test]
    fn resampling_keeps_duration_and_speech_band_signal() {
        for rate in [8_000, 16_000, 22_050, 44_100, 48_000] {
            let out = resample_all(rate, &sine(rate, 2.0, 300.0));
            assert!((out.len() as i64 - 32_000).abs() <= 2, "{rate}: {}", out.len());
            let peak = out.iter().fold(0f32, |m, x| m.max(x.abs()));
            assert!(peak > 0.45 && peak <= 0.51, "{rate}: peak {peak}");
        }
    }

    #[test]
    fn same_rate_passes_samples_through() {
        let input: Vec<f32> = (0..100).map(|i| i as f32 / 100.0).collect();
        assert_eq!(resample_all(16_000, &input), input);
    }

    #[test]
    fn chunks_are_cut_in_the_quiet_and_cover_everything() {
        // 25 s of tone with a silent gap at 8.0-8.5 s; chunks of 10 s, search the last 4 s.
        let mut audio = sine(16_000, 25.0, 200.0);
        for s in &mut audio[8 * RATE..8 * RATE + RATE / 2] {
            *s = 0.0;
        }
        let mut c = Chunker::new(10, 4);
        let mut chunks = Vec::new();
        for piece in audio.chunks(4_000) {
            chunks.extend(c.push(piece));
        }
        chunks.extend(c.finish());
        let first_len = chunks[0].1.len();
        assert!(first_len > 8 * RATE && first_len < 8 * RATE + RATE / 2, "cut at {first_len}");
        assert_eq!(chunks.iter().map(|(_, s)| s.len()).sum::<usize>(), audio.len());
        let mut expect = 0;
        for (off, s) in &chunks {
            assert_eq!(*off, expect);
            expect += s.len();
        }
    }

    fn seg(start: u64, end: u64, text: &str) -> Segment {
        Segment { start_ms: start, end_ms: end, text: text.into() }
    }

    #[test]
    fn transcript_formats() {
        let s = vec![seg(0, 2_500, " Hello class."), seg(2_600, 5_000, "The exam moved."), seg(9_000, 3_725_040, "Lab is due Friday.")];
        assert_eq!(plain_text(&s), "Hello class. The exam moved.\n\nLab is due Friday.");
        assert!(srt(&s).starts_with("1\n00:00:00,000 --> 00:00:02,500\nHello class.\n\n2\n"));
        assert!(srt(&s).contains("00:00:09,000 --> 01:02:05,040"));
        assert!(vtt(&s).starts_with("WEBVTT\n\n00:00:00.000 --> 00:00:02.500\nHello class.\n"));
        assert!(timestamped("a.m4a", &s).contains("[0:09] Lab is due Friday."));
        assert_eq!(clock(3_725_040), "1:02:05");
    }

    #[test]
    fn locates_quotes_even_across_segments() {
        let s = vec![seg(0, 2_000, "Hello class."), seg(2_000, 4_000, "The midterm has moved"), seg(4_000, 6_000, "to Thursday, November 19th.")];
        assert_eq!(locate("the midterm has moved to Thursday", &s), Some(2_000));
        assert_eq!(locate("November 19th", &s), Some(4_000));
        assert_eq!(locate("not said", &s), None);
    }

    #[test]
    fn missing_sentence_spaces_are_restored() {
        assert_eq!(fix_spacing("we start.The midterm moved.OK, room 210.Lab"), "we start. The midterm moved. OK, room 210. Lab");
        assert_eq!(fix_spacing("the U.S.A. team. Dr.Smith"), "the U.S.A. team. Dr. Smith");
        assert_eq!(fix_spacing("Is it?Yes!No."), "Is it? Yes! No.");
    }

    #[test]
    fn non_speech_markers_are_dropped() {
        assert!(is_non_speech("[BLANK_AUDIO]") && is_non_speech(" (music) ") && is_non_speech(""));
        assert!(!is_non_speech("(Quiz on Friday) is fine"));
    }

    /// Real speech recognition on every sample format. Needs the model:
    /// MYFILE_WHISPER_MODEL=<path to ggml-base.en.bin> cargo test live_transcribes -- --ignored --nocapture
    #[test]
    #[ignore]
    fn live_transcribes_every_sample_format() {
        let model = std::env::var("MYFILE_WHISPER_MODEL").expect("set MYFILE_WHISPER_MODEL");
        let files = [
            ("bio-200-announcements.wav", Decoder::Builtin),
            ("bio-200-announcements.mp3", Decoder::Builtin),
            ("bio-200-voice-memo.m4a", Decoder::Builtin),
            ("bio-200-lecture-recording.mp4", Decoder::Builtin),
            ("bio-200-announcements.webm", Decoder::Ffmpeg),
        ];
        for (name, decoder) in files {
            let started = Instant::now();
            let path = Path::new("../test-files/audio").join(name);
            let r = transcribe_file(Path::new(&model), &path, Arc::new(AtomicBool::new(false)), Arc::new(|_, _| {})).unwrap();
            let text = plain_text(&r.segments);
            println!("{name}: {:?}, {:.1} s audio in {:.1} s, {} segments
  {text}
", r.decoder, r.duration_ms as f64 / 1000.0, started.elapsed().as_secs_f64(), r.segments.len());
            assert_eq!(r.decoder, decoder);
            let lower = text.to_lowercase();
            assert!(lower.contains("midterm") && lower.contains("november 19"), "{name}: {text}");
            assert!((31_000..36_000).contains(&r.duration_ms), "{name}: {} ms", r.duration_ms);
        }
    }
}
