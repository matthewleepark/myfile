//! MyFile's MCP server. Claude Code starts it and talks to it over stdio.
//!
//! Usage: myfile-mcp [--root <folder>]
//! Every path the tools touch must be inside the root (default: home folder).

use std::path::PathBuf;

fn main() {
    let mut args = std::env::args().skip(1);
    let mut root: Option<PathBuf> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--root" => root = args.next().map(PathBuf::from),
            "--help" | "-h" => {
                eprintln!("Usage: myfile-mcp [--root <folder>]");
                return;
            }
            other => {
                eprintln!("myfile-mcp: unknown argument {other}");
                std::process::exit(2);
            }
        }
    }
    let root = root
        .or_else(|| std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    if let Err(e) = myfile_lib::run_mcp(root) {
        eprintln!("myfile-mcp: {e}");
        std::process::exit(1);
    }
}
