//! Local PDF text extraction. Nothing here touches the network.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

use crate::error::AppError;

/// PDFs longer than this are refused rather than tying up the app.
pub const MAX_PAGES: usize = 2_000;

/// Below this many non-whitespace characters per page (on average) the PDF
/// is treated as having no real text layer, e.g. a scanned document.
const MIN_CHARS_PER_PAGE: usize = 15;

#[derive(Debug, Clone)]
pub struct PdfText {
    /// Text of each page, in order. Index 0 is page 1.
    pub pages: Vec<String>,
    /// Number of pages the PDF says it has.
    pub page_count: usize,
}

impl PdfText {
    /// The whole document as plain text with a marker line before each page.
    pub fn joined(&self) -> String {
        let mut out = String::new();
        for (i, page) in self.pages.iter().enumerate() {
            if i > 0 {
                out.push_str("\n\n");
            }
            out.push_str(&format!("--- Page {} ---\n\n", i + 1));
            out.push_str(page);
        }
        out
    }

    pub fn char_count(&self) -> usize {
        self.pages.iter().map(|p| p.chars().count()).sum()
    }
}

/// Extracts text from the PDF at `path`. Slow for large files: call it from a
/// blocking task, not the UI thread.
pub fn extract(path: &Path) -> Result<PdfText, AppError> {
    let bytes = std::fs::read(path).map_err(AppError::from_read)?;

    let doc = pdf_extract::Document::load_mem(&bytes).map_err(|e| load_error(&e.to_string()))?;
    let page_count = doc.get_pages().len();
    drop(doc);
    if page_count == 0 {
        return Err(AppError::new("invalid_pdf", "This PDF doesn't contain any pages."));
    }
    if page_count > MAX_PAGES {
        return Err(AppError::new(
            "too_many_pages",
            format!("This PDF has {page_count} pages. MyFile currently handles up to {MAX_PAGES}."),
        ));
    }

    // The extraction library can panic on unusual PDFs. Contain that so a bad
    // file produces an error message instead of taking down the app.
    let pages = catch_unwind(AssertUnwindSafe(|| {
        pdf_extract::extract_text_from_mem_by_pages(&bytes)
    }))
    .map_err(|_| {
        AppError::new(
            "pdf_failed",
            "MyFile couldn't read the text in this PDF. It may use a structure the PDF reader doesn't support.",
        )
    })?
    .map_err(|e| load_error(&e.to_string()))?;

    let pages: Vec<String> = pages.iter().map(|p| tidy(p)).collect();
    let text_chars: usize = pages
        .iter()
        .map(|p| p.chars().filter(|c| !c.is_whitespace()).count())
        .sum();
    if text_chars < MIN_CHARS_PER_PAGE * page_count.min(10) {
        return Err(AppError::new(
            "no_text",
            "This PDF doesn't contain a text layer; it's probably a scan or a photo of pages.",
        ));
    }

    Ok(PdfText { pages, page_count })
}

fn load_error(detail: &str) -> AppError {
    let lower = detail.to_lowercase();
    if lower.contains("encrypt") || lower.contains("decrypt") || lower.contains("password") {
        AppError::new(
            "pdf_protected",
            "This PDF is password-protected. MyFile can't open protected PDFs yet.",
        )
    } else {
        AppError::new(
            "invalid_pdf",
            "MyFile couldn't open this PDF. The file may be damaged or not a standard PDF.",
        )
    }
}

/// Trims trailing spaces on each line and collapses runs of blank lines.
fn tidy(page: &str) -> String {
    let mut out = String::with_capacity(page.len());
    let mut blank_run = 0;
    for line in page.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            blank_run += 1;
            if blank_run > 1 {
                continue;
            }
        } else {
            blank_run = 0;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tidy_collapses_blank_lines_and_trailing_spaces() {
        assert_eq!(tidy("  a  \n\n\n\nb   \n\n"), "a\n\nb");
    }

    #[test]
    fn joined_marks_pages() {
        let t = PdfText { pages: vec!["one".into(), "two".into()], page_count: 2 };
        assert_eq!(t.joined(), "--- Page 1 ---\n\none\n\n--- Page 2 ---\n\ntwo");
        assert_eq!(t.char_count(), 6);
    }

    #[test]
    fn garbage_bytes_are_an_invalid_pdf() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("bad.pdf");
        std::fs::write(&p, b"%PDF-1.7\nthis is not really a pdf").unwrap();
        assert_eq!(extract(&p).unwrap_err().code, "invalid_pdf");
    }
}
