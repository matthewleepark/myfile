//! OCR: reading text from scanned PDFs and photos, locally, with Tesseract.
//!
//! No model is involved here and nothing leaves the computer. A scanned PDF
//! page is usually one embedded picture of the page, so MyFile pulls that
//! picture out (JPEG data as-is; raw gray, RGB, or 1-bit data rebuilt) and
//! runs the Tesseract command on it. The text then flows into the same
//! pipeline as any other document, so every quote the AI gives is checked
//! against the OCR text it was actually shown.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use image::{DynamicImage, GrayImage, RgbImage};

use crate::error::AppError;

const PAGE_TIMEOUT: Duration = Duration::from_secs(120);
/// Scans larger than this are scaled down first; Tesseract gains nothing from more.
const MAX_SIDE: u32 = 4000;

/// Where Tesseract is installed, if anywhere we know to look.
pub fn find_tesseract() -> Option<PathBuf> {
    let exe = if cfg!(windows) { "tesseract.exe" } else { "tesseract" };
    if let Some(p) = std::env::var_os("MYFILE_TESSERACT").map(PathBuf::from) {
        if p.is_file() {
            return Some(p);
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let c = dir.join(exe);
            if c.is_file() {
                return Some(c);
            }
        }
    }
    let mut candidates: Vec<PathBuf> = vec![
        // Windows installers (all users, then per user)
        PathBuf::from(r"C:\Program Files\Tesseract-OCR\tesseract.exe"),
        PathBuf::from(r"C:\Program Files (x86)\Tesseract-OCR\tesseract.exe"),
        // macOS Homebrew (Apple Silicon, then Intel), Linux
        PathBuf::from("/opt/homebrew/bin/tesseract"),
        PathBuf::from("/usr/local/bin/tesseract"),
        PathBuf::from("/usr/bin/tesseract"),
    ];
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(PathBuf::from(local).join(r"Programs\Tesseract-OCR\tesseract.exe"));
    }
    candidates.into_iter().find(|p| p.is_file())
}

pub fn missing() -> AppError {
    AppError::new(
        "ocr_unavailable",
        if cfg!(target_os = "macos") {
            "This needs OCR, which uses the free Tesseract program. Install it with Homebrew (brew install tesseract), then try again."
        } else {
            "This needs OCR, which uses the free Tesseract program. Install \"Tesseract-OCR\" (for example: winget install tesseract-ocr.tesseract), then try again."
        },
    )
}

pub fn available() -> bool {
    find_tesseract().is_some()
}

/// Reads the text in an image.
pub fn image_text(img: &DynamicImage) -> Result<String, AppError> {
    let tesseract = find_tesseract().ok_or_else(missing)?;
    let img = if img.width().max(img.height()) > MAX_SIDE { img.thumbnail(MAX_SIDE, MAX_SIDE) } else { img.clone() };
    // A private temporary folder, deleted when this function returns.
    let dir = tempfile::tempdir().map_err(|e| AppError::new("ocr_failed", format!("Couldn't prepare OCR ({e}).")))?;
    let input = dir.path().join("page.png");
    img.to_luma8()
        .save(&input)
        .map_err(|e| AppError::new("ocr_failed", format!("Couldn't prepare the image for OCR ({e}).")))?;
    run_tesseract(&tesseract, &input)
}

/// Runs `tesseract <input> stdout`, with a time limit. Arguments are passed
/// directly, never through a shell.
fn run_tesseract(tesseract: &Path, input: &Path) -> Result<String, AppError> {
    let mut cmd = Command::new(tesseract);
    cmd.arg(input).arg("stdout").args(["-l", "eng", "--psm", "3"]);
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // no console window
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| AppError::new("ocr_failed", format!("Couldn't start Tesseract ({e}).")))?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() > PAGE_TIMEOUT => {
                let _ = child.kill();
                return Err(AppError::new("ocr_timeout", "OCR took too long on a page and was stopped."));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(AppError::new("ocr_failed", format!("Tesseract stopped unexpectedly ({e})."))),
        }
    }
    let out = child
        .wait_with_output()
        .map_err(|e| AppError::new("ocr_failed", format!("Couldn't read Tesseract's output ({e}).")))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(AppError::new("ocr_failed", format!("Tesseract failed: {}", err.trim().chars().take(200).collect::<String>())));
    }
    Ok(tidy(&String::from_utf8_lossy(&out.stdout)))
}

/// Collapses OCR's runs of blank lines and stray form feeds.
fn tidy(s: &str) -> String {
    let mut out = String::new();
    let mut blank = 0;
    for line in s.replace('\u{000C}', "\n").lines() {
        let line = line.trim_end();
        if line.is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim().to_string()
}

/// The main picture on each page of a PDF (None for pages without one MyFile
/// can decode, e.g. JBIG2 or JPEG 2000 scans).
pub fn pdf_page_images(bytes: &[u8]) -> Result<Vec<Option<DynamicImage>>, AppError> {
    let doc = pdf_extract::Document::load_mem(bytes)
        .map_err(|_| AppError::new("invalid_pdf", "MyFile couldn't open this PDF for OCR."))?;
    let mut pages = Vec::new();
    for (_, page_id) in doc.get_pages() {
        let images = doc.get_page_images(page_id).unwrap_or_default();
        // The scan is the biggest picture on the page.
        let best = images.iter().max_by_key(|i| i.width.max(0) * i.height.max(0));
        pages.push(best.and_then(|img| decode_pdf_image(&doc, img)));
    }
    Ok(pages)
}

fn decode_pdf_image(doc: &pdf_extract::Document, img: &pdf_extract::xobject::PdfImage) -> Option<DynamicImage> {
    let filters = img.filters.clone().unwrap_or_default();
    let (w, h) = (u32::try_from(img.width).ok()?, u32::try_from(img.height).ok()?);
    if w == 0 || h == 0 || (w as u64) * (h as u64) > 100_000_000 {
        return None;
    }
    // JPEG data is a complete JPEG file.
    if filters.iter().any(|f| f == "DCTDecode") {
        return image::load_from_memory(img.content).ok();
    }
    if filters.iter().any(|f| matches!(f.as_str(), "JPXDecode" | "JBIG2Decode" | "CCITTFaxDecode")) {
        return None;
    }
    let raw = if filters.is_empty() {
        img.content.to_vec()
    } else {
        doc.get_object(img.id).ok()?.as_stream().ok()?.decompressed_content().ok()?
    };
    let bpc = img.bits_per_component.unwrap_or(8);
    let space = match img.color_space.as_deref().unwrap_or("DeviceGray") {
        // ICC profiles (common from scanners and browsers) and calibrated
        // spaces: what matters for OCR is the number of channels.
        "ICCBased" => match icc_components(doc, img.origin_dict) {
            Some(1) => "DeviceGray",
            Some(3) => "DeviceRGB",
            _ => return None,
        },
        "CalGray" => "DeviceGray",
        "CalRGB" => "DeviceRGB",
        other => other,
    };
    raw_to_image(&raw, w, h, bpc, space)
}

/// The channel count (/N) of the ICC profile an image uses.
fn icc_components(doc: &pdf_extract::Document, dict: &pdf_extract::Dictionary) -> Option<i64> {
    let cs = dict.get(b"ColorSpace").ok()?;
    let arr = match cs {
        pdf_extract::Object::Reference(id) => doc.get_object(*id).ok()?.as_array().ok()?,
        other => other.as_array().ok()?,
    };
    let stream = match arr.get(1)? {
        pdf_extract::Object::Reference(id) => doc.get_object(*id).ok()?.as_stream().ok()?,
        other => other.as_stream().ok()?,
    };
    stream.dict.get(b"N").ok()?.as_i64().ok()
}

/// Rebuilds an image from uncompressed PDF sample data.
pub fn raw_to_image(raw: &[u8], w: u32, h: u32, bpc: i64, space: &str) -> Option<DynamicImage> {
    let (wu, hu) = (w as usize, h as usize);
    match (space, bpc) {
        ("DeviceGray", 8) if raw.len() >= wu * hu => GrayImage::from_raw(w, h, raw[..wu * hu].to_vec()).map(DynamicImage::ImageLuma8),
        ("DeviceRGB", 8) if raw.len() >= wu * hu * 3 => RgbImage::from_raw(w, h, raw[..wu * hu * 3].to_vec()).map(DynamicImage::ImageRgb8),
        // 1-bit scans: each row is padded to a whole byte; 0 is black.
        ("DeviceGray", 1) => {
            let row = wu.div_ceil(8);
            if raw.len() < row * hu {
                return None;
            }
            let mut px = Vec::with_capacity(wu * hu);
            for y in 0..hu {
                for x in 0..wu {
                    let bit = (raw[y * row + x / 8] >> (7 - (x % 8))) & 1;
                    px.push(if bit == 1 { 255 } else { 0 });
                }
            }
            GrayImage::from_raw(w, h, px).map(DynamicImage::ImageLuma8)
        }
        _ => None,
    }
}

/// OCRs every page of a scanned PDF. Pages without a usable picture are empty.
pub fn pdf_text(path: &Path) -> Result<Vec<String>, AppError> {
    if !available() {
        return Err(missing());
    }
    let bytes = std::fs::read(path).map_err(AppError::from_read)?;
    let images = pdf_page_images(&bytes)?;
    if images.iter().all(|i| i.is_none()) {
        return Err(AppError::new(
            "ocr_unsupported",
            "This PDF has no text and its pages use an image format MyFile can't read for OCR.",
        ));
    }
    images
        .iter()
        .map(|img| match img {
            Some(i) => image_text(i),
            None => Ok(String::new()),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebuilds_one_bit_scans_row_by_row() {
        // 10 px wide → 2 bytes per row. Row 0: first pixel white; row 1: all black.
        let raw = [0b1000_0000, 0b0000_0000, 0, 0];
        let img = raw_to_image(&raw, 10, 2, 1, "DeviceGray").unwrap().to_luma8();
        assert_eq!(img.get_pixel(0, 0).0, [255]);
        assert_eq!(img.get_pixel(1, 0).0, [0]);
        assert_eq!(img.get_pixel(9, 1).0, [0]);
    }

    #[test]
    fn rejects_truncated_or_unknown_data() {
        assert!(raw_to_image(&[0; 5], 4, 4, 8, "DeviceGray").is_none());
        assert!(raw_to_image(&[0; 64], 4, 4, 8, "DeviceCMYK").is_none());
        assert!(raw_to_image(&[0; 48], 4, 4, 8, "DeviceRGB").is_some());
    }

    #[test]
    fn tidies_ocr_output() {
        assert_eq!(tidy("Line one  \n\n\n\u{000C}Line two\n\n"), "Line one\n\nLine two");
    }

    #[test]
    fn finds_the_page_pictures_in_a_scanned_pdf() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../test-files/scanned-syllabus.pdf");
        // No text layer, so normal extraction refuses it...
        assert_eq!(crate::pdf::extract(&path).unwrap_err().code, "no_text");
        // ...but each page's picture can be decoded for OCR.
        let pages = pdf_page_images(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(pages.len(), 2);
        for p in &pages {
            let img = p.as_ref().expect("page picture decoded");
            assert!(img.width() >= 600 && img.height() >= 800, "{}x{}", img.width(), img.height());
        }
    }

    /// Needs Tesseract installed. Run: cargo test ocr_live -- --ignored --nocapture
    #[test]
    #[ignore]
    fn ocr_live_reads_the_scanned_sample() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../test-files/scanned-syllabus.pdf");
        let pages = pdf_text(&path).unwrap();
        println!("{pages:#?}");
        assert!(pages.join(" ").to_lowercase().contains("november"));
    }
}

