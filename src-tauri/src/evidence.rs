//! "Show in document": the page a quote or passage came from, with the
//! exact span marked. Code finds the span with the same normalization the
//! quote check uses, so what's highlighted is what was verified.
//!
//! Every lookup is limited to text MyFile already has a reason to read: the
//! open file, a document in the open folder, or a file in the search index.

use std::path::PathBuf;

use rusqlite::OptionalExtension;
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::error::AppError;
use crate::files::FileKind;
use crate::{lock, AppState};

/// Byte range in `text` of the first match of `quote`, ignoring case,
/// whitespace runs, curly quotes, and dash variants (as `ai::normalize` does).
pub fn find_span(text: &str, quote: &str) -> Option<(usize, usize)> {
    let needle = crate::ai::normalize(quote);
    if needle.is_empty() {
        return None;
    }
    // The normalized text, with each normalized char's source byte range.
    let mut norm = String::with_capacity(text.len());
    let mut map: Vec<(usize, usize)> = Vec::with_capacity(text.len());
    let mut last_space = true;
    for (i, c) in text.char_indices() {
        let end = i + c.len_utf8();
        let c = match c {
            '\u{2018}' | '\u{2019}' | '\u{201B}' | '`' => '\'',
            '\u{201C}' | '\u{201D}' => '"',
            '\u{2010}'..='\u{2015}' | '\u{2212}' => '-',
            '\u{00A0}' => ' ',
            c => c,
        };
        if c.is_whitespace() {
            if !last_space {
                norm.push(' ');
                map.push((i, end));
                last_space = true;
            }
        } else {
            for l in c.to_lowercase() {
                norm.push(l);
                map.push((i, end));
            }
            last_space = false;
        }
    }
    let at = norm.find(&needle)?;
    // Byte offset in `norm` -> index into `map` (one entry per normalized char).
    let first = norm[..at].chars().count();
    let last = first + needle.chars().count() - 1;
    Some((map[first].0, map[last].1))
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    pub name: String,
    /// 1-based page shown, or None for documents without pages.
    pub page: Option<usize>,
    pub page_count: usize,
    pub before: String,
    pub matched: String,
    pub after: String,
    /// The quote was found on this page.
    pub found: bool,
}

/// Picks the page to show and splits it around the match. With `stay`, only
/// `page` is searched (the person is paging through); otherwise `page` is
/// tried first, then every page.
pub fn locate(name: &str, pages: &[String], has_pages: bool, quote: Option<&str>, page: Option<usize>, stay: bool) -> Evidence {
    let count = pages.len().max(1);
    let wanted = page.filter(|p| *p >= 1 && *p <= pages.len()).map(|p| p - 1);
    let mut order: Vec<usize> = wanted.into_iter().collect();
    if !stay {
        order.extend((0..pages.len()).filter(|i| Some(*i) != wanted));
    }
    let hit = quote.and_then(|q| order.iter().find_map(|&i| find_span(&pages[i], q).map(|span| (i, span))));
    let (index, span) = match hit {
        Some((i, s)) => (i, Some(s)),
        None => (wanted.unwrap_or(0), None),
    };
    let text = pages.get(index).map(String::as_str).unwrap_or("");
    let (before, matched, after) = match span {
        Some((s, e)) => (&text[..s], &text[s..e], &text[e..]),
        None => (text, "", ""),
    };
    Evidence {
        name: name.to_string(),
        page: has_pages.then_some(index + 1),
        page_count: if has_pages { count } else { 1 },
        before: before.to_string(),
        matched: matched.to_string(),
        after: after.to_string(),
        found: span.is_some(),
    }
}

/// From the open file (including a transcribed recording).
#[tauri::command]
pub async fn evidence_current(
    selection_id: u64,
    quote: Option<String>,
    page: Option<usize>,
    stay: bool,
    state: State<'_, AppState>,
) -> Result<Evidence, AppError> {
    let (id, info, doc) = crate::ensure_document(&state).await?;
    if id != selection_id {
        return Err(AppError::new("stale", "You switched files, so that can't be shown."));
    }
    Ok(locate(&info.name, &doc.pages, doc.has_pages, quote.as_deref(), page, stay))
}

/// From a document in the open folder, by its name in that folder.
#[tauri::command]
pub fn evidence_folder(
    selection_id: u64,
    name: String,
    quote: Option<String>,
    page: Option<usize>,
    stay: bool,
    state: State<AppState>,
) -> Result<Evidence, AppError> {
    let folder = lock(&state.folder);
    let f = folder
        .as_ref()
        .filter(|f| f.id == selection_id)
        .ok_or_else(|| AppError::new("stale", "That folder is no longer open."))?;
    let doc = f.docs.iter().find(|d| d.name == name).ok_or_else(|| AppError::new("not_found", "That document isn't in the open folder."))?;
    Ok(locate(&doc.name, &doc.pages, doc.has_pages, quote.as_deref(), page, stay))
}

fn index_db(app: &AppHandle) -> Result<rusqlite::Connection, AppError> {
    let dir = app.path().app_data_dir().map_err(|e| AppError::new("no_data_dir", e.to_string()))?;
    crate::search::open_db(&dir.join("search-index.sqlite"))
}

fn load(path: PathBuf, kind: FileKind) -> Result<(Vec<String>, bool), AppError> {
    crate::load_document(&path, kind).map(|d| (d.pages, d.has_pages))
}

/// From an indexed file. Only paths in the search index are allowed, so
/// the interface can't use this to read anything else.
#[tauri::command]
pub async fn evidence_indexed(
    path: String,
    quote: Option<String>,
    page: Option<usize>,
    stay: bool,
    app: AppHandle,
) -> Result<Evidence, AppError> {
    let conn = index_db(&app)?;
    let row: Option<(String, String)> = conn
        .query_row("SELECT rel, kind FROM files WHERE path = ?1", [&path], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()
        .map_err(|e| AppError::new("index_failed", e.to_string()))?;
    let (rel, kind) = row.ok_or_else(|| AppError::new("not_indexed", "That file isn't in the search index."))?;
    let kind = if kind == "pdf" { FileKind::Pdf } else { FileKind::Text };
    let p = PathBuf::from(&path);
    let (pages, has_pages) = crate::blocking(move || load(p, kind)).await?;
    Ok(locate(&rel, &pages, has_pages, quote.as_deref(), page, stay))
}

/// The sentence of a passage with the most search words, so the viewer
/// marks the part that matched rather than the whole passage.
pub fn best_sentence(passage: &str, query: &str) -> Option<String> {
    let stems = crate::search::stems(&crate::search::query_terms(query));
    let mut sentences = Vec::new();
    let mut start = 0;
    let chars: Vec<(usize, char)> = passage.char_indices().collect();
    for (k, &(i, c)) in chars.iter().enumerate() {
        let next_ws = chars.get(k + 1).is_none_or(|&(_, n)| n.is_whitespace());
        let blank_line = c == '\n' && chars.get(k + 1).is_some_and(|&(_, n)| n == '\n');
        if (matches!(c, '.' | '!' | '?') && next_ws) || blank_line {
            let end = i + c.len_utf8();
            sentences.push(&passage[start..end]);
            start = end;
        }
    }
    sentences.push(&passage[start..]);
    sentences
        .into_iter()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| (crate::search::stem_hits(s, &stems), s))
        .filter(|(n, _)| *n > 0)
        .max_by_key(|(n, s)| (*n, std::cmp::Reverse(s.len())))
        .map(|(_, s)| s.to_string())
}

/// A search result passage, marked on its page. The passage text comes from
/// the index; the page is read fresh from the file.
#[tauri::command]
pub async fn evidence_passage(chunk_id: i64, query: Option<String>, app: AppHandle) -> Result<(String, Evidence), AppError> {
    let conn = index_db(&app)?;
    let row: Option<(String, String, String, Option<i64>, String)> = conn
        .query_row(
            "SELECT f.path, f.rel, f.kind, c.page, c.text FROM chunks c JOIN files f ON f.id = c.file_id WHERE c.id = ?1",
            [chunk_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()
        .map_err(|e| AppError::new("index_failed", e.to_string()))?;
    let (path, rel, kind, page, text) = row.ok_or_else(|| AppError::new("not_indexed", "That passage is no longer in the index."))?;
    let kind = if kind == "pdf" { FileKind::Pdf } else { FileKind::Text };
    let p = PathBuf::from(&path);
    let (pages, has_pages) = crate::blocking(move || load(p, kind)).await?;
    let focus = query.as_deref().and_then(|q| best_sentence(&text, q)).unwrap_or(text);
    let ev = locate(&rel, &pages, has_pages, Some(&focus), page.map(|p| p as usize), false);
    Ok((path, ev))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_spans_despite_case_spacing_and_typography() {
        let text = "Intro.\n\nLate  lab   reports are\nnot “accepted” for any reason — ever.";
        let (s, e) = find_span(text, "late lab reports are not \"accepted\" for any reason - ever").unwrap();
        assert_eq!(&text[s..e], "Late  lab   reports are\nnot “accepted” for any reason — ever");
        assert_eq!(find_span(text, "not in here"), None);
        assert_eq!(find_span(text, "   "), None);
        // Multi-byte characters before the match keep offsets right.
        let t2 = "Café déjà vu. The fee is €20.";
        let (s, e) = find_span(t2, "the fee is €20").unwrap();
        assert_eq!(&t2[s..e], "The fee is €20");
    }

    #[test]
    fn best_sentence_picks_the_matching_one() {
        let p = "Exams

Exam 2 will be held on November 20, 2026.

The cumulative final exam is on December 11, 2026. Late lab reports are not accepted.";
        assert_eq!(best_sentence(p, "cumulative final exam").as_deref(), Some("The cumulative final exam is on December 11, 2026."));
        assert_eq!(best_sentence(p, "late reports").as_deref(), Some("Late lab reports are not accepted."));
        assert_eq!(best_sentence(p, "zebra"), None);
    }

    #[test]
    fn locate_prefers_the_cited_page_then_searches_the_rest() {
        let pages: Vec<String> = vec!["Cover page.".into(), "Rules. Notice is due March 31.".into(), "Notice is due March 31 (copy).".into()];
        let e = locate("lease.pdf", &pages, true, Some("notice is due March 31"), Some(3), false);
        assert_eq!((e.page, e.found, e.matched.as_str()), (Some(3), true, "Notice is due March 31"));
        // A wrong page still finds the quote elsewhere.
        let e = locate("lease.pdf", &pages, true, Some("Rules."), Some(1), false);
        assert_eq!((e.page, e.found), (Some(2), true));
        assert_eq!(e.before, "");
        assert_eq!(e.after, " Notice is due March 31.");
        // Paging through: stay on the asked page even without a match.
        let e = locate("lease.pdf", &pages, true, Some("Rules."), Some(1), true);
        assert_eq!((e.page, e.found, e.before.as_str()), (Some(1), false, "Cover page."));
        // Documents without pages.
        let e = locate("notes.txt", &["Hello there.".to_string()], false, Some("there"), None, false);
        assert_eq!((e.page, e.page_count, e.matched.as_str()), (None, 1, "there"));
    }
}
