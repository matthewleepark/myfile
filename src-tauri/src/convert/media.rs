//! Images (built in, or decoded by ffmpeg), audio and video (ffmpeg), and
//! subtitles.

use std::io::{BufRead, BufReader, Cursor, Read};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;

use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat};

use super::{pdfwrite, Job};
use crate::error::AppError;

/// Formats the Rust `image` crate reads in MyFile's build.
pub fn builtin_decodes(format: &str) -> bool {
    matches!(format, "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tiff" | "ico" | "tga" | "ppm" | "qoi")
}

fn fail(e: impl std::fmt::Display) -> AppError {
    AppError::new("convert_failed", format!("The conversion failed: {e}"))
}

fn no_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let _ = cmd;
}

/// Runs ffmpeg with `-progress` on stdout, reporting progress against
/// `duration` and stopping it if the job is cancelled.
pub fn run_ffmpeg(job: &Job, args: &[String], duration: Option<f64>) -> Result<(), AppError> {
    let ffmpeg = job.tools.ffmpeg.as_ref().ok_or_else(|| AppError::new("needs_tool", "This conversion needs ffmpeg."))?;
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-hide_banner", "-nostdin", "-y", "-loglevel", "error", "-progress", "pipe:1", "-nostats"])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    no_window(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| AppError::new("convert_failed", format!("Couldn't start ffmpeg: {e}")))?;
    // Drain stderr on its own thread so a chatty ffmpeg can't block.
    let mut stderr = child.stderr.take().expect("piped");
    let err_thread = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });
    let stdout = child.stdout.take().expect("piped");
    for line in BufReader::new(stdout).lines() {
        if job.cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(AppError::new("cancelled", "Conversion cancelled."));
        }
        let Ok(line) = line else { break };
        if let Some(us) = line.strip_prefix("out_time_us=").and_then(|v| v.trim().parse::<f64>().ok()) {
            (job.progress)(duration.filter(|d| *d > 0.0).map(|d| (us / 1e6 / d).clamp(0.0, 1.0)));
        }
    }
    let status = child.wait().map_err(fail)?;
    let err = err_thread.join().unwrap_or_default();
    if job.cancel.load(Ordering::Relaxed) {
        return Err(AppError::new("cancelled", "Conversion cancelled."));
    }
    if !status.success() {
        let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("unknown error").trim().to_string();
        return Err(AppError::new("convert_failed", format!("ffmpeg couldn't convert this file: {last}")));
    }
    Ok(())
}

pub struct Probe {
    pub duration: Option<f64>,
    pub has_video: bool,
    pub has_audio: bool,
}

/// Duration and which kinds of streams a media file has (ffprobe).
pub fn probe(job: &Job) -> Probe {
    let Some(ffprobe) = &job.tools.ffprobe else { return Probe { duration: None, has_video: true, has_audio: true } };
    let mut cmd = Command::new(ffprobe);
    cmd.args(["-v", "error", "-show_entries", "format=duration:stream=codec_type,disposition", "-of", "json"]).arg(job.input);
    no_window(&mut cmd);
    let Ok(out) = cmd.output() else { return Probe { duration: None, has_video: true, has_audio: true } };
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    let streams = v["streams"].as_array().cloned().unwrap_or_default();
    // Cover art in an audio file shows up as an "attached picture" video stream.
    let has_video = streams.iter().any(|s| s["codec_type"] == "video" && s["disposition"]["attached_pic"] != 1);
    Probe {
        duration: v["format"]["duration"].as_str().and_then(|d| d.parse().ok()),
        has_video,
        has_audio: streams.iter().any(|s| s["codec_type"] == "audio"),
    }
}

fn scale_filter(max: Option<u32>) -> Option<String> {
    max.map(|m| format!("scale=w='min(iw,{m})':h='min(ih,{m})':force_original_aspect_ratio=decrease:force_divisible_by=2"))
}

// ---------------------------------------------------------------------------
// Images
// ---------------------------------------------------------------------------

fn decode(job: &Job) -> Result<DynamicImage, AppError> {
    if builtin_decodes(job.format) {
        return crate::image_ops::load(job.input);
    }
    // HEIC, AVIF, PSD, EXR...: ffmpeg decodes the first frame to PNG.
    let tmp = job.out_dir.join(".decoded.png");
    run_ffmpeg(job, &["-i".into(), job.input.to_string_lossy().into_owned(), "-frames:v".into(), "1".into(), tmp.to_string_lossy().into_owned()], None)
        .map_err(|e| AppError::new(e.code, format!("{} (ffmpeg's support for {} files depends on how it was built.)", e.message, job.format.to_uppercase())))?;
    let img = crate::image_ops::load(&tmp);
    let _ = std::fs::remove_file(&tmp);
    img
}

pub fn fit(img: DynamicImage, max: Option<u32>) -> DynamicImage {
    match max {
        Some(m) if img.width().max(img.height()) > m => img.resize(m, m, FilterType::Lanczos3),
        _ => img,
    }
}

pub fn encode(img: &DynamicImage, target: &str, quality: u8) -> Result<Vec<u8>, AppError> {
    let mut out = Cursor::new(Vec::new());
    match target {
        "jpg" => {
            let rgb = img.to_rgb8();
            JpegEncoder::new_with_quality(&mut out, quality).encode_image(&rgb).map_err(fail)?;
        }
        "ico" => {
            // Icons are at most 256 x 256.
            let small = if img.width().max(img.height()) > 256 { img.resize(256, 256, FilterType::Lanczos3) } else { img.clone() };
            small.to_rgba8().write_to(&mut out, ImageFormat::Ico).map_err(fail)?;
        }
        "gif" => img.to_rgba8().write_to(&mut out, ImageFormat::Gif).map_err(fail)?,
        "bmp" => img.to_rgb8().write_to(&mut out, ImageFormat::Bmp).map_err(fail)?,
        "tga" => img.to_rgba8().write_to(&mut out, ImageFormat::Tga).map_err(fail)?,
        "ppm" => img.to_rgb8().write_to(&mut out, ImageFormat::Pnm).map_err(fail)?,
        "qoi" => img.to_rgba8().write_to(&mut out, ImageFormat::Qoi).map_err(fail)?,
        "tiff" => img.write_to(&mut out, ImageFormat::Tiff).map_err(fail)?,
        "webp" => img.to_rgba8().write_to(&mut out, ImageFormat::WebP).map_err(fail)?,
        _ => img.write_to(&mut out, ImageFormat::Png).map_err(fail)?,
    }
    Ok(out.into_inner())
}

pub fn image(job: &Job) -> Result<Vec<PathBuf>, AppError> {
    let quality = job.settings.quality.unwrap_or(85).clamp(1, 100);
    let img = fit(decode(job)?, job.settings.max_dimension);
    job.check_cancel()?;
    let out = job.out(job.target);
    match job.target {
        "pdf" => std::fs::write(&out, pdfwrite::from_images(&[img], quality)?).map_err(fail)?,
        // Lossy WebP and AVIF come from ffmpeg's encoders; the built-in WebP
        // encoder is lossless only (used when ffmpeg isn't there).
        "webp" | "avif" if job.tools.ffmpeg.is_some() => {
            let tmp = job.out_dir.join(".source.png");
            std::fs::write(&tmp, encode(&img, "png", quality)?).map_err(fail)?;
            let mut args = vec!["-i".to_string(), tmp.to_string_lossy().into_owned()];
            if job.target == "webp" {
                args.extend(["-c:v", "libwebp", "-quality"].map(String::from));
                args.push(quality.to_string());
            } else {
                let crf = (60.0 - quality as f64 * 0.4).round().clamp(10.0, 55.0) as u32;
                args.extend(["-c:v", "libaom-av1", "-still-picture", "1", "-cpu-used", "6", "-crf"].map(String::from));
                args.push(crf.to_string());
                args.extend(["-pix_fmt", "yuv420p"].map(String::from));
            }
            args.push(out.to_string_lossy().into_owned());
            let r = run_ffmpeg(job, &args, None);
            let _ = std::fs::remove_file(&tmp);
            r?;
        }
        t => std::fs::write(&out, encode(&img, t, quality)?).map_err(fail)?,
    }
    Ok(vec![out])
}

// ---------------------------------------------------------------------------
// Audio and video
// ---------------------------------------------------------------------------

fn audio_args(target: &str, bitrate: Option<u32>) -> Vec<String> {
    let br = |default: u32| format!("{}k", bitrate.unwrap_or(default));
    let v: Vec<String> = match target {
        "mp3" => vec!["-c:a".into(), "libmp3lame".into(), "-b:a".into(), br(192)],
        "m4a" => vec!["-c:a".into(), "aac".into(), "-b:a".into(), br(192)],
        "ogg" => vec!["-c:a".into(), "libvorbis".into(), "-b:a".into(), br(160)],
        "opus" => vec!["-c:a".into(), "libopus".into(), "-b:a".into(), br(128)],
        "flac" => vec!["-c:a".into(), "flac".into()],
        "aiff" => vec!["-c:a".into(), "pcm_s16be".into()],
        _ => vec!["-c:a".into(), "pcm_s16le".into()],
    };
    v
}

pub fn av(job: &Job) -> Result<Vec<PathBuf>, AppError> {
    let p = probe(job);
    let input = job.input.to_string_lossy().into_owned();
    let out = job.out(job.target);
    let out_s = out.to_string_lossy().into_owned();
    let audio_only = matches!(job.target, "mp3" | "m4a" | "wav" | "flac" | "ogg" | "opus" | "aiff");
    if audio_only && !p.has_audio {
        return Err(AppError::new("no_audio", "This file has no sound track to convert."));
    }
    if !audio_only && !p.has_video {
        return Err(AppError::new("no_video", "This file has no video picture to convert."));
    }
    let q = job.settings.quality.unwrap_or(75).clamp(1, 100) as f64;
    // Map 1-100 quality onto the encoders' quality scales.
    let x264_crf = (51.0 - q * 0.35).round().clamp(16.0, 35.0) as u32;
    let mut args: Vec<String> = vec!["-i".into(), input];
    let mut duration = p.duration;
    match job.target {
        t if audio_only => {
            args.push("-vn".into());
            args.extend(audio_args(t, job.settings.bitrate));
        }
        "gif" => {
            let m = job.settings.max_dimension.unwrap_or(480);
            duration = duration.map(|d| d.min(15.0));
            args.extend(["-t", "15", "-filter_complex"].map(String::from));
            args.push(format!(
                "fps=12,scale=w='min(iw,{m})':h='min(ih,{m})':force_original_aspect_ratio=decrease:flags=lanczos,split[a][b];[a]palettegen[p];[b][p]paletteuse"
            ));
            args.extend(["-loop", "0"].map(String::from));
        }
        "jpg" => {
            let at = p.duration.map(|d| (d * 0.1).min(1.0)).unwrap_or(0.0);
            args.splice(0..0, ["-ss".to_string(), format!("{at:.2}")]);
            args.extend(["-frames:v", "1", "-q:v", "2"].map(String::from));
            if let Some(f) = scale_filter(job.settings.max_dimension) {
                args.extend(["-vf".to_string(), f]);
            }
            duration = None;
        }
        "mkv" if job.format != "mkv" && job.settings.quality.is_none() && job.settings.max_dimension.is_none() => {
            // Same streams in a new container: instant and lossless.
            args.extend(["-map", "0", "-c", "copy"].map(String::from));
        }
        t => {
            if let Some(f) = scale_filter(job.settings.max_dimension) {
                args.extend(["-vf".to_string(), f]);
            }
            match t {
                "webm" => {
                    args.extend(["-c:v", "libvpx-vp9", "-b:v", "0", "-row-mt", "1", "-deadline", "good", "-cpu-used", "5", "-crf"].map(String::from));
                    args.push((x264_crf + 8).to_string());
                    args.extend(["-c:a", "libopus", "-b:a", "128k"].map(String::from));
                }
                "avi" => {
                    let qv = (11.0 - q / 10.0).round().clamp(2.0, 10.0) as u32;
                    args.extend(["-c:v", "mpeg4", "-q:v"].map(String::from));
                    args.push(qv.to_string());
                    args.extend(["-c:a", "libmp3lame", "-b:a", "192k"].map(String::from));
                }
                _ => {
                    args.extend(["-c:v", "libx264", "-preset", "veryfast", "-pix_fmt", "yuv420p", "-crf"].map(String::from));
                    args.push(x264_crf.to_string());
                    args.extend(["-c:a", "aac", "-b:a", "160k"].map(String::from));
                    if t == "mp4" || t == "mov" {
                        args.extend(["-movflags", "+faststart"].map(String::from));
                    }
                }
            }
        }
    }
    args.push(out_s);
    run_ffmpeg(job, &args, duration)?;
    Ok(vec![out])
}

// ---------------------------------------------------------------------------
// Subtitles
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq)]
pub struct Cue {
    pub start: String,
    pub end: String,
    pub text: Vec<String>,
}

/// Parses SRT or WebVTT cues (timestamps kept as written, normalized later).
pub fn parse_cues(text: &str) -> Vec<Cue> {
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let mut cues = Vec::new();
    for block in text.split("\n\n") {
        let lines: Vec<&str> = block.lines().filter(|l| !l.trim().is_empty()).collect();
        let Some(pos) = lines.iter().position(|l| l.contains("-->")) else { continue };
        let mut parts = lines[pos].split("-->");
        let start = parts.next().unwrap_or("").trim().to_string();
        // VTT cue settings follow the end time ("00:01.000 align:start").
        let end = parts.next().unwrap_or("").split_whitespace().next().unwrap_or("").to_string();
        cues.push(Cue { start, end, text: lines[pos + 1..].iter().map(|l| l.to_string()).collect() });
    }
    cues
}

/// "1:02.5" / "00:01:02,500" / "00:01:02.500" -> "00:01:02<sep>500".
pub fn norm_time(t: &str, sep: char) -> String {
    let t = t.replace(',', ".");
    let (hms, ms) = t.split_once('.').unwrap_or((&t, "0"));
    let parts: Vec<u64> = hms.split(':').filter_map(|p| p.trim().parse().ok()).collect();
    let (h, m, s) = match parts.as_slice() {
        [h, m, s] => (*h, *m, *s),
        [m, s] => (0, *m, *s),
        [s] => (0, 0, *s),
        _ => (0, 0, 0),
    };
    let ms: u64 = format!("{:0<3}", &ms[..ms.len().min(3)]).parse().unwrap_or(0);
    format!("{h:02}:{m:02}:{s:02}{sep}{ms:03}")
}

pub fn write_cues(cues: &[Cue], target: &str) -> String {
    match target {
        "vtt" => {
            let mut out = String::from("WEBVTT\n\n");
            for c in cues {
                out.push_str(&format!("{} --> {}\n{}\n\n", norm_time(&c.start, '.'), norm_time(&c.end, '.'), c.text.join("\n")));
            }
            out
        }
        "srt" => cues
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{}\n{} --> {}\n{}\n", i + 1, norm_time(&c.start, ','), norm_time(&c.end, ','), c.text.join("\n")))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => {
            // Plain text: the lines, without tags like <i> or {\an8}.
            let strip = |s: &str| {
                let mut out = String::new();
                let mut depth = 0;
                for ch in s.chars() {
                    match ch {
                        '<' | '{' => depth += 1,
                        '>' | '}' if depth > 0 => depth -= 1,
                        c if depth == 0 => out.push(c),
                        _ => {}
                    }
                }
                out
            };
            cues.iter().flat_map(|c| c.text.iter().map(|l| strip(l))).filter(|l| !l.trim().is_empty()).collect::<Vec<_>>().join("\n") + "\n"
        }
    }
}

pub fn subtitles(job: &Job) -> Result<Vec<PathBuf>, AppError> {
    let text = if job.format == "ass" {
        // ffmpeg turns ASS/SSA into SRT first.
        let tmp = job.out_dir.join(".source.srt");
        run_ffmpeg(job, &["-i".into(), job.input.to_string_lossy().into_owned(), tmp.to_string_lossy().into_owned()], None)?;
        let t = std::fs::read_to_string(&tmp).map_err(fail)?;
        let _ = std::fs::remove_file(&tmp);
        t
    } else {
        String::from_utf8_lossy(&std::fs::read(job.input).map_err(AppError::from_read)?).into_owned()
    };
    let cues = parse_cues(&text);
    if cues.is_empty() {
        return Err(AppError::new("convert_failed", "No subtitle cues were found in this file."));
    }
    let out = job.out(job.target);
    std::fs::write(&out, write_cues(&cues, job.target)).map_err(fail)?;
    Ok(vec![out])
}
