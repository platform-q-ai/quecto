//! The global `--config <path>` flag: extracted before dispatch and stripped
//! from the argument vector the subcommands see.

use std::path::PathBuf;

/// Extract `--config <path>` from args (consumed globally).
/// Skips values of flags that take arguments (e.g. `-m`, `--system`) to avoid
/// misinterpreting message text like `-m "--config"` as the flag.
pub(super) fn extract_config_flag(args: &[String]) -> Result<Option<PathBuf>, String> {
    /// Flags that consume the next arg as a value (skip their value during scan).
    const VALUE_FLAGS: &[&str] = &[
        "-m",
        "--message",
        "-s",
        "--session",
        "--system",
        "--model",
        "--max-iterations",
        "--max-time",
        "--mode",
        "--socket",
        "--disable-tool",
    ];
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--config" {
            let value = args
                .get(i + 1)
                .filter(|value| !value.starts_with('-'))
                .ok_or_else(|| "--config requires a path".to_string())?;
            return Ok(Some(PathBuf::from(value)));
        }
        if VALUE_FLAGS.contains(&args[i].as_str()) {
            i += 2; // skip the flag and its value
        } else {
            i += 1;
        }
    }
    Ok(None)
}

pub(super) fn strip_global_config_flag(args: &[String]) -> Vec<String> {
    const VALUE_FLAGS: &[&str] = &[
        "-m",
        "--message",
        "-s",
        "--session",
        "--system",
        "--model",
        "--max-iterations",
        "--max-time",
        "--mode",
        "--socket",
        "--disable-tool",
    ];
    let mut stripped = Vec::with_capacity(args.len());
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--config" && i + 1 < args.len() {
            i += 2;
        } else if VALUE_FLAGS.contains(&args[i].as_str()) && i + 1 < args.len() {
            stripped.push(args[i].clone());
            stripped.push(args[i + 1].clone());
            i += 2;
        } else {
            stripped.push(args[i].clone());
            i += 1;
        }
    }
    stripped
}
