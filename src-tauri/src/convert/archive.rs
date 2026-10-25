//! Archives: extract ZIP, TAR, TAR.GZ and .gz files, and repack between
//! ZIP and TAR.GZ. Entries are written only inside the output folder, links
//! are skipped, and total size and entry count are capped (zip bombs).

use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use super::Job;
use crate::error::AppError;

const MAX_ENTRIES: usize = 20_000;
const MAX_TOTAL_BYTES: u64 = 8 * 1024 * 1024 * 1024;

fn fail(e: impl std::fmt::Display) -> AppError {
    AppError::new("convert_failed", format!("The archive couldn't be processed: {e}"))
}

/// A relative path with only normal parts (no "..", no root, no drive).
pub fn safe_relative(p: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

/// One file inside an archive, streamed to the callback.
type Visit<'a> = dyn FnMut(&Path, &mut dyn Read) -> Result<(), AppError> + 'a;

fn walk_zip(job: &Job, visit: &mut Visit) -> Result<(), AppError> {
    let mut z = zip::ZipArchive::new(std::fs::File::open(job.input).map_err(AppError::from_read)?).map_err(fail)?;
    let len = z.len();
    if len > MAX_ENTRIES {
        return Err(AppError::new("too_many_entries", format!("This archive has {} entries; MyFile handles up to {MAX_ENTRIES}.", z.len())));
    }
    let mut total = 0u64;
    for i in 0..len {
        job.check_cancel()?;
        let mut entry = z.by_index(i).map_err(fail)?;
        if entry.is_dir() || entry.is_symlink() {
            continue;
        }
        let Some(rel) = entry.enclosed_name().and_then(|p| safe_relative(&p)) else { continue };
        total += entry.size();
        if total > MAX_TOTAL_BYTES {
            return Err(AppError::new("too_large", "This archive unpacks to more than 8 GB."));
        }
        visit(&rel, &mut entry)?;
        drop(entry);
        (job.progress)(Some((i + 1) as f64 / len as f64));
    }
    Ok(())
}

fn walk_tar(job: &Job, gz: bool, visit: &mut Visit) -> Result<(), AppError> {
    let file = std::fs::File::open(job.input).map_err(AppError::from_read)?;
    let reader: Box<dyn Read> = if gz { Box::new(flate2::read::GzDecoder::new(file)) } else { Box::new(file) };
    let mut ar = tar::Archive::new(reader);
    let (mut count, mut total) = (0usize, 0u64);
    for entry in ar.entries().map_err(fail)? {
        job.check_cancel()?;
        let mut entry = entry.map_err(fail)?;
        // Regular files only: no links, devices or directories.
        if !entry.header().entry_type().is_file() {
            continue;
        }
        count += 1;
        total += entry.size();
        if count > MAX_ENTRIES || total > MAX_TOTAL_BYTES {
            return Err(AppError::new("too_large", "This archive is too large to process."));
        }
        let Some(rel) = entry.path().ok().and_then(|p| safe_relative(&p)) else { continue };
        visit(&rel, &mut entry)?;
    }
    Ok(())
}

fn walk(job: &Job, visit: &mut Visit) -> Result<(), AppError> {
    match job.format {
        "zip" => walk_zip(job, visit),
        "tar" => walk_tar(job, false, visit),
        "tgz" => walk_tar(job, true, visit),
        f => Err(AppError::new("bad_target", format!("MyFile can't open {} archives.", f.to_uppercase()))),
    }
}

pub fn convert(job: &Job) -> Result<Vec<PathBuf>, AppError> {
    match (job.format, job.target) {
        ("gz", _) => {
            let mut d = flate2::read::GzDecoder::new(std::fs::File::open(job.input).map_err(AppError::from_read)?);
            let out = job.out_dir.join(job.stem);
            let mut f = std::fs::File::create(&out).map_err(fail)?;
            let n = std::io::copy(&mut Read::by_ref(&mut d).take(MAX_TOTAL_BYTES + 1), &mut f).map_err(fail)?;
            if n > MAX_TOTAL_BYTES {
                return Err(AppError::new("too_large", "This file decompresses to more than 8 GB."));
            }
            Ok(vec![out])
        }
        (_, "extract") => {
            // Everything goes under a folder named after the archive.
            let root = job.out_dir.join(job.stem);
            let mut made = Vec::new();
            walk(job, &mut |rel, r| {
                let dest = root.join(rel);
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent).map_err(fail)?;
                }
                let mut f = std::fs::File::create(&dest).map_err(fail)?;
                std::io::copy(r, &mut f).map_err(fail)?;
                made.push(dest);
                Ok(())
            })?;
            Ok(made)
        }
        (_, "zip") => {
            let out = job.out("zip");
            let mut z = zip::ZipWriter::new(std::fs::File::create(&out).map_err(fail)?);
            let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            walk(job, &mut |rel, r| {
                z.start_file(rel.to_string_lossy().replace('\\', "/"), opts).map_err(fail)?;
                std::io::copy(r, &mut z).map_err(fail)?;
                Ok(())
            })?;
            z.finish().map_err(fail)?;
            Ok(vec![out])
        }
        (_, "tgz") => {
            let out = job.out_dir.join(format!("{}.tar.gz", job.stem));
            let gz = flate2::write::GzEncoder::new(std::fs::File::create(&out).map_err(fail)?, flate2::Compression::default());
            let mut b = tar::Builder::new(gz);
            walk(job, &mut |rel, r| {
                let mut data = Vec::new();
                r.read_to_end(&mut data).map_err(fail)?;
                let mut h = tar::Header::new_gnu();
                h.set_size(data.len() as u64);
                h.set_mode(0o644);
                h.set_cksum();
                b.append_data(&mut h, rel, data.as_slice()).map_err(fail)?;
                Ok(())
            })?;
            b.into_inner().map_err(fail)?.finish().map_err(fail)?.flush().map_err(fail)?;
            Ok(vec![out])
        }
        _ => Err(AppError::new("bad_target", "MyFile can't convert this archive to that.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_relative_paths_are_allowed() {
        assert_eq!(safe_relative(Path::new("a/b.txt")), Some(PathBuf::from("a/b.txt")));
        assert_eq!(safe_relative(Path::new("./a.txt")), Some(PathBuf::from("a.txt")));
        assert_eq!(safe_relative(Path::new("../evil.txt")), None);
        assert_eq!(safe_relative(Path::new("a/../../evil.txt")), None);
        assert_eq!(safe_relative(Path::new("/etc/passwd")), None);
        #[cfg(windows)]
        assert_eq!(safe_relative(Path::new("C:\\Windows\\x")), None);
    }
}
