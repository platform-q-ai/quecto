// Shared truncation module for consistent output limiting across all tools.
// Mirrors Pi's truncate.js — provides head/tail truncation, line truncation,
// and human-readable size formatting.
//
// Note on CRLF: `str::lines()` strips both `\n` and `\r\n`. The output is
// always normalised to `\n` separators. Byte accounting uses 1-byte `\n`.
//
// Note on zero limits: `max_lines = 0` or `max_bytes = 0` returns empty
// content with `truncated = true`. This is by design — callers should not
// pass zero limits unless they intend to suppress all output.

pub const DEFAULT_MAX_LINES: usize = 2_000;
pub const DEFAULT_MAX_BYTES: usize = 50 * 1024; // 50 KB

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
///
/// Single-pass implementation: counts lines and builds the output string
/// in one iteration, then counts remaining lines only when truncated.
pub fn truncate_head(content: &str, max_lines: usize, max_bytes: usize) -> TruncationResult {
    let total_bytes = content.len();

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

    let mut output = String::with_capacity(max_bytes.min(total_bytes));
    let mut output_lines = 0;
    let mut output_bytes = 0;
    let mut truncated = false;
    let mut truncated_by = None;
    let mut first_line_exceeds_limit = false;
    let mut lines_iter = content.lines();
    let mut remaining_lines = 0;

    for line in lines_iter.by_ref() {
        let line_bytes = line.len();
        let separator_bytes = if output_bytes > 0 { 1 } else { 0 };
        let would_be = output_bytes + separator_bytes + line_bytes;

        // Check byte limit
        if would_be > max_bytes {
            truncated = true;
            if output_lines == 0 {
                first_line_exceeds_limit = true;
            }
            truncated_by = Some(TruncatedBy::Bytes);
            remaining_lines = 1; // this line we couldn't fit
            break;
        }

        // Check line limit
        if output_lines >= max_lines {
            truncated = true;
            truncated_by = Some(TruncatedBy::Lines);
            remaining_lines = 1; // this line we couldn't fit
            break;
        }

        // Append to output
        if output_bytes > 0 {
            output.push('\n');
        }
        output.push_str(line);
        output_bytes = would_be;
        output_lines += 1;
    }

    // Count remaining lines only when truncated (avoids upfront O(n) scan)
    if truncated {
        remaining_lines += lines_iter.count();
    }
    let total_lines = output_lines + remaining_lines;

    let result_bytes = if first_line_exceeds_limit {
        output.clear();
        0
    } else {
        output.len()
    };

    TruncationResult {
        content: output,
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

/// Keep the **last** N lines/bytes. Used by: bash.
///
/// Scans backwards from the end to avoid O(n) memory for the full line
/// collection. If the last line alone exceeds the byte limit, takes the
/// tail of that line (partial), respecting UTF-8 boundaries.
///
/// Note: when `max_bytes` is very small and the content contains only
/// multi-byte characters, the UTF-8 boundary search may return fewer
/// bytes than `max_bytes` (or even empty) to avoid splitting codepoints.
pub fn truncate_tail(content: &str, max_lines: usize, max_bytes: usize) -> TruncationResult {
    let total_bytes = content.len();

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

    // Find line boundaries by scanning for newlines from the end.
    // We collect at most max_lines+1 boundary offsets to determine the cut point.
    // This is O(output) not O(input) in the common case.
    let bytes = content.as_bytes();
    let mut line_starts: Vec<usize> = Vec::with_capacity(max_lines + 2);
    line_starts.push(content.len()); // sentinel: end of content

    let mut pos = content.len();
    let mut total_lines = 1; // at least one line if content is non-empty

    // Scan backwards for newlines
    while pos > 0 {
        pos -= 1;
        if bytes[pos] == b'\n' {
            total_lines += 1;
            line_starts.push(pos + 1); // start of line after this newline

            // Stop collecting once we have enough for max_lines + 1
            // (we need one extra to know the start of the first selected line)
            if line_starts.len() > max_lines + 1 {
                // But continue counting total_lines
                while pos > 0 {
                    pos -= 1;
                    if bytes[pos] == b'\n' {
                        total_lines += 1;
                    }
                }
                break;
            }
        }
    }

    // Handle the very first line (starts at byte 0)
    if pos == 0 && (line_starts.is_empty() || *line_starts.last().unwrap() != 0) {
        line_starts.push(0);
    }

    // line_starts is in reverse order: [end, last_line_start, ..., first_line_start]
    line_starts.reverse();
    // Now line_starts[i] = start of line i, line_starts[last] = end sentinel

    let available_lines = line_starts.len() - 1; // subtract the end sentinel
    let lines_to_take = available_lines.min(max_lines);

    // Start from the end, taking lines_to_take lines
    let start_idx = available_lines - lines_to_take;
    let start_byte = line_starts[start_idx];
    let end_byte = content.len();

    // Calculate the content slice (strip trailing newline if present)
    let mut slice = &content[start_byte..end_byte];
    if slice.ends_with('\n') {
        slice = &slice[..slice.len() - 1];
    }

    // Check byte limit
    if slice.len() <= max_bytes {
        // Fits within byte limit
        let truncated = lines_to_take < total_lines;
        let truncated_by = if truncated {
            Some(TruncatedBy::Lines)
        } else {
            None
        };
        let output_lines = slice
            .lines()
            .count()
            .max(if slice.is_empty() { 0 } else { 1 });
        let result = slice.to_string();
        let result_bytes = result.len();
        return TruncationResult {
            content: result,
            truncated,
            truncated_by,
            total_lines,
            total_bytes,
            output_lines,
            output_bytes: result_bytes,
            last_line_partial: false,
            first_line_exceeds_limit: false,
        };
    }

    // Byte limit exceeded. Need to find a cut point.
    // If this is a single line (lines_to_take == 1 or all content is one line),
    // take the tail of that line.
    if available_lines == 1 || lines_to_take <= 1 {
        // Single line case: take the last max_bytes bytes, UTF-8 safe
        let tail_start = content.len().saturating_sub(max_bytes);
        let safe_start = (tail_start..content.len())
            .find(|&p| content.is_char_boundary(p))
            .unwrap_or(content.len());
        let partial = &content[safe_start..];
        // Strip trailing newline from partial
        let partial = partial.strip_suffix('\n').unwrap_or(partial);
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

    // Multiple lines but byte limit exceeded: walk backwards through line_starts
    // to find how many complete lines fit within max_bytes
    let mut best_start_idx = available_lines; // will take 0 lines
    let mut accumulated = 0;

    for idx in (start_idx..available_lines).rev() {
        let line_start = line_starts[idx];
        let line_end = if idx + 1 < line_starts.len() {
            line_starts[idx + 1]
        } else {
            content.len()
        };
        let mut line_slice = &content[line_start..line_end];
        if line_slice.ends_with('\n') {
            line_slice = &line_slice[..line_slice.len() - 1];
        }

        let separator = if best_start_idx < available_lines {
            1
        } else {
            0
        };
        let would_be = accumulated + separator + line_slice.len();

        if would_be > max_bytes {
            break;
        }

        accumulated = would_be;
        best_start_idx = idx;
    }

    if best_start_idx >= available_lines {
        // Can't fit even one complete line — take partial tail of last line
        let tail_start = content.len().saturating_sub(max_bytes);
        let safe_start = (tail_start..content.len())
            .find(|&p| content.is_char_boundary(p))
            .unwrap_or(content.len());
        let partial = &content[safe_start..];
        let partial = partial.strip_suffix('\n').unwrap_or(partial);
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

    let final_start = line_starts[best_start_idx];
    let mut result_slice = &content[final_start..end_byte];
    if result_slice.ends_with('\n') {
        result_slice = &result_slice[..result_slice.len() - 1];
    }
    let result = result_slice.to_string();
    let output_lines = result
        .lines()
        .count()
        .max(if result.is_empty() { 0 } else { 1 });
    let result_bytes = result.len();

    TruncationResult {
        content: result,
        truncated: true,
        truncated_by: Some(TruncatedBy::Bytes),
        total_lines,
        total_bytes,
        output_lines,
        output_bytes: result_bytes,
        last_line_partial: false,
        first_line_exceeds_limit: false,
    }
}

/// Truncate a single line to max characters. Used by: grep (500 chars).
///
/// If the line exceeds the limit, truncates and appends `... [truncated]`.
/// Returns `(truncated_line, was_truncated)`.
///
/// Single-pass: uses `char_indices().nth()` to find the byte offset in one
/// scan, then slices directly without re-iterating.
pub fn truncate_line(line: &str, max_chars: usize) -> (String, bool) {
    match line.char_indices().nth(max_chars) {
        None => (line.to_string(), false), // under limit
        Some((byte_offset, _)) => (format!("{}... [truncated]", &line[..byte_offset]), true),
    }
}

/// Human-readable size formatting: "1.2KB", "3.5MB", "512B".
///
/// Note: for values above 2^53 (~9 PB), `usize` to `f64` conversion loses
/// precision. This is acceptable for tool output display purposes.
pub fn format_size(bytes: usize) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;

    let b = bytes as f64;

    if b >= GB {
        format!("{:.1}GB", b / GB)
    } else if b >= MB {
        format!("{:.1}MB", b / MB)
    } else if b >= KB {
        format!("{:.1}KB", b / KB)
    } else {
        format!("{}B", bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // truncate_head tests
    // -----------------------------------------------------------------------

    #[test]
    fn head_empty_string() {
        let r = truncate_head("", 2000, 50 * 1024);
        assert!(!r.truncated);
        assert!(r.content.is_empty());
        assert_eq!(r.total_lines, 0);
        assert_eq!(r.output_lines, 0);
    }

    #[test]
    fn head_single_short_line() {
        let r = truncate_head("hello", 2000, 50 * 1024);
        assert!(!r.truncated);
        assert_eq!(r.content, "hello");
        assert_eq!(r.output_lines, 1);
    }

    #[test]
    fn head_truncates_by_line_limit() {
        let input: String = (0..3000)
            .map(|i| format!("line{}", i))
            .collect::<Vec<_>>()
            .join("\n");
        let r = truncate_head(&input, 2000, 50 * 1024 * 1024); // large byte limit
        assert!(r.truncated);
        assert_eq!(r.truncated_by, Some(TruncatedBy::Lines));
        assert_eq!(r.output_lines, 2000);
        assert_eq!(r.total_lines, 3000);
    }

    #[test]
    fn head_truncates_by_byte_limit() {
        // 100 lines of ~1000 bytes each = ~100KB > 50KB
        let line = "x".repeat(999);
        let input: String = (0..100)
            .map(|_| line.clone())
            .collect::<Vec<_>>()
            .join("\n");
        let r = truncate_head(&input, 2000, 50 * 1024);
        assert!(r.truncated);
        assert_eq!(r.truncated_by, Some(TruncatedBy::Bytes));
        assert!(r.output_bytes <= 50 * 1024);
    }

    #[test]
    fn head_first_line_exceeds_limit() {
        let input = "z".repeat(60000);
        let r = truncate_head(&input, 2000, 50 * 1024);
        assert!(r.truncated);
        assert!(r.first_line_exceeds_limit);
        assert!(r.content.is_empty());
    }

    #[test]
    fn head_exact_at_limit() {
        let total_bytes = 50 * 1024;
        let lines = 2000;
        let newlines = lines - 1;
        let content_chars = total_bytes - newlines;
        let chars_per_line = content_chars / lines;
        let extra = content_chars % lines;
        let mut s = String::new();
        for i in 0..lines {
            let n = if i < extra {
                chars_per_line + 1
            } else {
                chars_per_line
            };
            s.push_str(&"a".repeat(n));
            if i < lines - 1 {
                s.push('\n');
            }
        }
        assert_eq!(s.len(), total_bytes);
        let r = truncate_head(&s, 2000, 50 * 1024);
        assert!(!r.truncated);
        assert_eq!(r.output_lines, 2000);
    }

    #[test]
    fn head_multibyte_utf8() {
        let mut input = String::new();
        for i in 0..20 {
            input.push_str("héllo 世界 🦀");
            if i < 19 {
                input.push('\n');
            }
        }
        let r = truncate_head(&input, 10, 100);
        assert!(std::str::from_utf8(r.content.as_bytes()).is_ok());
    }

    #[test]
    fn head_no_partial_lines() {
        let line = "x".repeat(999);
        let input: String = (0..100)
            .map(|_| line.clone())
            .collect::<Vec<_>>()
            .join("\n");
        let r = truncate_head(&input, 2000, 50 * 1024);
        let counted = r.content.lines().count();
        assert_eq!(counted, r.output_lines);
    }

    #[test]
    fn head_zero_max_lines() {
        let r = truncate_head("hello\nworld", 0, 50 * 1024);
        assert!(r.truncated);
        assert!(r.content.is_empty());
        assert_eq!(r.truncated_by, Some(TruncatedBy::Lines));
    }

    #[test]
    fn head_zero_max_bytes() {
        let r = truncate_head("hello", 2000, 0);
        assert!(r.truncated);
        assert!(r.content.is_empty());
        assert!(r.first_line_exceeds_limit);
    }

    #[test]
    fn head_trailing_newline() {
        let r = truncate_head("a\nb\n", 2000, 50 * 1024);
        assert!(!r.truncated);
        // str::lines() yields ["a", "b"] for "a\nb\n"
        assert_eq!(r.output_lines, 2);
    }

    // -----------------------------------------------------------------------
    // truncate_tail tests
    // -----------------------------------------------------------------------

    #[test]
    fn tail_empty_string() {
        let r = truncate_tail("", 2000, 50 * 1024);
        assert!(!r.truncated);
        assert!(r.content.is_empty());
    }

    #[test]
    fn tail_single_short_line() {
        let r = truncate_tail("hello", 2000, 50 * 1024);
        assert!(!r.truncated);
        assert_eq!(r.content, "hello");
    }

    #[test]
    fn tail_truncates_by_line_limit_keeps_last() {
        let input: String = (0..3000)
            .map(|i| format!("line{}", i))
            .collect::<Vec<_>>()
            .join("\n");
        let r = truncate_tail(&input, 2000, 50 * 1024 * 1024);
        assert!(r.truncated);
        assert_eq!(r.truncated_by, Some(TruncatedBy::Lines));
        assert_eq!(r.output_lines, 2000);
        assert!(r.content.contains("line2999"));
        assert!(!r.content.starts_with("line0\n"));
    }

    #[test]
    fn tail_truncates_by_byte_limit_keeps_last() {
        let line = "y".repeat(999);
        let input: String = (0..100)
            .map(|_| line.clone())
            .collect::<Vec<_>>()
            .join("\n");
        let r = truncate_tail(&input, 2000, 50 * 1024);
        assert!(r.truncated);
        assert_eq!(r.truncated_by, Some(TruncatedBy::Bytes));
        assert!(r.output_bytes <= 50 * 1024);
        assert!(r.content.ends_with(&line));
    }

    #[test]
    fn tail_single_huge_line_partial() {
        let input = "z".repeat(60000);
        let r = truncate_tail(&input, 2000, 50 * 1024);
        assert!(r.truncated);
        assert!(r.last_line_partial);
        assert!(r.output_bytes <= 50 * 1024);
        assert!(input.ends_with(&r.content));
    }

    #[test]
    fn tail_utf8_safe() {
        let input = "🦀".repeat(20000); // 80KB of 4-byte chars
        let r = truncate_tail(&input, 2000, 50 * 1024);
        assert!(std::str::from_utf8(r.content.as_bytes()).is_ok());
    }

    #[test]
    fn tail_zero_max_lines() {
        let r = truncate_tail("hello\nworld", 0, 50 * 1024);
        assert!(r.truncated);
    }

    #[test]
    fn tail_multiline_byte_truncation() {
        // 20 lines of 5000 bytes each = 100KB > 50KB
        let line = "m".repeat(4999);
        let input: String = (0..20).map(|_| line.clone()).collect::<Vec<_>>().join("\n");
        let r = truncate_tail(&input, 2000, 50 * 1024);
        assert!(r.truncated);
        assert_eq!(r.truncated_by, Some(TruncatedBy::Bytes));
        assert!(r.output_bytes <= 50 * 1024);
        assert!(r.output_lines > 0);
        assert!(r.output_lines < 20);
        // Should contain the last line
        assert!(r.content.ends_with(&line));
    }

    // -----------------------------------------------------------------------
    // truncate_line tests
    // -----------------------------------------------------------------------

    #[test]
    fn line_short_unchanged() {
        let (result, truncated) = truncate_line("hello world", 500);
        assert_eq!(result, "hello world");
        assert!(!truncated);
    }

    #[test]
    fn line_exact_at_limit() {
        let input = "a".repeat(500);
        let (result, truncated) = truncate_line(&input, 500);
        assert_eq!(result, input);
        assert!(!truncated);
    }

    #[test]
    fn line_over_limit_truncated() {
        let input = "a".repeat(600);
        let (result, truncated) = truncate_line(&input, 500);
        assert!(truncated);
        assert!(result.ends_with("... [truncated]"));
        assert!(result.chars().count() <= 515); // 500 + "... [truncated]"
    }

    #[test]
    fn line_multibyte_chars() {
        let input = "🦀".repeat(600);
        let (result, truncated) = truncate_line(&input, 500);
        assert!(truncated);
        assert!(std::str::from_utf8(result.as_bytes()).is_ok());
    }

    // -----------------------------------------------------------------------
    // format_size tests
    // -----------------------------------------------------------------------

    #[test]
    fn format_size_bytes() {
        assert_eq!(format_size(0), "0B");
        assert_eq!(format_size(512), "512B");
        assert_eq!(format_size(1023), "1023B");
    }

    #[test]
    fn format_size_kilobytes() {
        assert_eq!(format_size(1024), "1.0KB");
        assert_eq!(format_size(1536), "1.5KB");
        assert_eq!(format_size(51200), "50.0KB");
    }

    #[test]
    fn format_size_megabytes() {
        assert_eq!(format_size(1048576), "1.0MB");
        assert_eq!(format_size(1572864), "1.5MB");
    }
}
