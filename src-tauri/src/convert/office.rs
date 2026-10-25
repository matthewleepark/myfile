//! Office documents through LibreOffice (headless) and markup through
//! pandoc, when they're installed. Both run with argument lists (no shell),
//! a time limit, and a private LibreOffice profile.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use super::Job;
use crate::error::AppError;

const TIME_LIMIT: Duration = Duration::from_secs(300);

fn find_on_path(names: &[&str], extra: &[PathBuf]) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    dirs.extend(extra.iter().cloned());
    dirs.iter().flat_map(|d| names.iter().map(move |n| d.join(n))).find(|p| p.is_file())
}

pub fn find_soffice() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("MYFILE_SOFFICE").map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(p);
    }
    let mut extra = vec![
        PathBuf::from("/Applications/LibreOffice.app/Contents/MacOS"),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/opt/homebrew/bin"),
    ];
    for var in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(pf) = std::env::var_os(var) {
            extra.push(PathBuf::from(pf).join("LibreOffice").join("program"));
        }
    }
    find_on_path(&["soffice.exe", "soffice.com", "soffice", "libreoffice"], &extra)
}

pub fn find_pandoc() -> Option<PathBuf> {
    let mut extra = vec![PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/usr/local/bin")];
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        extra.push(PathBuf::from(local).join("Pandoc"));
    }
    if let Some(pf) = std::env::var_os("ProgramFiles") {
        extra.push(PathBuf::from(pf).join("Pandoc"));
    }
    find_on_path(&["pandoc.exe", "pandoc"], &extra)
}

/// Runs a tool to completion, honoring cancel and the time limit.
fn run(job: &Job, mut cmd: Command, tool: &str) -> Result<(), AppError> {
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = cmd.spawn().map_err(|e| AppError::new("convert_failed", format!("Couldn't start {tool}: {e}")))?;
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(|e| AppError::new("convert_failed", e.to_string()))? {
            if !status.success() {
                let mut err = String::new();
                if let Some(mut e) = child.stderr.take() {
                    use std::io::Read;
                    let _ = e.read_to_string(&mut err);
                }
                let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("unknown error").trim().to_string();
                return Err(AppError::new("convert_failed", format!("{tool} couldn't convert this file: {last}")));
            }
            return Ok(());
        }
        if job.cancel.load(Ordering::Relaxed) || started.elapsed() > TIME_LIMIT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(if job.cancel.load(Ordering::Relaxed) {
                AppError::new("cancelled", "Conversion cancelled.")
            } else {
                AppError::new("timeout", format!("{tool} took longer than 5 minutes, so MyFile stopped it."))
            });
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

fn file_url(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    if s.starts_with('/') { format!("file://{s}") } else { format!("file:///{s}") }
}

pub fn libreoffice(job: &Job) -> Result<Vec<PathBuf>, AppError> {
    let soffice = job.tools.soffice.as_ref().ok_or_else(|| AppError::new("needs_tool", "This conversion needs LibreOffice."))?;
    let filter = match job.target {
        "docx" => "docx:MS Word 2007 XML",
        "xlsx" => "xlsx:Calc MS Excel 2007 XML",
        "pptx" => "pptx:Impress MS PowerPoint 2007 XML",
        "txt" => "txt:Text (encoded):UTF8",
        "html" => "html",
        t => t,
    };
    // A private profile, so a LibreOffice window that's already open doesn't
    // swallow the request.
    let profile = job.out_dir.join(".lo-profile");
    // LibreOffice names its output after the input file, so convert a copy
    // named after the stem.
    let ext = job.input.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
    let staged_dir = job.out_dir.join(".lo-in");
    std::fs::create_dir_all(&staged_dir).map_err(|e| AppError::new("convert_failed", e.to_string()))?;
    let staged = staged_dir.join(format!("{}.{ext}", job.stem));
    std::fs::copy(job.input, &staged).map_err(AppError::from_read)?;
    let out_dir = job.out_dir.join(".lo-out");
    let mut cmd = Command::new(soffice);
    cmd.arg(format!("-env:UserInstallation={}", file_url(&profile)))
        .args(["--headless", "--norestore", "--nolockcheck", "--nodefault", "--nofirststartwizard"]);
    if job.format == "pdf" {
        cmd.arg("--infilter=writer_pdf_import");
    }
    cmd.args(["--convert-to", filter, "--outdir"]).arg(&out_dir).arg(&staged);
    let result = run(job, cmd, "LibreOffice");
    let _ = std::fs::remove_dir_all(&profile);
    let _ = std::fs::remove_dir_all(&staged_dir);
    result?;
    let made = out_dir.join(format!("{}.{}", job.stem, job.target));
    if !made.is_file() {
        return Err(AppError::new("convert_failed", "LibreOffice finished but didn't produce a file."));
    }
    let dest = job.out(job.target);
    std::fs::rename(&made, &dest).map_err(|e| AppError::new("convert_failed", e.to_string()))?;
    let _ = std::fs::remove_dir_all(&out_dir);
    Ok(vec![dest])
}

pub fn pandoc(job: &Job) -> Result<Vec<PathBuf>, AppError> {
    let pandoc = job.tools.pandoc.as_ref().ok_or_else(|| AppError::new("needs_tool", "This conversion needs pandoc."))?;
    let from = match job.format {
        "md" | "txt" => "markdown",
        "tex" => "latex",
        f => f,
    };
    let to = match job.target {
        "md" => "markdown",
        "tex" => "latex",
        "txt" => "plain",
        t => t,
    };
    let out = job.out(job.target);
    let mut cmd = Command::new(pandoc);
    cmd.arg(job.input).args(["-f", from, "-t", to, "--standalone", "-o"]).arg(&out);
    run(job, cmd, "pandoc")?;
    Ok(vec![out])
}
