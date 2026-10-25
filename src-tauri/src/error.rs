use serde::Serialize;

/// An error that is safe to show to the user.
///
/// `code` is a stable identifier the frontend can branch on; `message` is
/// plain-English text explaining what went wrong and, where possible, what
/// the user can do about it.
#[derive(Debug, Serialize)]
pub struct AppError {
    pub code: &'static str,
    pub message: String,
}

impl AppError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }

    /// Translates an I/O error from reading the user's file into a friendly message.
    pub fn from_read(err: std::io::Error) -> Self {
        use std::io::ErrorKind::*;
        match err.kind() {
            NotFound => Self::new(
                "not_found",
                "That file no longer exists. It may have been moved, renamed, or deleted.",
            ),
            PermissionDenied => Self::new(
                "permission_denied",
                "MyFile doesn't have permission to read that file.",
            ),
            _ => Self::new("unreadable", format!("MyFile couldn't read that file ({err}).")),
        }
    }
}

/// Formats a byte count the way a person would say it, e.g. "2.4 MB".
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} bytes");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}
