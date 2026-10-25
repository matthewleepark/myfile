//! Search across folders the person chooses to index.
//!
//! - Words: a local SQLite FTS5 index of document text (BM25 ranking, Porter
//!   stemming), kept in MyFile's data folder and updated incrementally by
//!   size and modified time.
//! - Meaning (optional): vectors from a local embedding model through Ollama,
//!   merged with the word results by reciprocal rank fusion.
//! - Ask: the best passages go to a model with the folder-mode prompt, and
//!   every quote in the answer is checked against those passages.
//!
//! Nothing here reads outside the indexed folders, follows links, or writes
//! anywhere but the index. Snippets are built by code, never by a model.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::error::AppError;
use crate::files::{self, FileKind};
use crate::folder::{self, AskItem, FolderDoc};
use crate::{lock, AppState};

const DB_FILE: &str = "search-index.sqlite";
pub const EMBED_MODEL: &str = "nomic-embed-text";
const OLLAMA: &str = "http://127.0.0.1:11434";
const MAX_FILES_PER_FOLDER: usize = 5_000;
const MAX_DEPTH: usize = 8;
/// Passages are about this long, split at paragraph and sentence ends.
pub const CHUNK_CHARS: usize = 1_200;
/// Folders that are almost never documents and can be huge.
const SKIP_DIRS: &[&str] = &["node_modules", "target", "build", "dist", "__pycache__", "venv", ".venv", "AppData", "Library", "$RECYCLE.BIN", "System Volume Information"];
/// Extensions worth opening; the real type is still checked from the contents.
const DOC_EXTENSIONS: &[&str] = &["pdf", "txt", "md", "markdown", "csv", "tsv", "log", "tex", "rst", "org", "json", "xml", "html", "htm"];
/// How much text "ask" sends at most.
const ASK_CHARS: usize = 24_000;
const ASK_PASSAGES: usize = 14;
/// A meaning match must beat the median passage by this much. Similarity
/// levels differ a lot by query (medians 0.40-0.55 on the samples), so the
/// cut is relative. Real matches cleared it by 0.11-0.49; the best wrong
/// matches stayed under 0.10, including every passage for an unrelated query.
const MEANING_MARGIN: f32 = 0.11;

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn db_err(e: rusqlite::Error) -> AppError {
    AppError::new("index_failed", format!("The search index had a problem: {e}"))
}

// ---------------------------------------------------------------------------
// Database
// ---------------------------------------------------------------------------

pub fn open_db(path: &Path) -> Result<Connection, AppError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| AppError::new("index_failed", format!("Couldn't create the index folder: {e}")))?;
    }
    let conn = Connection::open(path).map_err(db_err)?;
    conn.busy_timeout(Duration::from_secs(10)).map_err(db_err)?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA foreign_keys = ON;
         CREATE TABLE IF NOT EXISTS roots (
             id INTEGER PRIMARY KEY,
             path TEXT UNIQUE NOT NULL,
             added_at INTEGER NOT NULL,
             indexed_at INTEGER
         );
         CREATE TABLE IF NOT EXISTS files (
             id INTEGER PRIMARY KEY,
             root_id INTEGER NOT NULL REFERENCES roots(id) ON DELETE CASCADE,
             path TEXT UNIQUE NOT NULL,
             rel TEXT NOT NULL,
             kind TEXT NOT NULL,
             size INTEGER NOT NULL,
             mtime INTEGER NOT NULL,
             has_pages INTEGER NOT NULL,
             error TEXT
         );
         CREATE TABLE IF NOT EXISTS chunks (
             id INTEGER PRIMARY KEY,
             file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
             page INTEGER,
             text TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS chunks_by_file ON chunks(file_id);
         CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(
             text, content = 'chunks', content_rowid = 'id',
             tokenize = 'porter unicode61 remove_diacritics 2'
         );
         CREATE TRIGGER IF NOT EXISTS chunks_ai AFTER INSERT ON chunks BEGIN
             INSERT INTO chunks_fts(rowid, text) VALUES (new.id, new.text);
         END;
         CREATE TRIGGER IF NOT EXISTS chunks_ad AFTER DELETE ON chunks BEGIN
             INSERT INTO chunks_fts(chunks_fts, rowid, text) VALUES ('delete', old.id, old.text);
         END;
         CREATE TABLE IF NOT EXISTS vectors (
             chunk_id INTEGER PRIMARY KEY REFERENCES chunks(id) ON DELETE CASCADE,
             model TEXT NOT NULL,
             vec BLOB NOT NULL
         );",
    )
    .map_err(db_err)?;
    Ok(conn)
}

fn db_path(app: &AppHandle) -> Result<PathBuf, AppError> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::new("no_data_dir", format!("Couldn't find the app's data folder: {e}")))?;
    Ok(dir.join(DB_FILE))
}

fn open(app: &AppHandle) -> Result<Connection, AppError> {
    open_db(&db_path(app)?)
}

// ---------------------------------------------------------------------------
// Chunking
// ---------------------------------------------------------------------------

/// Splits a page into passages of about `CHUNK_CHARS`, at paragraph ends
/// where possible, then sentence ends, then spaces.
pub fn chunk_page(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let flush = |cur: &mut String, out: &mut Vec<String>| {
        let t = cur.trim();
        if !t.is_empty() {
            out.push(t.to_string());
        }
        cur.clear();
    };
    for para in text.split("\n\n").map(str::trim).filter(|p| !p.is_empty()) {
        if cur.chars().count() + para.chars().count() + 2 > CHUNK_CHARS {
            flush(&mut cur, &mut out);
        }
        if para.chars().count() <= CHUNK_CHARS {
            if !cur.is_empty() {
                cur.push_str("\n\n");
            }
            cur.push_str(para);
            continue;
        }
        // A long paragraph: cut at the last sentence end (or space) before the limit.
        let mut rest = para;
        while rest.chars().count() > CHUNK_CHARS {
            let limit = rest.char_indices().nth(CHUNK_CHARS).map(|(i, _)| i).unwrap_or(rest.len());
            let window = &rest[..limit];
            let cut = window
                .rfind(['.', '!', '?'])
                .filter(|&i| i > limit / 2)
                .map(|i| i + 1)
                .or_else(|| window.rfind(char::is_whitespace).filter(|&i| i > limit / 2))
                .unwrap_or(limit);
            out.push(rest[..cut].trim().to_string());
            rest = rest[cut..].trim_start();
        }
        cur.push_str(rest);
    }
    flush(&mut cur, &mut out);
    out
}

// ---------------------------------------------------------------------------
// Indexing
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct IndexReport {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    pub unchanged: usize,
    pub failed: usize,
    pub skipped_over_limit: usize,
    pub embedded: usize,
}

/// Candidate documents under `root`: no hidden or skipped folders, no links,
/// at most `MAX_DEPTH` deep, by extension (the type is checked later).
pub fn walk(root: &Path, limit: usize) -> (Vec<PathBuf>, usize) {
    let mut found = Vec::new();
    let mut over = 0;
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name.starts_with('~') {
                continue;
            }
            // symlink_metadata: never follow links or junctions out of the folder.
            let Ok(meta) = std::fs::symlink_metadata(e.path()) else { continue };
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                if depth < MAX_DEPTH && !SKIP_DIRS.iter().any(|s| s.eq_ignore_ascii_case(&name)) {
                    stack.push((e.path(), depth + 1));
                }
                continue;
            }
            let ext = Path::new(&name).extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
            if meta.is_file() && DOC_EXTENSIONS.contains(&ext.as_str()) {
                if found.len() < limit {
                    found.push(e.path());
                } else {
                    over += 1;
                }
            }
        }
    }
    found.sort();
    (found, over)
}

fn stamp(meta: &std::fs::Metadata) -> (i64, i64) {
    let mtime = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0);
    (meta.len() as i64, mtime)
}

/// Brings one folder's part of the index up to date. `load` reads a
/// document's pages (the app passes `crate::load_document`; tests pass a stub).
pub fn index_root(
    conn: &Connection,
    root_id: i64,
    root: &Path,
    cancel: &AtomicBool,
    load: &dyn Fn(&Path, FileKind) -> Result<(Vec<String>, bool), AppError>,
    progress: &mut dyn FnMut(usize, usize, &str),
) -> Result<IndexReport, AppError> {
    let mut report = IndexReport::default();
    let (paths, over) = walk(root, MAX_FILES_PER_FOLDER);
    report.skipped_over_limit = over;

    let mut known: HashMap<String, (i64, i64, i64)> = HashMap::new();
    {
        let mut st = conn.prepare("SELECT id, path, size, mtime FROM files WHERE root_id = ?1").map_err(db_err)?;
        let rows = st
            .query_map([root_id], |r| Ok((r.get::<_, String>(1)?, (r.get::<_, i64>(0)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?))))
            .map_err(db_err)?;
        for row in rows {
            let (p, v) = row.map_err(db_err)?;
            known.insert(p, v);
        }
    }

    let total = paths.len();
    let mut seen = HashSet::new();
    for (i, path) in paths.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err(AppError::new("cancelled", "Indexing stopped."));
        }
        let key = path.to_string_lossy().into_owned();
        seen.insert(key.clone());
        let Ok(meta) = std::fs::symlink_metadata(path) else { continue };
        let (size, mtime) = stamp(&meta);
        let existing = known.get(&key).copied();
        if existing.is_some_and(|(_, s, m)| s == size && m == mtime) {
            report.unchanged += 1;
            continue;
        }
        let rel = path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/");
        progress(i, total, &rel);
        let kind = match files::inspect(path) {
            Ok(info) if matches!(info.kind, FileKind::Pdf | FileKind::Text) => info.kind,
            // Not really a document (e.g. a binary named .txt): forget it.
            _ => {
                if let Some((id, _, _)) = existing {
                    conn.execute("DELETE FROM files WHERE id = ?1", [id]).map_err(db_err)?;
                    report.removed += 1;
                }
                continue;
            }
        };
        let loaded = load(path, kind);
        let tx = conn.unchecked_transaction().map_err(db_err)?;
        if let Some((id, _, _)) = existing {
            tx.execute("DELETE FROM files WHERE id = ?1", [id]).map_err(db_err)?;
        }
        let kind_name = if kind == FileKind::Pdf { "pdf" } else { "text" };
        let (has_pages, error) = match &loaded {
            Ok((_, hp)) => (*hp, None),
            Err(e) => (false, Some(e.message.clone())),
        };
        tx.execute(
            "INSERT INTO files (root_id, path, rel, kind, size, mtime, has_pages, error) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![root_id, key, rel, kind_name, size, mtime, has_pages as i64, error],
        )
        .map_err(db_err)?;
        let file_id = tx.last_insert_rowid();
        if let Ok((pages, has_pages)) = &loaded {
            let mut ins = tx.prepare("INSERT INTO chunks (file_id, page, text) VALUES (?1, ?2, ?3)").map_err(db_err)?;
            for (pi, page) in pages.iter().enumerate() {
                let page_no = has_pages.then_some(pi as i64 + 1);
                for chunk in chunk_page(page) {
                    ins.execute(params![file_id, page_no, chunk]).map_err(db_err)?;
                }
            }
        }
        tx.commit().map_err(db_err)?;
        match (&loaded, existing) {
            (Err(_), _) => report.failed += 1,
            (Ok(_), Some(_)) => report.updated += 1,
            (Ok(_), None) => report.added += 1,
        }
    }
    // Files that are gone (or now beyond the limit) leave the index.
    for (p, (id, _, _)) in &known {
        if !seen.contains(p) {
            conn.execute("DELETE FROM files WHERE id = ?1", [id]).map_err(db_err)?;
            report.removed += 1;
        }
    }
    conn.execute("UPDATE roots SET indexed_at = ?1 WHERE id = ?2", params![now(), root_id]).map_err(db_err)?;
    progress(total, total, "");
    Ok(report)
}

// ---------------------------------------------------------------------------
// Meaning search: embeddings through Ollama
// ---------------------------------------------------------------------------

pub fn to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

pub fn from_blob(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

pub fn normalize(mut v: Vec<f32>) -> Vec<f32> {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 {
        v.iter_mut().for_each(|x| *x /= n);
    }
    v
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn ollama_models() -> Option<Vec<String>> {
    let tags: Value = ureq::get(&format!("{OLLAMA}/api/tags")).timeout(Duration::from_secs(3)).call().ok()?.into_json().ok()?;
    Some(tags["models"].as_array()?.iter().filter_map(|m| m["name"].as_str().map(String::from)).collect())
}

fn embed_model_installed() -> bool {
    ollama_models().is_some_and(|ms| ms.iter().any(|m| m == EMBED_MODEL || m == &format!("{EMBED_MODEL}:latest")))
}

/// nomic-embed-text expects these task prefixes.
fn embed(texts: &[String], query: bool) -> Result<Vec<Vec<f32>>, AppError> {
    let prefix = if query { "search_query: " } else { "search_document: " };
    let input: Vec<String> = texts.iter().map(|t| format!("{prefix}{t}")).collect();
    let resp: Value = ureq::post(&format!("{OLLAMA}/api/embed"))
        .timeout(Duration::from_secs(120))
        .send_json(json!({ "model": EMBED_MODEL, "input": input }))
        .map_err(|e| AppError::new("embed_failed", format!("The local embedding model didn't answer: {e}")))?
        .into_json()
        .map_err(|_| AppError::new("embed_failed", "The embedding model answered in an unexpected way."))?;
    let vecs: Vec<Vec<f32>> = resp["embeddings"]
        .as_array()
        .ok_or_else(|| AppError::new("embed_failed", "The embedding model returned no vectors."))?
        .iter()
        .map(|v| normalize(v.as_array().map(|a| a.iter().filter_map(|x| x.as_f64()).map(|x| x as f32).collect()).unwrap_or_default()))
        .collect();
    if vecs.len() != texts.len() || vecs.iter().any(|v| v.is_empty()) {
        return Err(AppError::new("embed_failed", "The embedding model returned the wrong number of vectors."));
    }
    Ok(vecs)
}

/// Adds vectors for every passage that doesn't have one yet.
fn embed_missing(conn: &Connection, cancel: &AtomicBool, progress: &mut dyn FnMut(usize, usize)) -> Result<usize, AppError> {
    let todo: Vec<(i64, String)> = {
        let mut st = conn
            .prepare("SELECT c.id, c.text FROM chunks c LEFT JOIN vectors v ON v.chunk_id = c.id AND v.model = ?1 WHERE v.chunk_id IS NULL")
            .map_err(db_err)?;
        let rows = st.query_map([EMBED_MODEL], |r| Ok((r.get(0)?, r.get(1)?))).map_err(db_err)?;
        rows.collect::<Result<_, _>>().map_err(db_err)?
    };
    let total = todo.len();
    let mut done = 0;
    for batch in todo.chunks(32) {
        if cancel.load(Ordering::Relaxed) {
            return Err(AppError::new("cancelled", "Indexing stopped."));
        }
        let texts: Vec<String> = batch.iter().map(|(_, t)| t.clone()).collect();
        let vecs = embed(&texts, false)?;
        let tx = conn.unchecked_transaction().map_err(db_err)?;
        for ((id, _), v) in batch.iter().zip(vecs) {
            tx.execute("INSERT OR REPLACE INTO vectors (chunk_id, model, vec) VALUES (?1, ?2, ?3)", params![id, EMBED_MODEL, to_blob(&v)])
                .map_err(db_err)?;
        }
        tx.commit().map_err(db_err)?;
        done += batch.len();
        progress(done, total);
    }
    Ok(done)
}

// ---------------------------------------------------------------------------
// Searching
// ---------------------------------------------------------------------------

/// Words too common to search for on their own.
const STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "can", "do", "does", "for", "from", "has", "have", "how", "i", "in", "is",
    "it", "its", "me", "my", "of", "on", "or", "our", "out", "so", "that", "the", "their", "there", "this", "to", "was", "we",
    "what", "when", "where", "which", "who", "why", "will", "with", "you", "your",
];

/// Lowercased word tokens of a query, at most 12, without common words
/// (unless the query is nothing but common words).
pub fn query_terms(q: &str) -> Vec<String> {
    let all: Vec<String> = q.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(|w| w.to_lowercase()).collect();
    let content: Vec<String> = all.iter().filter(|w| !STOPWORDS.contains(&w.as_str())).cloned().collect();
    if content.is_empty() { all } else { content }.into_iter().take(12).collect()
}

/// An FTS5 query from plain words. Every term is quoted, so nothing the user
/// types is read as FTS syntax; the last term also matches as a prefix.
pub fn fts_query(terms: &[String], all: bool) -> Option<String> {
    if terms.is_empty() {
        return None;
    }
    let parts: Vec<String> = terms
        .iter()
        .enumerate()
        .map(|(i, t)| if i + 1 == terms.len() && t.chars().count() >= 3 { format!("\"{t}\"*") } else { format!("\"{t}\"") })
        .collect();
    Some(parts.join(if all { " " } else { " OR " }))
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Part {
    pub text: String,
    pub hit: bool,
}

/// A rough stem per term: short words whole, longer ones by their first two
/// thirds ("accepting" matches "accepted", "deadlines" matches "deadline").
pub fn stems(terms: &[String]) -> Vec<String> {
    terms
        .iter()
        .map(|t| {
            let n = t.chars().count();
            t.chars().take(if n <= 4 { n } else { (n * 2).div_ceil(3).max(4) }).collect()
        })
        .collect()
}

/// How many words of `text` match one of `stems`.
pub fn stem_hits(text: &str, stems: &[String]) -> usize {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .filter(|w| {
            let w = w.to_lowercase();
            stems.iter().any(|s| w.starts_with(s.as_str()))
        })
        .count()
}

/// A short excerpt around the densest run of query words, with the words
/// marked. Matching is by word prefix so "deadlines" also marks "deadline".
pub fn snippet(text: &str, terms: &[String], width: usize) -> Vec<Part> {
    let stems = stems(terms);
    let words: Vec<(usize, usize)> = {
        let mut v = Vec::new();
        let mut start = None;
        for (i, c) in text.char_indices() {
            if c.is_alphanumeric() {
                start.get_or_insert(i);
            } else if let Some(s) = start.take() {
                v.push((s, i));
            }
        }
        if let Some(s) = start {
            v.push((s, text.len()));
        }
        v
    };
    let is_hit = |&(s, e): &(usize, usize)| {
        let w = text[s..e].to_lowercase();
        stems.iter().any(|st| w.starts_with(st.as_str()))
    };
    // Window start: the hit with the most other hits within `width` bytes after it.
    let hits: Vec<(usize, usize)> = words.iter().copied().filter(is_hit).collect();
    let anchor = hits
        .iter()
        .map(|&(s, _)| (s, hits.iter().filter(|&&(h, _)| h >= s && h < s + width).count()))
        .max_by_key(|&(s, n)| (n, std::cmp::Reverse(s)))
        .map(|(s, _)| s)
        .unwrap_or(0);
    let floor = |mut i: usize| {
        while i > 0 && !text.is_char_boundary(i) {
            i -= 1;
        }
        i
    };
    let mut from = floor(anchor.saturating_sub(width / 4));
    if let Some(sp) = text[..from].rfind(char::is_whitespace).filter(|&sp| from - sp < 20) {
        from = sp + 1;
    }
    let to = floor((from + width).min(text.len()));
    let to = text[to..].find(char::is_whitespace).map(|i| to + i).filter(|&i| i - to < 20).unwrap_or(to);

    let mut parts: Vec<Part> = Vec::new();
    let mut push = |t: &str, hit: bool| {
        if t.is_empty() {
            return;
        }
        let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
        match parts.last_mut() {
            Some(p) if p.hit == hit => {
                p.text.push(' ');
                p.text.push_str(&t);
            }
            _ => parts.push(Part { text: t, hit }),
        }
    };
    if from > 0 {
        push("…", false);
    }
    let mut pos = from;
    for &(s, e) in words.iter().filter(|&&(s, e)| s >= from && e <= to) {
        if is_hit(&(s, e)) {
            push(&text[pos..s], false);
            push(&text[s..e], true);
            pos = e;
        }
    }
    push(&text[pos..to], false);
    if to < text.len() {
        push("…", false);
    }
    // Re-insert the spaces lost at hit boundaries.
    let mut out: Vec<Part> = Vec::new();
    for (i, mut p) in parts.into_iter().enumerate() {
        if i > 0 && !p.text.starts_with(['.', ',', ';', ':', '!', '?', ')']) {
            p.text.insert(0, ' ');
        }
        out.push(p);
    }
    out
}

/// Reciprocal rank fusion of ranked chunk-id lists.
pub fn fuse(lists: &[&[i64]]) -> Vec<(i64, f64)> {
    let mut score: HashMap<i64, f64> = HashMap::new();
    for list in lists {
        for (rank, id) in list.iter().enumerate() {
            *score.entry(*id).or_default() += 1.0 / (60.0 + rank as f64 + 1.0);
        }
    }
    let mut v: Vec<(i64, f64)> = score.into_iter().collect();
    v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal).then(a.0.cmp(&b.0)));
    v
}

fn word_hits(conn: &Connection, terms: &[String], limit: usize) -> Result<Vec<i64>, AppError> {
    for all in [true, false] {
        let Some(q) = fts_query(terms, all) else { return Ok(vec![]) };
        let mut st = conn
            .prepare("SELECT rowid FROM chunks_fts WHERE chunks_fts MATCH ?1 ORDER BY bm25(chunks_fts) LIMIT ?2")
            .map_err(db_err)?;
        let ids: Vec<i64> = st.query_map(params![q, limit as i64], |r| r.get(0)).map_err(db_err)?.collect::<Result<_, _>>().map_err(db_err)?;
        if !ids.is_empty() {
            return Ok(ids);
        }
    }
    Ok(vec![])
}

pub fn meaning_hits(conn: &Connection, query_vec: &[f32], limit: usize) -> Result<Vec<i64>, AppError> {
    let mut st = conn.prepare("SELECT chunk_id, vec FROM vectors WHERE model = ?1").map_err(db_err)?;
    let mut scored: Vec<(i64, f32)> = st
        .query_map([EMBED_MODEL], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)))
        .map_err(db_err)?
        .filter_map(|r| r.ok())
        .map(|(id, b)| (id, dot(query_vec, &from_blob(&b))))
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let Some(&(_, median)) = scored.get(scored.len() / 2) else { return Ok(vec![]) };
    Ok(scored.into_iter().take_while(|&(_, s)| s >= median + MEANING_MARGIN).take(limit).map(|(id, _)| id).collect())
}

fn vector_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM vectors WHERE model = ?1", [EMBED_MODEL], |r| r.get(0)).unwrap_or(0)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Passage {
    pub chunk_id: i64,
    pub page: Option<i64>,
    pub snippet: Vec<Part>,
    pub matched_by: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    pub path: String,
    pub name: String,
    pub folder: String,
    pub kind: String,
    pub passages: Vec<Passage>,
}

pub struct Ranked {
    pub ids: Vec<i64>,
    pub words: HashSet<i64>,
    pub meaning: HashSet<i64>,
}

/// Word and (when available) meaning results, fused into one ranking.
pub fn rank(conn: &Connection, query: &str, query_vec: Option<&[f32]>, limit: usize) -> Result<Ranked, AppError> {
    let terms = query_terms(query);
    let words = word_hits(conn, &terms, limit)?;
    let meaning = match query_vec {
        Some(v) => meaning_hits(conn, v, limit)?,
        None => vec![],
    };
    let ids = fuse(&[&words, &meaning]).into_iter().map(|(id, _)| id).collect();
    Ok(Ranked { ids, words: words.into_iter().collect(), meaning: meaning.into_iter().collect() })
}

/// Groups ranked passages by file (best file first), up to 3 passages each.
pub fn hits(conn: &Connection, query: &str, ranked: &Ranked, max_files: usize) -> Result<Vec<Hit>, AppError> {
    let terms = query_terms(query);
    let mut out: Vec<Hit> = Vec::new();
    let mut by_path: HashMap<String, usize> = HashMap::new();
    let mut st = conn
        .prepare(
            "SELECT f.path, f.rel, f.kind, r.path, c.page, c.text FROM chunks c
             JOIN files f ON f.id = c.file_id JOIN roots r ON r.id = f.root_id WHERE c.id = ?1",
        )
        .map_err(db_err)?;
    for id in &ranked.ids {
        let Some((path, rel, kind, root, page, text)) = st
            .query_row([id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, Option<i64>>(4)?, r.get::<_, String>(5)?)))
            .optional()
            .map_err(db_err)?
        else {
            continue;
        };
        let matched_by = match (ranked.words.contains(id), ranked.meaning.contains(id)) {
            (true, true) => "both",
            (true, false) => "words",
            _ => "meaning",
        };
        let passage = Passage { chunk_id: *id, page, snippet: snippet(&text, &terms, 240), matched_by };
        match by_path.get(&path) {
            Some(&i) if out[i].passages.len() < 3 => out[i].passages.push(passage),
            Some(_) => {}
            None if out.len() < max_files => {
                let folder = Path::new(&root).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(root.clone());
                by_path.insert(path.clone(), out.len());
                out.push(Hit { path, name: rel, folder, kind, passages: vec![passage] });
            }
            None => {}
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RootInfo {
    path: String,
    name: String,
    files: i64,
    failed: i64,
    passages: i64,
    indexed_at: Option<i64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexStatus {
    roots: Vec<RootInfo>,
    passages: i64,
    vectors: i64,
    ollama: bool,
    embed_model_installed: bool,
    embed_model: &'static str,
    running: bool,
}

#[tauri::command]
pub fn search_status(app: AppHandle, state: State<AppState>) -> Result<IndexStatus, AppError> {
    let conn = open(&app)?;
    let mut st = conn
        .prepare(
            "SELECT r.path, r.indexed_at,
                    (SELECT COUNT(*) FROM files f WHERE f.root_id = r.id),
                    (SELECT COUNT(*) FROM files f WHERE f.root_id = r.id AND f.error IS NOT NULL),
                    (SELECT COUNT(*) FROM chunks c JOIN files f ON f.id = c.file_id WHERE f.root_id = r.id)
             FROM roots r ORDER BY r.path",
        )
        .map_err(db_err)?;
    let roots = st
        .query_map([], |r| {
            let path: String = r.get(0)?;
            Ok(RootInfo {
                name: Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(path.clone()),
                path,
                indexed_at: r.get(1)?,
                files: r.get(2)?,
                failed: r.get(3)?,
                passages: r.get(4)?,
            })
        })
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    let passages = conn.query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0)).map_err(db_err)?;
    let models = ollama_models();
    Ok(IndexStatus {
        roots,
        passages,
        vectors: vector_count(&conn),
        ollama: models.is_some(),
        embed_model_installed: models.is_some_and(|ms| ms.iter().any(|m| m == EMBED_MODEL || m == &format!("{EMBED_MODEL}:latest"))),
        embed_model: EMBED_MODEL,
        running: lock(&state.index_cancel).is_some(),
    })
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct IndexProgress {
    phase: &'static str,
    done: usize,
    total: usize,
    current: String,
}

/// Indexes every folder (or one), then embeds new passages if meaning
/// search is on. Only one run at a time.
async fn run_index(app: AppHandle, state: &AppState, only: Option<String>) -> Result<IndexReport, AppError> {
    let cancel = {
        let mut slot = lock(&state.index_cancel);
        if slot.is_some() {
            return Err(AppError::new("index_busy", "MyFile is already indexing. Wait for it to finish or stop it."));
        }
        let flag = Arc::new(AtomicBool::new(false));
        *slot = Some(flag.clone());
        flag
    };
    let db = db_path(&app)?;
    let a = app.clone();
    let result = crate::blocking(move || {
        let conn = open_db(&db)?;
        let roots: Vec<(i64, String)> = {
            let mut st = conn.prepare("SELECT id, path FROM roots ORDER BY path").map_err(db_err)?;
            let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(db_err)?;
            rows.collect::<Result<_, _>>().map_err(db_err)?
        };
        let load = |p: &Path, k: FileKind| crate::load_document(p, k).map(|d| (d.pages, d.has_pages));
        let mut total = IndexReport::default();
        for (id, path) in roots.iter().filter(|(_, p)| only.as_ref().is_none_or(|o| o == p)) {
            let root = PathBuf::from(path);
            if !root.is_dir() {
                continue;
            }
            let mut emit = |done: usize, of: usize, cur: &str| {
                let _ = a.emit("search-index-progress", IndexProgress { phase: "reading", done, total: of, current: cur.to_string() });
            };
            let r = index_root(&conn, *id, &root, &cancel, &load, &mut emit)?;
            total.added += r.added;
            total.updated += r.updated;
            total.removed += r.removed;
            total.unchanged += r.unchanged;
            total.failed += r.failed;
            total.skipped_over_limit += r.skipped_over_limit;
        }
        if vector_count(&conn) > 0 && embed_model_installed() {
            let mut emit = |done: usize, of: usize| {
                let _ = a.emit("search-index-progress", IndexProgress { phase: "meaning", done, total: of, current: String::new() });
            };
            total.embedded = embed_missing(&conn, &cancel, &mut emit)?;
        }
        Ok(total)
    })
    .await;
    *lock(&state.index_cancel) = None;
    result
}

#[tauri::command]
pub async fn add_search_folder(path: String, app: AppHandle, state: State<'_, AppState>) -> Result<IndexReport, AppError> {
    let dir = PathBuf::from(&path);
    let meta = std::fs::symlink_metadata(&dir).map_err(|_| AppError::new("not_a_folder", "That isn't a folder."))?;
    if !meta.is_dir() {
        return Err(AppError::new("not_a_folder", "That isn't a folder."));
    }
    // A drive root or the whole home folder would index far too much.
    if dir.parent().is_none() || app.path().home_dir().ok().is_some_and(|h| h == dir) {
        return Err(AppError::new("folder_too_broad", "Choose a specific folder (like Documents or a class folder), not a whole drive or your home folder."));
    }
    let key = dir.to_string_lossy().into_owned();
    {
        let conn = open(&app)?;
        conn.execute("INSERT OR IGNORE INTO roots (path, added_at) VALUES (?1, ?2)", params![key, now()]).map_err(db_err)?;
    }
    run_index(app, &state, Some(key)).await
}

#[tauri::command]
pub fn remove_search_folder(path: String, app: AppHandle) -> Result<(), AppError> {
    let conn = open(&app)?;
    conn.execute("DELETE FROM roots WHERE path = ?1", [path]).map_err(db_err)?;
    Ok(())
}

#[tauri::command]
pub async fn reindex(app: AppHandle, state: State<'_, AppState>) -> Result<IndexReport, AppError> {
    run_index(app, &state, None).await
}

#[tauri::command]
pub fn stop_indexing(state: State<AppState>) {
    if let Some(f) = lock(&state.index_cancel).as_ref() {
        f.store(true, Ordering::Relaxed);
    }
}

/// Deletes the whole index (the document text MyFile stored for searching).
#[tauri::command]
pub fn delete_search_index(app: AppHandle, state: State<AppState>) -> Result<(), AppError> {
    if lock(&state.index_cancel).is_some() {
        return Err(AppError::new("index_busy", "Stop indexing first."));
    }
    let path = db_path(&app)?;
    for p in [path.clone(), path.with_extension("sqlite-wal"), path.with_extension("sqlite-shm")] {
        if p.exists() {
            std::fs::remove_file(&p).map_err(|e| AppError::new("index_failed", format!("Couldn't delete the index: {e}")))?;
        }
    }
    Ok(())
}

/// Downloads the embedding model through Ollama (with progress), then
/// embeds every indexed passage.
#[tauri::command]
pub async fn enable_meaning_search(app: AppHandle, state: State<'_, AppState>) -> Result<IndexReport, AppError> {
    if ollama_models().is_none() {
        return Err(AppError::new("local_unavailable", "Meaning search uses Ollama, which isn't running. Install or start Ollama (https://ollama.com) and try again."));
    }
    if !embed_model_installed() {
        let a = app.clone();
        crate::blocking(move || {
            let resp = ureq::post(&format!("{OLLAMA}/api/pull"))
                .timeout(Duration::from_secs(60 * 30))
                .send_json(json!({ "model": EMBED_MODEL, "stream": true }))
                .map_err(|e| AppError::new("pull_failed", format!("Ollama couldn't download {EMBED_MODEL}: {e}")))?;
            let reader = std::io::BufReader::new(resp.into_reader());
            use std::io::BufRead;
            for line in reader.lines() {
                let line = line.map_err(|e| AppError::new("pull_failed", format!("The download stopped: {e}")))?;
                let v: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
                if let Some(err) = v["error"].as_str() {
                    return Err(AppError::new("pull_failed", format!("Ollama couldn't download {EMBED_MODEL}: {err}")));
                }
                let (done, total) = (v["completed"].as_u64().unwrap_or(0), v["total"].as_u64().unwrap_or(0));
                let _ = a.emit("search-index-progress", IndexProgress { phase: "download", done: done as usize, total: total as usize, current: v["status"].as_str().unwrap_or("").to_string() });
            }
            Ok(())
        })
        .await?;
        if !embed_model_installed() {
            return Err(AppError::new("pull_failed", format!("Ollama finished but {EMBED_MODEL} isn't installed.")));
        }
    }
    // Same single-run guard as indexing.
    let cancel = {
        let mut slot = lock(&state.index_cancel);
        if slot.is_some() {
            return Err(AppError::new("index_busy", "MyFile is already indexing. Wait for it to finish or stop it."));
        }
        let flag = Arc::new(AtomicBool::new(false));
        *slot = Some(flag.clone());
        flag
    };
    let db = db_path(&app)?;
    let a = app.clone();
    let result = crate::blocking(move || {
        let conn = open_db(&db)?;
        let mut emit = |done: usize, of: usize| {
            let _ = a.emit("search-index-progress", IndexProgress { phase: "meaning", done, total: of, current: String::new() });
        };
        let embedded = embed_missing(&conn, &cancel, &mut emit)?;
        Ok(IndexReport { embedded, ..Default::default() })
    })
    .await;
    *lock(&state.index_cancel) = None;
    result
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    hits: Vec<Hit>,
    /// "words", or "words and meaning" when vectors were used.
    mode: &'static str,
    elapsed_ms: u64,
}

fn query_vector(conn: &Connection, query: &str) -> Option<Vec<f32>> {
    if vector_count(conn) == 0 || !embed_model_installed() {
        return None;
    }
    embed(&[query.to_string()], true).ok()?.into_iter().next()
}

#[tauri::command]
pub async fn search_files(query: String, app: AppHandle) -> Result<SearchResult, AppError> {
    let query = query.trim().to_string();
    if query.is_empty() || query.chars().count() > 300 {
        return Err(AppError::new("bad_query", "Type what you're looking for (up to 300 characters)."));
    }
    let db = db_path(&app)?;
    crate::blocking(move || {
        let started = Instant::now();
        let conn = open_db(&db)?;
        let qv = query_vector(&conn, &query);
        let ranked = rank(&conn, &query, qv.as_deref(), 200)?;
        let hits = hits(&conn, &query, &ranked, 25)?;
        Ok(SearchResult { hits, mode: if qv.is_some() { "words and meaning" } else { "words" }, elapsed_ms: started.elapsed().as_millis() as u64 })
    })
    .await
}

// ---------------------------------------------------------------------------
// Ask your files
// ---------------------------------------------------------------------------

pub struct StoredAsk {
    id: u64,
    question: String,
    docs: Vec<Arc<FolderDoc>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AskSource {
    name: String,
    path: String,
    pages: Vec<i64>,
    passages: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedAsk {
    ask_id: u64,
    sources: Vec<AskSource>,
    chars: usize,
}

/// Picks the passages an answer would use and stores exactly those, so
/// consent (for Claude) covers precisely what gets sent.
#[tauri::command]
pub async fn prepare_ask(question: String, app: AppHandle, state: State<'_, AppState>) -> Result<PreparedAsk, AppError> {
    let question = question.trim().to_string();
    if question.is_empty() || question.chars().count() > crate::ai::MAX_QUESTION_CHARS {
        return Err(AppError::new("bad_question", "Type a question (up to 1,000 characters)."));
    }
    let db = db_path(&app)?;
    let q = question.clone();
    let (docs, sources, chars) = crate::blocking(move || {
        let conn = open_db(&db)?;
        let qv = query_vector(&conn, &q);
        let ranked = rank(&conn, &q, qv.as_deref(), 60)?;
        Ok(gather(&conn, &ranked.ids, ASK_PASSAGES, ASK_CHARS)?)
    })
    .await?;
    if docs.is_empty() {
        return Err(AppError::new("no_results", "Nothing in your indexed folders matches that question."));
    }
    let id = state.new_id();
    *lock(&state.search_ask) = Some(StoredAsk { id, question, docs });
    Ok(PreparedAsk { ask_id: id, sources, chars })
}

/// Turns the top passages into one document per file. Pages keep their
/// real numbers (pages without a chosen passage are left empty), so the
/// answer's page citations can be checked.
pub fn gather(conn: &Connection, ids: &[i64], max_passages: usize, max_chars: usize) -> Result<(Vec<Arc<FolderDoc>>, Vec<AskSource>, usize), AppError> {
    let mut st = conn
        .prepare("SELECT f.path, f.rel, f.has_pages, c.page, c.text FROM chunks c JOIN files f ON f.id = c.file_id WHERE c.id = ?1")
        .map_err(db_err)?;
    let mut order: Vec<String> = Vec::new();
    let mut files: HashMap<String, (String, bool, Vec<(Option<i64>, String)>)> = HashMap::new();
    let (mut used, mut chars) = (0, 0);
    for id in ids {
        if used >= max_passages {
            break;
        }
        let Some((path, rel, has_pages, page, text)) = st
            .query_row([id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)? != 0, r.get::<_, Option<i64>>(3)?, r.get::<_, String>(4)?)))
            .optional()
            .map_err(db_err)?
        else {
            continue;
        };
        let n = text.chars().count();
        if chars + n > max_chars {
            continue;
        }
        chars += n;
        used += 1;
        let e = files.entry(path.clone()).or_insert_with(|| {
            order.push(path.clone());
            (rel, has_pages, Vec::new())
        });
        e.2.push((page, text));
    }
    let mut docs = Vec::new();
    let mut sources = Vec::new();
    // Names must be unique for the model to cite them; files in different folders can share one.
    let mut names: HashSet<String> = HashSet::new();
    for path in order {
        let (rel, has_pages, passages) = files.remove(&path).expect("present");
        let mut name = rel.clone();
        let mut k = 2;
        while !names.insert(name.clone()) {
            name = format!("{rel} ({k})");
            k += 1;
        }
        let pages: Vec<String> = if has_pages {
            let last = passages.iter().filter_map(|(p, _)| *p).max().unwrap_or(1) as usize;
            let mut pages = vec![String::new(); last];
            for (p, t) in &passages {
                let slot = &mut pages[p.unwrap_or(1) as usize - 1];
                if !slot.is_empty() {
                    slot.push_str("\n\n");
                }
                slot.push_str(t);
            }
            pages
        } else {
            vec![passages.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>().join("\n\n")]
        };
        let mut page_list: Vec<i64> = passages.iter().filter_map(|(p, _)| *p).collect();
        page_list.sort();
        page_list.dedup();
        sources.push(AskSource { name: name.clone(), path: path.clone(), pages: page_list, passages: passages.len() });
        docs.push(Arc::new(FolderDoc { name, path: PathBuf::from(&path), pages, has_pages, error: None }));
    }
    Ok((docs, sources, chars))
}

/// The prompt lists only pages with passages; empty pages are skipped.
fn ask_prompt(question: &str, docs: &[Arc<FolderDoc>]) -> String {
    let mut p = format!(
        "Question about the person's files. Below are the passages from their files that best match it; answer only from them.\n<question>\n{question}\n</question>\n\n"
    );
    for d in docs {
        p.push_str(&format!("<document name=\"{}\">\n", d.name.replace('"', "'")));
        for (i, page) in d.pages.iter().enumerate().filter(|(_, t)| !t.is_empty()) {
            if d.has_pages {
                p.push_str(&format!("[Page {}]\n", i + 1));
            }
            p.push_str(page);
            p.push('\n');
        }
        p.push_str("</document>\n\n");
    }
    p
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AskAnswer {
    answer: String,
    found: bool,
    items: Vec<AskItem>,
    notes: Vec<String>,
    /// File paths by source name, so the interface can open a cited file.
    paths: HashMap<String, String>,
    model: String,
    usage: crate::ai::Usage,
    duration_ms: u64,
}

/// Answers the prepared question from exactly the prepared passages. For
/// Claude, calling this with that ask id is the consent: the interface shows
/// the sources first.
#[tauri::command]
pub async fn ask_files(ask_id: u64, provider_choice: String, app: AppHandle, state: State<'_, AppState>) -> Result<AskAnswer, AppError> {
    let (question, docs) = {
        let s = lock(&state.search_ask);
        let a = s.as_ref().filter(|a| a.id == ask_id).ok_or_else(|| AppError::new("stale", "That question was replaced by a newer one. Ask again."))?;
        (a.question.clone(), a.docs.clone())
    };
    let cli = folder::provider(&app, &provider_choice, "MYFILE_MODEL", "sonnet")?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    *lock(&state.ai_cancel) = Some(tx);
    let started = Instant::now();
    let reply = cli.run_with(folder::ask_system_prompt(), folder::ask_schema(), ask_prompt(&question, &docs), rx).await?;
    let (answer, found, items, notes) = folder::validate_ask(reply.json, &docs)?;
    let paths = docs.iter().map(|d| (d.name.clone(), d.path.to_string_lossy().into_owned())).collect();
    Ok(AskAnswer { answer, found, items, notes, paths, model: reply.model, usage: reply.usage, duration_ms: started.elapsed().as_millis() as u64 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_respect_paragraphs_and_size() {
        let para = "Sentence one is here. ".repeat(20); // ~440 chars
        let text = format!("{para}\n\n{para}\n\n{para}\n\nShort end.");
        let chunks = chunk_page(&text);
        assert!(chunks.iter().all(|c| c.chars().count() <= CHUNK_CHARS), "{:?}", chunks.iter().map(|c| c.len()).collect::<Vec<_>>());
        assert_eq!(chunks.len(), 2);
        assert!(chunks[1].ends_with("Short end."));
        let long = "word ".repeat(1_000);
        let parts = chunk_page(&long);
        assert!(parts.len() >= 4 && parts.iter().all(|c| c.chars().count() <= CHUNK_CHARS));
        assert_eq!(parts.join(" ").split_whitespace().count(), 1_000);
    }

    #[test]
    fn user_text_never_becomes_fts_syntax() {
        let t = query_terms(r#"late "policy" OR NEAR(x) -draft* syllabus"#);
        assert_eq!(t, ["late", "policy", "near", "x", "draft", "syllabus"]);
        assert_eq!(fts_query(&t, true).unwrap(), r#""late" "policy" "near" "x" "draft" "syllabus"*"#);
        // Common words are dropped unless that leaves nothing.
        assert_eq!(query_terms("notice to vacate my apartment"), ["notice", "vacate", "apartment"]);
        assert_eq!(query_terms("to be or not"), ["not"]);
        assert_eq!(query_terms("to be"), ["to", "be"]);
        assert_eq!(fts_query(&query_terms("go"), false).unwrap(), r#""go""#);
        assert_eq!(fts_query(&[], true), None);
    }

    #[test]
    fn snippets_mark_query_words() {
        let text = "Intro text. The late policy: late work loses 10% per day. Other stuff follows here.";
        let parts = snippet(text, &query_terms("late policy"), 240);
        let marked: Vec<&str> = parts.iter().filter(|p| p.hit).map(|p| p.text.trim()).collect();
        assert!(marked.contains(&"late") && marked.contains(&"policy"), "{parts:?}");
        let joined: String = parts.iter().map(|p| p.text.as_str()).collect();
        assert!(joined.contains("loses 10% per day"), "{joined}");
        // Prefix matching: "deadlines" marks "deadline".
        let p = snippet("The deadline is Friday.", &query_terms("deadlines"), 240);
        assert!(p.iter().any(|x| x.hit && x.text.trim() == "deadline"), "{p:?}");
    }

    #[test]
    fn fusion_rewards_agreement() {
        let words = [1, 2, 3];
        let meaning = [3, 4, 1];
        let fused = fuse(&[&words, &meaning]);
        assert_eq!(fused[0].0, 1);
        assert_eq!(fused[1].0, 3);
        assert_eq!(fused.len(), 4);
    }

    fn stub(p: &Path, _: FileKind) -> Result<(Vec<String>, bool), AppError> {
        let text = std::fs::read_to_string(p).unwrap();
        Ok((text.split("\u{c}").map(String::from).collect(), text.contains('\u{c}')))
    }

    fn setup() -> (tempfile::TempDir, Connection, i64) {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_db(&dir.path().join("idx.sqlite")).unwrap();
        let root = dir.path().join("docs");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::create_dir_all(root.join("node_modules")).unwrap();
        std::fs::create_dir_all(root.join(".hidden")).unwrap();
        std::fs::write(root.join("bio.txt"), "Biology 200. Late lab reports are not accepted.").unwrap();
        std::fs::write(root.join("sub/cs.md"), "CS 101. Projects lose 10% per day late, up to three days.").unwrap();
        // Pages separated by form feeds in the stub loader.
        std::fs::write(root.join("sub/lease.txt"), "Lease page one.\u{c}The tenant must give written notice by March 31.").unwrap();
        std::fs::write(root.join("node_modules/junk.txt"), "late late late").unwrap();
        std::fs::write(root.join(".hidden/secret.txt"), "late secret").unwrap();
        std::fs::write(root.join("photo.png"), b"\x89PNG\r\n\x1a\n").unwrap();
        conn.execute("INSERT INTO roots (path, added_at) VALUES (?1, 0)", [root.to_string_lossy()]).unwrap();
        (dir, conn, 1)
    }

    fn names(conn: &Connection, q: &str) -> Vec<String> {
        let r = rank(conn, q, None, 50).unwrap();
        hits(conn, q, &r, 10).unwrap().into_iter().map(|h| h.name).collect()
    }

    #[test]
    fn indexes_searches_and_keeps_up_with_changes() {
        let (dir, conn, root_id) = setup();
        let root = dir.path().join("docs");
        let cancel = AtomicBool::new(false);
        let r = index_root(&conn, root_id, &root, &cancel, &stub, &mut |_, _, _| {}).unwrap();
        assert_eq!((r.added, r.updated, r.removed, r.unchanged), (3, 0, 0, 0));

        // Stemming: "accepting" finds "accepted"; skipped folders aren't indexed.
        assert_eq!(names(&conn, "late"), ["bio.txt", "sub/cs.md"]);
        assert_eq!(names(&conn, "accepting"), ["bio.txt"]);
        // AND first, then OR when nothing has every word.
        assert_eq!(names(&conn, "tenant notice"), ["sub/lease.txt"]);
        let mut either = names(&conn, "tenant biology");
        either.sort();
        assert_eq!(either, ["bio.txt", "sub/lease.txt"]);
        // Pages are kept.
        let rk = rank(&conn, "notice", None, 10).unwrap();
        assert_eq!(hits(&conn, "notice", &rk, 10).unwrap()[0].passages[0].page, Some(2));

        // Unchanged files are skipped; edits and deletions are picked up.
        let r = index_root(&conn, root_id, &root, &cancel, &stub, &mut |_, _, _| {}).unwrap();
        assert_eq!((r.added, r.updated, r.unchanged), (0, 0, 3));
        std::thread::sleep(Duration::from_millis(1100));
        std::fs::write(root.join("bio.txt"), "Biology 200. Field trip on Friday.").unwrap();
        std::fs::remove_file(root.join("sub/cs.md")).unwrap();
        let r = index_root(&conn, root_id, &root, &cancel, &stub, &mut |_, _, _| {}).unwrap();
        assert_eq!((r.updated, r.removed, r.unchanged), (1, 1, 1));
        assert!(names(&conn, "late").is_empty());
        assert_eq!(names(&conn, "field trip"), ["bio.txt"]);

        // Removing the folder removes everything, including the FTS rows.
        conn.execute("DELETE FROM roots WHERE id = ?1", [root_id]).unwrap();
        assert!(names(&conn, "field").is_empty());
        let fts_rows: i64 = conn.query_row("SELECT COUNT(*) FROM chunks_fts WHERE chunks_fts MATCH 'field'", [], |r| r.get(0)).unwrap();
        assert_eq!(fts_rows, 0);
    }

    #[test]
    fn meaning_vectors_rank_by_similarity_and_gather_keeps_real_pages() {
        let (dir, conn, root_id) = setup();
        let root = dir.path().join("docs");
        index_root(&conn, root_id, &root, &AtomicBool::new(false), &stub, &mut |_, _, _| {}).unwrap();
        // Hand-made 3-d "embeddings": lease passage points along x.
        let ids: Vec<(i64, String)> = {
            let mut st = conn.prepare("SELECT id, text FROM chunks").unwrap();
            st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(|r| r.unwrap()).collect()
        };
        for (id, text) in &ids {
            let v = if text.contains("tenant") { vec![1.0, 0.1, 0.0] } else { vec![0.0, 1.0, 0.2] };
            conn.execute("INSERT INTO vectors (chunk_id, model, vec) VALUES (?1, ?2, ?3)", params![id, EMBED_MODEL, to_blob(&normalize(v))]).unwrap();
        }
        let q = normalize(vec![0.9, 0.0, 0.1]);
        let top = meaning_hits(&conn, &q, 1).unwrap()[0];
        assert!(ids.iter().any(|(id, t)| *id == top && t.contains("tenant")));

        let ranked = rank(&conn, "moving out notice", Some(&q), 20).unwrap();
        // Only the best passage: page 2 of the lease, so page 1 stays empty.
        let (docs, sources, chars) = gather(&conn, &ranked.ids, 1, 10_000).unwrap();
        let lease = docs.iter().find(|d| d.name == "sub/lease.txt").unwrap();
        assert!(lease.has_pages && lease.pages.len() == 2 && lease.pages[0].is_empty() && lease.pages[1].contains("March 31"));
        assert!(sources.iter().any(|s| s.name == "sub/lease.txt" && s.pages == vec![2]));
        assert!(chars > 0);
        assert!(ask_prompt("q", &docs).contains("[Page 2]") && !ask_prompt("q", &docs).contains("[Page 1]"));
    }
}
