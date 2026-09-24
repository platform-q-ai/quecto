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
/// `path\0count\n` per file (`-c`). A record that does not parse is skipped.
pub(super) fn parse_listing(stdout: &str, mode: OutputMode) -> Vec<ListedFile> {
    match mode {
        OutputMode::Files => stdout
            .split('\0')
            .map(|path| path.trim_start_matches('\n'))
            .filter(|path| !path.is_empty())
            .map(|path| ListedFile {
                path: path.to_string(),
                count: None,
            })
            .collect(),
        OutputMode::Count => stdout
            .lines()
            .filter_map(|line| {
                let (path, count) = line.split_once('\0')?;
                let count = count.trim().parse::<u64>().ok()?;
                (!path.is_empty()).then(|| ListedFile {
                    path: path.to_string(),
                    count: Some(count),
                })
            })
            .collect(),
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
        let shown = file
            .path
            .strip_prefix(ws_slash.as_str())
            .or_else(|| file.path.strip_prefix("./"))
            .unwrap_or(&file.path);
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
