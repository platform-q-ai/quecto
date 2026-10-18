// Shared truncation module for consistent output limiting across all tools.
// Mirrors Quecto's truncate.js — provides head/tail truncation, line truncation,
// and human-readable size formatting.

pub const DEFAULT_MAX_LINES: usize = 2_000;
pub const DEFAULT_MAX_BYTES: usize = crate::domain::constants::DEFAULT_OUTPUT_CAP_BYTES; // 50 KB
pub const GREP_MAX_LINE_LENGTH: usize = 500;

#[derive(Debug, Clone)]
pub struct TruncationResult {
    pub content: String,
    pub truncated: bool,
    pub truncated_by: Option<TruncatedBy>,
    pub total_lines: usize,
    pub total_bytes: usize,
    pub output_lines: usize,
    pub output_bytes: usize,
    pub last_line_partial: bool,
    pub first_line_exceeds_limit: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TruncatedBy {
    Lines,
    Bytes,
}

/// Keep the **first** N lines/bytes. Used by: read, grep, find, ls.
///
/// Whichever limit is hit first wins. Never returns partial lines.
/// If the first line alone exceeds the byte limit, returns empty content
/// with `first_line_exceeds_limit = true`.
pub fn truncate_head(content: &str, max_lines: usize, max_bytes: usize) -> TruncationResult {
    if content.is_empty() {
        return TruncationResult {
            content: String::new(),
            truncated: false,
            truncated_by: None,
            total_lines: 0,
            total_bytes: 0,
            output_lines: 0,
            output_bytes: 0,
            last_line_partial: false,
            first_line_exceeds_limit: false,
        };
    }
    let total_bytes = content.len();
    let total_lines = content.lines().count();

    let (output_lines, output_bytes, truncated, truncated_by, first_line_exceeds_limit) =
        compute_head_limits(content, max_lines, max_bytes);

    let result_content = build_head_content(
        content,
        output_lines,
        output_bytes,
        first_line_exceeds_limit,
    );
    let truncated = truncated || output_lines < total_lines;
    let result_bytes = result_content.len();

    TruncationResult {
        content: result_content,
        truncated,
        truncated_by,
        total_lines,
        total_bytes,
        output_lines,
        output_bytes: result_bytes,
        last_line_partial: false,
        first_line_exceeds_limit,
    }
}

/// Compute how many lines/bytes fit within the limits.
/// Returns (output_lines, output_bytes, truncated, truncated_by, first_line_exceeds_limit).
fn compute_head_limits(
    content: &str,
    max_lines: usize,
    max_bytes: usize,
) -> (usize, usize, bool, Option<TruncatedBy>, bool) {
    let mut output_lines = 0;
    let mut output_bytes = 0;
    let mut truncated = false;
    let mut truncated_by = None;
    let mut first_line_exceeds_limit = false;

    for line in content.lines() {
        let separator_bytes = usize::from(output_bytes > 0);
        let would_be = output_bytes + separator_bytes + line.len();
        if would_be > max_bytes {
            truncated = true;
            first_line_exceeds_limit = output_lines == 0;
            truncated_by = Some(TruncatedBy::Bytes);
            break;
        }
        if output_lines >= max_lines {
            truncated = true;
            truncated_by = Some(TruncatedBy::Lines);
            break;
        }
        output_bytes = would_be;
        output_lines += 1;
    }

    (
        output_lines,
        output_bytes,
        truncated,
        truncated_by,
        first_line_exceeds_limit,
    )
}

/// Build result string from the first `output_lines` lines of content.
fn build_head_content(
    content: &str,
    output_lines: usize,
    output_bytes: usize,
    first_line_exceeds_limit: bool,
) -> String {
    if first_line_exceeds_limit {
        return String::new();
    }
    let mut result = String::with_capacity(output_bytes);
    for (i, line) in content.lines().enumerate() {
        if i >= output_lines {
            break;
        }
        if i > 0 {
            result.push('\n');
        }
        result.push_str(line);
    }
    result
}

/// Keep the **last** N lines/bytes. Used by: bash.
///
/// Works backwards from the end. If the last line alone exceeds the byte
/// limit, takes the tail of that line (partial). Never splits UTF-8
/// codepoints.
pub fn truncate_tail(content: &str, max_lines: usize, max_bytes: usize) -> TruncationResult {
    let total_bytes = content.len();
    let total_lines = if content.is_empty() {
        0
    } else {
        content.lines().count()
    };

    if content.is_empty() {
        return TruncationResult {
            content: String::new(),
            truncated: false,
            truncated_by: None,
            total_lines: 0,
            total_bytes: 0,
            output_lines: 0,
            output_bytes: 0,
            last_line_partial: false,
            first_line_exceeds_limit: false,
        };
    }

    let mut selected_start = content.len();
    let mut output_bytes = 0;
    let mut output_lines = 0;
    let mut truncated = false;
    let mut truncated_by = None;
    let last_line_partial = false;
    let mut line_end = content.strip_suffix('\n').map_or(content.len(), str::len);

    loop {
        let line_start = content[..line_end].rfind('\n').map_or(0, |idx| idx + 1);
        // `str::lines()` treats "\r\n" as one terminator, so a '\r' preceding
        // a '\n' terminator is not part of the line; mirror that here. A lone
        // trailing '\r' with no '\n' after it is literal content and is kept.
        let mut line = &content[line_start..line_end];
        if content.as_bytes().get(line_end) == Some(&b'\n') {
            line = line.strip_suffix('\r').unwrap_or(line);
        }
        let separator_bytes = usize::from(output_lines > 0);
        let would_be = output_bytes + separator_bytes + line.len();

        if would_be > max_bytes {
            truncated = true;
            if output_lines == 0 {
                let tail_start = line.len().saturating_sub(max_bytes);
                let safe_start = (tail_start..line.len())
                    .find(|&pos| line.is_char_boundary(pos))
                    .unwrap_or(line.len());
                let partial = &line[safe_start..];
                return TruncationResult {
                    content: partial.to_string(),
                    truncated: true,
                    truncated_by: Some(TruncatedBy::Bytes),
                    total_lines,
                    total_bytes,
                    output_lines: 1,
                    output_bytes: partial.len(),
                    last_line_partial: true,
                    first_line_exceeds_limit: false,
                };
            }
            truncated_by = Some(TruncatedBy::Bytes);
            break;
        }

        if output_lines >= max_lines {
            truncated = true;
            truncated_by = Some(TruncatedBy::Lines);
            break;
        }

        output_bytes = would_be;
        output_lines += 1;
        selected_start = line_start;

        if line_start == 0 {
            break;
        }
        line_end = line_start - 1;
    }

    let result_content = build_tail_content(content, selected_start, output_lines, output_bytes);
    let result_bytes = result_content.len();

    TruncationResult {
        content: result_content,
        truncated,
        truncated_by,
        total_lines,
        total_bytes,
        output_lines,
        output_bytes: result_bytes,
        last_line_partial,
        first_line_exceeds_limit: false,
    }
}

fn build_tail_content(
    content: &str,
    selected_start: usize,
    output_lines: usize,
    output_bytes: usize,
) -> String {
    let mut result = String::with_capacity(output_bytes);
    for (idx, line) in content[selected_start..].lines().enumerate() {
        if idx >= output_lines {
            break;
        }
        if idx > 0 {
            result.push('\n');
        }
        result.push_str(line);
    }
    result
}

/// Truncate a single line to max characters. Used by: grep (500 chars).
///
/// If the line exceeds the limit, truncates and appends `... [truncated]`
/// (the marker does not count toward the budget). Returns
/// `(truncated_line, was_truncated)`. Bounded-scan core in [`crate::domain::text`].
pub fn truncate_line(line: &str, max_chars: usize) -> (String, bool) {
    match crate::domain::text::truncate_chars(line, max_chars, max_chars, "... [truncated]") {
        std::borrow::Cow::Borrowed(_) => (line.to_string(), false),
        std::borrow::Cow::Owned(s) => (s, true),
    }
}

/// Human-readable size formatting: "1.2KB", "3.5MB", "512B".
///
/// Uses `write!` into a pre-allocated buffer to avoid repeated heap
/// allocations from `format!()`.
pub fn format_size(bytes: usize) -> String {
    use std::fmt::Write;
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;

    let mut buf = String::with_capacity(8);
    let b = bytes as f64;

    if b >= GB {
        let _ = write!(buf, "{:.1}GB", b / GB);
    } else if b >= MB {
        let _ = write!(buf, "{:.1}MB", b / MB);
    } else if b >= KB {
        let _ = write!(buf, "{:.1}KB", b / KB);
    } else {
        let _ = write!(buf, "{}B", bytes);
    }
    buf
}

#[cfg(test)]
#[path = "truncate_tests.rs"]
mod tests;
