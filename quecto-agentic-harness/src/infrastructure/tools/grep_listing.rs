//! Files and count output for the grep tool (#2136): rg `-l` / `-c` with
//! `--null`, so a path holding `:` or spaces is read whole. Files are
//! listed in path order; counts busiest first, then by path. Both are
//! bounded by the match limit (here: files) and the output cap.

use std::path::Path;

use super::grep_request::OutputMode;
use crate::infrastructure::security::sandbox::Sandbox;
use crate::infrastructure::tools::truncate::format_size;

/// One matching file, with its match count in count mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ListedFile {
    pub path: String,
    pub count: Option<u64>,
}

/// Parse rg's `--null` listing: `path\0` per file (`-l`), or
/// `path\0count\n` per file (`--count-matches`). Only complete records are
/// read: a tail cut off by the output cap is dropped, and a path may hold a
/// newline or a colon.
pub(super) fn parse_listing(stdout: &str, mode: OutputMode) -> Vec<ListedFile> {
    match mode {
        OutputMode::Files => {
            let mut records: Vec<&str> = stdout.split('\0').collect();
            // The piece after the last terminator is incomplete (or empty).
            records.pop();
            records
                .into_iter()
                .filter(|path| !path.is_empty())
                .map(|path| ListedFile {
                    path: path.to_string(),
                    count: None,
                })
                .collect()
        }
        OutputMode::Count => {
            let mut files = Vec::new();
            let mut rest = stdout;
            while let Some((path, after)) = rest.split_once('\0') {
                let Some((count, next)) = after.split_once('\n') else {
                    break;
                };
                rest = next;
                match count.trim().parse::<u64>() {
                    Ok(count) if !path.is_empty() => files.push(ListedFile {
                        path: path.to_string(),
                        count: Some(count),
                    }),
                    Ok(_) | Err(_) => {}
                }
            }
            files
        }
        OutputMode::Content => Vec::new(),
    }
}

pub(super) struct ListingFormat<'a> {
    pub workspace: &'a Path,
    pub sandbox: &'a Sandbox,
    pub limit: usize,
    pub max_output_bytes: usize,
}

/// Format a listing: workspace-relative paths, sandbox-checked, ordered,
/// capped at `limit` files and `max_output_bytes`, with the notices.
pub(super) fn format_listing(mut files: Vec<ListedFile>, f: &ListingFormat<'_>) -> String {
    files.retain(|file| f.sandbox.validate_path(&file.path).is_ok());
    files.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.path.cmp(&b.path)));
    if files.is_empty() {
        return "No matches found".to_string();
    }
    let total = files.len();
    let ws = f.workspace.to_string_lossy();
    let ws_slash = format!("{ws}/");
    let mut lines = Vec::new();
    let mut bytes = 0;
    let mut byte_capped = false;
    for file in files.iter().take(f.limit) {
        let relative = file
            .path
            .strip_prefix(ws_slash.as_str())
            .unwrap_or(&file.path);
        // A search of "." reports `<workspace>/./x`: show `x`.
        let shown = relative.strip_prefix("./").unwrap_or(relative);
        let line = match file.count {
            Some(count) => format!("{shown}: {count}"),
            None => shown.to_string(),
        };
        bytes += line.len() + 1;
        if bytes > f.max_output_bytes {
            byte_capped = true;
            break;
        }
        lines.push(line);
    }
    let mut output = lines.join("\n");
    let mut notices = Vec::new();
    if total > f.limit {
        notices.push(format!(
            "{} of {total} files shown. Use limit={} for more, or refine pattern",
            f.limit,
            f.limit.saturating_mul(2)
        ));
    }
    if byte_capped {
        notices.push(format!("{} limit reached", format_size(f.max_output_bytes)));
    }
    if !notices.is_empty() {
        output.push_str(&format!("\n\n[{}]", notices.join(". ")));
    }
    output
}

#[cfg(test)]
#[path = "grep_listing_tests.rs"]
mod tests;
