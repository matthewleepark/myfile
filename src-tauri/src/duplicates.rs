//! Duplicate detection for an open folder. All code, no model: nothing here
//! is sent anywhere, and MyFile never deletes or moves a file. It only lists
//! what it found so the person can decide.
//!
//! Four kinds of match, each file reported at most once per kind:
//! - identical: the same bytes,
//! - same text: different files whose text is word-for-word the same
//!   (a re-saved PDF, a .txt copy),
//! - versions: mostly the same text, with the real differences and any
//!   moved dates listed by the diff engine,
//! - similar images: pictures that look the same (resized or re-saved).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::UNIX_EPOCH;

use serde::Serialize;
use tauri::State;

use crate::error::AppError;
use crate::{compare, lock, monitor, AppState};

/// Versions are found by overlapping runs of this many words.
const SHINGLE: usize = 3;
/// Documents with fewer word runs than this are too short to judge.
const MIN_SHINGLES: usize = 20;
/// Share of word runs two documents must have in common to count as versions.
/// On the samples, real versions share 0.43 and unrelated syllabi at most 0.03.
const VERSION_SIMILARITY: f64 = 0.25;
/// Max differing bits (of 64) for two image fingerprints to count as alike.
const IMAGE_DISTANCE: u32 = 6;

pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    /// Document text by page; None for images.
    pub pages: Option<Vec<String>>,
    pub has_pages: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum GroupKind {
    Identical,
    SameText,
    Versions,
    SimilarImages,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DupFile {
    pub name: String,
    pub size_bytes: u64,
    /// Seconds since the Unix epoch.
    pub modified: Option<u64>,
    pub newest: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub kind: GroupKind,
    pub files: Vec<DupFile>,
    /// Percent alike, for versions and images.
    pub similarity: Option<u8>,
    /// Written by code.
    pub detail: String,
    /// Dates that differ between two versions, e.g. "Nov 12, 2026 → Nov 17, 2026".
    pub date_changes: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub groups: Vec<Group>,
    pub files_checked: usize,
    /// Bytes that identical copies take up beyond the first of each group.
    pub reclaimable_bytes: u64,
}

fn fnv(bytes: impl IntoIterator<Item = u8>) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

fn stamp(path: &std::path::Path) -> (u64, Option<u64>) {
    match std::fs::metadata(path) {
        Ok(m) => (m.len(), m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs())),
        Err(_) => (0, None),
    }
}

/// Files of a group, oldest first, with the newest marked.
fn files_of(entries: &[Entry], idx: &[usize]) -> Vec<DupFile> {
    let mut files: Vec<DupFile> = idx
        .iter()
        .map(|&i| {
            let (size_bytes, modified) = stamp(&entries[i].path);
            DupFile { name: entries[i].name.clone(), size_bytes, modified, newest: false }
        })
        .collect();
    files.sort_by(|a, b| a.modified.cmp(&b.modified).then_with(|| a.name.cmp(&b.name)));
    if files.iter().all(|f| f.modified.is_some()) && files.first().map(|f| f.modified) != files.last().map(|f| f.modified) {
        if let Some(last) = files.last_mut() {
            last.newest = true;
        }
    }
    files
}

/// Groups of entries with byte-for-byte identical contents.
fn identical(entries: &[Entry]) -> Vec<Vec<usize>> {
    let mut by_size: HashMap<u64, Vec<usize>> = HashMap::new();
    for (i, e) in entries.iter().enumerate() {
        if let Ok(m) = std::fs::metadata(&e.path) {
            by_size.entry(m.len()).or_default().push(i);
        }
    }
    let mut groups = Vec::new();
    for (_, same_size) in by_size.into_iter().filter(|(_, v)| v.len() > 1) {
        // Compare the actual bytes; the hash only buckets them.
        let mut buckets: Vec<(u64, Vec<u8>, Vec<usize>)> = Vec::new();
        for i in same_size {
            let Ok(bytes) = std::fs::read(&entries[i].path) else { continue };
            let h = fnv(bytes.iter().copied());
            match buckets.iter_mut().find(|(bh, b, _)| *bh == h && *b == bytes) {
                Some((_, _, members)) => members.push(i),
                None => buckets.push((h, bytes, vec![i])),
            }
        }
        groups.extend(buckets.into_iter().map(|(_, _, m)| m).filter(|m| m.len() > 1));
    }
    groups.sort();
    groups
}

fn shingles(pages: &[String]) -> HashSet<u64> {
    let words: Vec<String> = pages
        .iter()
        .flat_map(|p| p.split_whitespace())
        .map(|w| w.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>())
        .filter(|w| !w.is_empty())
        .collect();
    words.windows(SHINGLE).map(|w| fnv(w.join(" ").bytes())).collect()
}

fn jaccard(a: &HashSet<u64>, b: &HashSet<u64>) -> f64 {
    let inter = a.intersection(b).count();
    let union = a.len() + b.len() - inter;
    if union == 0 {
        0.0
    } else {
        inter as f64 / union as f64
    }
}

/// A 64-bit "difference hash": which neighboring pixels get brighter, on a
/// 9x8 grayscale thumbnail. Resizing or re-saving barely changes it.
fn image_hash(path: &std::path::Path) -> Option<u64> {
    let img = image::open(path).ok()?.grayscale();
    let small = img.resize_exact(9, 8, image::imageops::FilterType::Triangle).to_luma8();
    let mut h = 0u64;
    for y in 0..8 {
        for x in 0..8 {
            h <<= 1;
            if small.get_pixel(x, y)[0] < small.get_pixel(x + 1, y)[0] {
                h |= 1;
            }
        }
    }
    Some(h)
}

/// Later passes compare only the oldest file of a group (most likely the original).
fn skip_all_but_oldest(entries: &[Entry], group: &[usize], files: &[DupFile], skip: &mut HashSet<usize>) {
    let keep = group.iter().copied().find(|&i| entries[i].name == files[0].name);
    skip.extend(group.iter().copied().filter(|&i| Some(i) != keep));
}

pub fn find(entries: &[Entry]) -> Report {
    let mut groups = Vec::new();
    let mut reclaimable_bytes = 0;
    // Later passes only look at one representative per identical group.
    let mut skip = HashSet::new();
    for g in identical(entries) {
        let files = files_of(entries, &g);
        let extra: u64 = files.iter().skip(1).map(|f| f.size_bytes).sum();
        reclaimable_bytes += extra;
        skip_all_but_oldest(entries, &g, &files, &mut skip);
        groups.push(Group {
            kind: GroupKind::Identical,
            detail: "Byte-for-byte identical copies.".into(),
            files,
            similarity: None,
            date_changes: vec![],
        });
    }

    // Same text in different files.
    let docs: Vec<usize> = (0..entries.len())
        .filter(|i| !skip.contains(i))
        .filter(|&i| entries[i].pages.as_ref().is_some_and(|p| p.iter().any(|t| !t.trim().is_empty())))
        .collect();
    let mut by_text: HashMap<String, Vec<usize>> = HashMap::new();
    for &i in &docs {
        by_text.entry(monitor::fingerprint(entries[i].pages.as_deref().unwrap_or_default())).or_default().push(i);
    }
    let mut same_text: Vec<Vec<usize>> = by_text.into_values().filter(|v| v.len() > 1).collect();
    same_text.sort();
    for g in same_text {
        let files = files_of(entries, &g);
        skip_all_but_oldest(entries, &g, &files, &mut skip);
        groups.push(Group {
            kind: GroupKind::SameText,
            files,
            similarity: Some(100),
            detail: "Different files with word-for-word the same text, like a re-saved PDF or a text copy.".into(),
            date_changes: vec![],
        });
    }

    // Versions: mostly the same text.
    let reps: Vec<(usize, HashSet<u64>)> = docs
        .iter()
        .filter(|i| !skip.contains(i))
        .map(|&i| (i, shingles(entries[i].pages.as_deref().unwrap_or_default())))
        .filter(|(_, s)| s.len() >= MIN_SHINGLES)
        .collect();
    for a in 0..reps.len() {
        for b in a + 1..reps.len() {
            let sim = jaccard(&reps[a].1, &reps[b].1);
            if sim < VERSION_SIMILARITY {
                continue;
            }
            let files = files_of(entries, &[reps[a].0, reps[b].0]);
            // Diff oldest to newest so "moved" dates read the right way.
            let (old, new) = if stamp(&entries[reps[b].0].path).1 < stamp(&entries[reps[a].0].path).1 {
                (reps[b].0, reps[a].0)
            } else {
                (reps[a].0, reps[b].0)
            };
            let s = |i: usize| compare::sentences(entries[i].pages.as_deref().unwrap_or_default(), entries[i].has_pages);
            let d = compare::diff(&s(old), &s(new));
            let date_changes: Vec<String> = d.changes.iter().filter_map(|c| c.date_change.clone()).collect();
            let text = |i: usize| entries[i].pages.as_deref().unwrap_or_default().join(" ");
            let (old_text, new_text) = (text(old), text(new));
            let alike = similar::TextDiff::from_words(old_text.as_str(), new_text.as_str()).ratio();
            groups.push(Group {
                kind: GroupKind::Versions,
                detail: format!(
                    "Probably two versions of one document: {} sentences unchanged, {} changed, {} added, {} removed.",
                    d.unchanged, d.changed, d.added, d.removed
                ),
                files,
                // Same-text pairs were caught above, so never claim 100%.
                similarity: Some(((alike * 100.0).round() as u8).min(99)),
                date_changes,
            });
        }
    }

    // Images that look alike.
    let pics: Vec<(usize, u64)> = (0..entries.len())
        .filter(|i| !skip.contains(i) && entries[*i].pages.is_none())
        .filter_map(|i| image_hash(&entries[i].path).map(|h| (i, h)))
        .collect();
    let mut taken = HashSet::new();
    for a in 0..pics.len() {
        if taken.contains(&a) {
            continue;
        }
        let mut members = vec![pics[a].0];
        let mut worst = 0;
        for b in a + 1..pics.len() {
            let dist = (pics[a].1 ^ pics[b].1).count_ones();
            if !taken.contains(&b) && dist <= IMAGE_DISTANCE {
                taken.insert(b);
                members.push(pics[b].0);
                worst = worst.max(dist);
            }
        }
        if members.len() > 1 {
            groups.push(Group {
                kind: GroupKind::SimilarImages,
                files: files_of(entries, &members),
                similarity: Some((100 - worst * 100 / 64) as u8),
                detail: "These pictures look the same; one may be a resized or re-saved copy.".into(),
                date_changes: vec![],
            });
        }
    }

    Report { groups, files_checked: entries.len(), reclaimable_bytes }
}

#[tauri::command]
pub async fn find_duplicates(selection_id: u64, state: State<'_, AppState>) -> Result<Report, AppError> {
    let entries: Vec<Entry> = {
        let folder = lock(&state.folder);
        let f = folder
            .as_ref()
            .filter(|f| f.id == selection_id)
            .ok_or_else(|| AppError::new("stale_selection", "That folder is no longer open."))?;
        f.docs
            .iter()
            .map(|d| Entry {
                name: d.name.clone(),
                path: d.path.clone(),
                pages: Some(d.pages.clone()),
                has_pages: d.has_pages,
            })
            .chain(f.images.iter().map(|i| Entry { name: i.name.clone(), path: i.path.clone(), pages: None, has_pages: false }))
            .collect()
    };
    crate::blocking(move || Ok(find(&entries))).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(dir: &std::path::Path, name: &str, text: &str) -> Entry {
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        Entry { name: name.into(), path, pages: Some(vec![text.into()]), has_pages: false }
    }

    fn long(seed: &str) -> String {
        (0..60).map(|i| format!("{seed} sentence number {i} talks about the course.")).collect::<Vec<_>>().join(" ")
    }

    fn unrelated() -> String {
        (0..60).map(|i| format!("Week {i} of the chemistry lab measures pH and records results.")).collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn finds_identical_files_and_reclaimable_space() {
        let dir = tempfile::tempdir().unwrap();
        let text = long("Alpha");
        let entries = vec![doc(dir.path(), "a.txt", &text), doc(dir.path(), "b.txt", "unrelated"), doc(dir.path(), "a copy.txt", &text)];
        let r = find(&entries);
        assert_eq!(r.groups.len(), 1);
        assert_eq!(r.groups[0].kind, GroupKind::Identical);
        assert_eq!(r.groups[0].files.len(), 2);
        assert_eq!(r.reclaimable_bytes, text.len() as u64);
    }

    #[test]
    fn same_text_with_different_wrapping_is_same_text_not_identical() {
        let dir = tempfile::tempdir().unwrap();
        let text = long("Beta");
        let entries = vec![doc(dir.path(), "x.txt", &text), doc(dir.path(), "y.txt", &text.replace(". ", ".\n\n"))];
        let r = find(&entries);
        assert_eq!(r.groups.len(), 1);
        assert_eq!(r.groups[0].kind, GroupKind::SameText);
    }

    #[test]
    fn versions_report_moved_dates() {
        let dir = tempfile::tempdir().unwrap();
        let base = long("Gamma");
        let v1 = format!("{base} The essay is due Nov 12, 2026.");
        let v2 = format!("{base} The essay is due Nov 17, 2026.");
        let entries = vec![doc(dir.path(), "v1.txt", &v1), doc(dir.path(), "v2.txt", &v2), doc(dir.path(), "other.txt", &unrelated())];
        let r = find(&entries);
        assert_eq!(r.groups.len(), 1, "{:?}", r.groups);
        let g = &r.groups[0];
        assert_eq!(g.kind, GroupKind::Versions);
        assert!(g.similarity.unwrap() >= 90);
        assert_eq!(g.date_changes.len(), 1);
        assert!(g.date_changes[0].contains("Nov 12") && g.date_changes[0].contains("Nov 17"), "{:?}", g.date_changes);
    }

    #[test]
    fn short_or_different_documents_are_not_flagged() {
        let dir = tempfile::tempdir().unwrap();
        let entries = vec![doc(dir.path(), "a.txt", "Due Friday."), doc(dir.path(), "b.txt", "Due Monday."), doc(dir.path(), "c.txt", &long("Eps"))];
        assert!(find(&entries).groups.is_empty());
    }

    #[test]
    fn resized_image_is_similar_but_a_different_picture_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let pic = image::RgbImage::from_fn(200, 160, |x, y| image::Rgb([(x + y) as u8, (x * 2 % 255) as u8, (y * 3 % 255) as u8]));
        let other = image::RgbImage::from_fn(200, 160, |x, y| image::Rgb([255 - ((x * y) % 255) as u8, 30, (x % 7 * 30) as u8]));
        pic.save(dir.path().join("photo.png")).unwrap();
        image::imageops::resize(&pic, 100, 80, image::imageops::FilterType::Triangle).save(dir.path().join("photo small.jpg")).unwrap();
        other.save(dir.path().join("other.png")).unwrap();
        let img = |n: &str| Entry { name: n.into(), path: dir.path().join(n), pages: None, has_pages: false };
        let r = find(&[img("photo.png"), img("other.png"), img("photo small.jpg")]);
        assert_eq!(r.groups.len(), 1, "{:?}", r.groups);
        assert_eq!(r.groups[0].kind, GroupKind::SimilarImages);
        let names: Vec<_> = r.groups[0].files.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"photo.png") && names.contains(&"photo small.jpg"));
    }

    #[test]
    fn sample_syllabus_versions_are_found_and_other_syllabi_are_not() {
        let entry = |rel: &str| {
            let path = std::path::PathBuf::from(format!("../test-files/{rel}"));
            let pages = crate::pdf::extract(&path).unwrap().pages;
            Entry { name: rel.into(), path, pages: Some(pages), has_pages: true }
        };
        let r = find(&[
            entry("versions/ENGL-201-syllabus-v1.pdf"),
            entry("semester/BIO-200-syllabus.pdf"),
            entry("versions/ENGL-201-syllabus-v2.pdf"),
            entry("semester/CS-101-syllabus.pdf"),
            entry("semester/HIST-150-syllabus.pdf"),
        ]);
        assert_eq!(r.groups.len(), 1, "{:?}", r.groups);
        assert_eq!(r.groups[0].kind, GroupKind::Versions);
        assert!(r.groups[0].date_changes.iter().any(|d| d.contains("Nov 12") && d.contains("Nov 17")), "{:?}", r.groups[0].date_changes);
    }
}
