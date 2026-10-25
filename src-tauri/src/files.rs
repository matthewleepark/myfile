//! Validating a user-selected file and working out what kind of file it is.
//!
//! The type is decided from the file's *contents* (its leading "magic bytes"),
//! not from its extension, because an extension is just part of the name and
//! can be wrong or misleading.

use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

use serde::Serialize;

use crate::error::{human_size, AppError};

/// Largest file MyFile will accept. Keeps memory use and processing time sane.
pub const MAX_FILE_BYTES: u64 = 100 * 1024 * 1024;
/// Recordings are streamed rather than loaded whole, so they may be larger.
pub const MAX_MEDIA_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// How much of the file is read to detect its type.
const SNIFF_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FileKind {
    Image,
    Pdf,
    Text,
    Audio,
    Video,
    Unsupported,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileInfo {
    /// Filename only. The full path is deliberately not sent to the interface.
    pub name: String,
    /// Lowercased extension without the dot, or empty if there is none.
    pub extension: String,
    pub size_bytes: u64,
    pub kind: FileKind,
    /// Human-readable description of the detected type, e.g. "PNG image".
    pub type_label: String,
    /// True when the contents say one thing and the extension says another.
    pub extension_mismatch: bool,
}

/// Checks that `path` is a readable, non-empty, reasonably sized file and
/// describes it.
pub fn inspect(path: &Path) -> Result<FileInfo, AppError> {
    let meta = fs::metadata(path).map_err(AppError::from_read)?;
    if meta.is_dir() {
        // The app opens folders in "semester brain" mode instead.
        return Err(AppError::new("is_folder", "That's a folder."));
    }
    if !meta.is_file() {
        return Err(AppError::new("not_a_file", "That isn't a regular file MyFile can open."));
    }

    let size_bytes = meta.len();
    if size_bytes == 0 {
        return Err(AppError::new("empty", "That file is empty, so there's nothing to work with."));
    }
    if size_bytes > MAX_MEDIA_BYTES {
        return Err(too_large(size_bytes, MAX_MEDIA_BYTES));
    }

    let mut head = Vec::with_capacity(SNIFF_BYTES);
    File::open(path)
        .and_then(|f| f.take(SNIFF_BYTES as u64).read_to_end(&mut head))
        .map_err(AppError::from_read)?;

    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let extension = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    let (kind, type_label, extension_mismatch) = classify(&head, &extension);
    let limit = if matches!(kind, FileKind::Audio | FileKind::Video) { MAX_MEDIA_BYTES } else { MAX_FILE_BYTES };
    if size_bytes > limit {
        return Err(too_large(size_bytes, limit));
    }
    Ok(FileInfo { name, extension, size_bytes, kind, type_label, extension_mismatch })
}

fn too_large(size: u64, limit: u64) -> AppError {
    AppError::new(
        "too_large",
        format!("That file is {}. MyFile handles files of this type up to {}.", human_size(size), human_size(limit)),
    )
}

/// Decides the file kind from its first bytes, using the extension only to
/// pick a friendlier label for text files and to flag mismatches.
fn classify(head: &[u8], extension: &str) -> (FileKind, String, bool) {
    if let Some(t) = infer::get(head) {
        let detected_ext = t.extension();
        let mismatch = !extension.is_empty() && !same_extension(extension, detected_ext);
        let (kind, label) = match t.mime_type() {
            "image/png" => (FileKind::Image, "PNG image".to_string()),
            "image/jpeg" => (FileKind::Image, "JPEG image".to_string()),
            "image/webp" => (FileKind::Image, "WebP image".to_string()),
            "image/gif" => (FileKind::Image, "GIF image".to_string()),
            "image/bmp" => (FileKind::Image, "BMP image".to_string()),
            "application/pdf" => (FileKind::Pdf, "PDF document".to_string()),
            "audio/mpeg" => (FileKind::Audio, "MP3 audio".to_string()),
            "audio/x-wav" => (FileKind::Audio, "WAV audio".to_string()),
            "audio/m4a" => (FileKind::Audio, "M4A audio".to_string()),
            "audio/aac" => (FileKind::Audio, "AAC audio".to_string()),
            "audio/x-flac" => (FileKind::Audio, "FLAC audio".to_string()),
            "audio/ogg" => (FileKind::Audio, "Ogg audio".to_string()),
            "audio/opus" => (FileKind::Audio, "Opus audio".to_string()),
            "audio/x-aiff" => (FileKind::Audio, "AIFF audio".to_string()),
            "audio/amr" => (FileKind::Audio, "AMR voice recording".to_string()),
            "video/mp4" => (FileKind::Video, "MP4 video".to_string()),
            "video/x-m4v" => (FileKind::Video, "M4V video".to_string()),
            "video/quicktime" => (FileKind::Video, "QuickTime video".to_string()),
            "video/webm" => (FileKind::Video, "WebM video".to_string()),
            "video/x-matroska" => (FileKind::Video, "MKV video".to_string()),
            "video/x-msvideo" => (FileKind::Video, "AVI video".to_string()),
            "video/x-ms-wmv" => (FileKind::Video, "WMV video".to_string()),
            "video/mpeg" => (FileKind::Video, "MPEG video".to_string()),
            "video/x-flv" => (FileKind::Video, "FLV video".to_string()),
            mime => (
                FileKind::Unsupported,
                format!("{} file ({mime})", detected_ext.to_uppercase()),
            ),
        };
        return (kind, label, mismatch);
    }

    if looks_like_text(head) {
        let label = match extension {
            "md" | "markdown" => "Markdown text",
            "csv" => "CSV text",
            _ => "Plain text",
        };
        return (FileKind::Text, label.to_string(), false);
    }

    let label = if extension.is_empty() {
        "Unrecognized file".to_string()
    } else {
        format!("{} file", extension.to_uppercase())
    };
    (FileKind::Unsupported, label, false)
}

fn same_extension(given: &str, detected: &str) -> bool {
    let normalize = |e: &str| match e {
        "jpeg" | "jpe" => "jpg",
        "tiff" => "tif",
        other => other,
    }
    .to_string();
    normalize(given) == normalize(detected)
}

/// True if the bytes look like UTF-8 text (no NUL bytes, valid encoding).
/// A multi-byte character cut off at the end of the sample is tolerated.
fn looks_like_text(head: &[u8]) -> bool {
    if head.contains(&0) {
        return false;
    }
    match std::str::from_utf8(head) {
        Ok(_) => true,
        // error_len() == None means the input ended mid-character: still text.
        Err(e) => e.error_len().is_none() && e.valid_up_to() > 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp(name: &str, bytes: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        File::create(&path).unwrap().write_all(bytes).unwrap();
        (dir, path)
    }

    const PNG_HEADER: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";

    #[test]
    fn detects_png_by_content() {
        let (_d, p) = write_temp("photo.png", PNG_HEADER);
        let info = inspect(&p).unwrap();
        assert_eq!(info.kind, FileKind::Image);
        assert_eq!(info.type_label, "PNG image");
        assert!(!info.extension_mismatch);
    }

    #[test]
    fn flags_extension_that_lies() {
        let (_d, p) = write_temp("report.pdf", PNG_HEADER);
        let info = inspect(&p).unwrap();
        assert_eq!(info.kind, FileKind::Image);
        assert!(info.extension_mismatch);
    }

    #[test]
    fn detects_pdf() {
        let (_d, p) = write_temp("doc.PDF", b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n");
        let info = inspect(&p).unwrap();
        assert_eq!(info.kind, FileKind::Pdf);
        assert_eq!(info.extension, "pdf");
    }

    #[test]
    fn detects_text_including_unicode() {
        let (_d, p) = write_temp("notes.md", "# Café notes — due Friday".as_bytes());
        let info = inspect(&p).unwrap();
        assert_eq!(info.kind, FileKind::Text);
        assert_eq!(info.type_label, "Markdown text");
    }

    #[test]
    fn binary_without_magic_is_unsupported() {
        let (_d, p) = write_temp("blob.dat", &[1, 2, 0, 255, 254]);
        assert_eq!(inspect(&p).unwrap().kind, FileKind::Unsupported);
    }

    #[test]
    fn rejects_empty_missing_and_folders() {
        let (d, p) = write_temp("empty.txt", b"");
        assert_eq!(inspect(&p).unwrap_err().code, "empty");
        assert_eq!(inspect(&d.path().join("nope.txt")).unwrap_err().code, "not_found");
        assert_eq!(inspect(d.path()).unwrap_err().code, "is_folder");
    }

    #[test]
    fn rejects_oversized_files() {
        let (_d, p) = write_temp("big.txt", b"x");
        File::options().write(true).open(&p).unwrap().set_len(MAX_FILE_BYTES + 1).unwrap();
        assert_eq!(inspect(&p).unwrap_err().code, "too_large");
    }

    #[test]
    fn text_cut_mid_character_is_still_text() {
        let mut bytes = "a".repeat(10).into_bytes();
        bytes.extend_from_slice(&"é".as_bytes()[..1]);
        assert!(looks_like_text(&bytes));
    }
}
