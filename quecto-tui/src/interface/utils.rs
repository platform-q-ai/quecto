//! Terminal text utilities — visible width, truncation, word wrapping.
//!
//! All width calculations are ANSI-aware: escape sequences have zero visual
//! width. CJK characters are correctly counted as width 2.

use crate::interface::ansi::{AnsiSegment, ansi_segments};
use unicode_width::UnicodeWidthChar;

/// Calculate the visible width of a string, ignoring ANSI escape sequences.
///
/// CJK characters count as width 2. Control characters and ANSI escapes
/// count as width 0.
pub fn visible_width(s: &str) -> usize {
    let mut width = 0;
    for seg in ansi_segments(s) {
        if let AnsiSegment::Text(text) = seg {
            // `width()` returns `None` for control characters, so `unwrap_or(0)`
            // gives them zero width — matching the historical behaviour.
            for ch in text.chars() {
                width += ch.width().unwrap_or(0);
            }
        }
    }
    width
}

/// Truncate a string to fit within `max_width` visible columns.
///
/// ANSI escape sequences are preserved. If the text is truncated and
/// `ellipsis` is provided, it replaces the last few characters.
pub fn truncate_to_width(s: &str, max_width: usize, ellipsis: Option<&str>) -> String {
    let ell = ellipsis.unwrap_or("");
    let ell_width = visible_width(ell);

    if visible_width(s) <= max_width {
        return s.to_string();
    }

    let target = max_width.saturating_sub(ell_width);

    let mut result = String::new();
    let mut width = 0;
    'outer: for seg in ansi_segments(s) {
        match seg {
            // Escape sequences are preserved verbatim and never count as width.
            AnsiSegment::Escape(esc) => result.push_str(esc),
            AnsiSegment::Text(text) => {
                for ch in text.chars() {
                    let ch_width = ch.width().unwrap_or(0);
                    // Zero-width chars (control chars, combining marks) are kept
                    // without advancing the column count.
                    if ch_width == 0 {
                        result.push(ch);
                        continue;
                    }
                    if width + ch_width > target {
                        break 'outer;
                    }
                    result.push(ch);
                    width += ch_width;
                }
            }
        }
    }

    // Append ellipsis
    result.push_str(ell);
    // Append SGR reset so truncated escape sequences don't leak
    result.push_str("\x1b[0m");
    result
}

/// Truncate by Unicode scalar count, preserving legacy char-count ellipsis semantics.
pub fn truncate_chars_with_ellipsis(s: &str, max_chars: usize, ellipsis: &str) -> String {
    let mut iter = s.chars();
    let out: String = iter.by_ref().take(max_chars).collect();
    if iter.next().is_some() {
        format!("{out}{ellipsis}")
    } else {
        // `out` already holds all of `s` here; reuse it instead of
        // allocating a second copy on the common fits path.
        out
    }
}

/// Sanitize, then truncate by Unicode scalar count with an ellipsis on overflow.
pub fn sanitize_truncate_chars_with_ellipsis(s: &str, max_chars: usize, ellipsis: &str) -> String {
    let (mut out, truncated) = crate::interface::ansi::sanitize_control_truncated(s, max_chars);
    if truncated {
        out.push_str(ellipsis);
    }
    out
}

/// Word-wrap text to fit within `max_width` columns, preserving ANSI escapes.
///
/// Splits on word boundaries when possible. Long words that exceed the width
/// are broken at the column boundary.
pub fn wrap_text(s: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return vec![s.to_string()];
    }

    // Honor explicit line breaks first. A literal newline embedded in the
    // input (e.g. a multi-line paste or a hyphenated list typed in the editor)
    // is a hard break, not ordinary whitespace. Treating it as whitespace let
    // the newline survive inside a rendered line, which the terminal then
    // printed as a real cursor move — producing the staircased, corrupted
    // output. Split on newlines and word-wrap each segment independently.
    let mut lines = Vec::new();
    for segment in s.split('\n') {
        // `split('\n')` drops the delimiters and yields an empty segment for a
        // trailing newline, which we preserve as a blank line. Word-wrap each
        // segment independently; empty segments become a single blank line.
        lines.extend(wrap_segment(segment, max_width));
    }
    lines
}

/// Word-wrap a single newline-free segment to `max_width` columns.
fn track_active_escape(esc: &str, active_osc8: &mut Option<String>, active_sgr: &mut String) {
    if esc.starts_with("\x1b]8;;") {
        if esc == "\x1b]8;;\x07" || esc == "\x1b]8;;\x1b\\" {
            *active_osc8 = None;
        } else {
            *active_osc8 = Some(esc.to_string());
        }
    } else if esc.starts_with("\x1b[") && esc.ends_with('m') {
        if esc == "\x1b[0m" {
            active_sgr.clear();
        } else {
            active_sgr.push_str(esc);
        }
    }
}

fn wrap_segment(s: &str, max_width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current_line = String::new();
    let mut current_width = 0;

    for word in s.split_inclusive(|c: char| c.is_whitespace()) {
        let word_width = visible_width(word);

        if current_width + word_width <= max_width {
            current_line.push_str(word);
            current_width += word_width;
        } else if word_width > max_width {
            // Word is longer than one line — break it. Preserve escape
            // sequences atomically so wrapping cannot split SGR/OSC controls.
            let mut active_osc8: Option<String> = None;
            let mut active_sgr = String::new();
            for seg in ansi_segments(word) {
                match seg {
                    AnsiSegment::Escape(esc) => {
                        track_active_escape(esc, &mut active_osc8, &mut active_sgr);
                        current_line.push_str(esc);
                    }
                    AnsiSegment::Text(text) => {
                        for ch in text.chars() {
                            let ch_width = ch.width().unwrap_or(0);
                            if current_width + ch_width > max_width && current_width > 0 {
                                current_line.push_str("\x1b[0m");
                                if active_osc8.is_some() {
                                    current_line.push_str("\x1b]8;;\x07");
                                }
                                lines.push(current_line);
                                current_line = String::new();
                                if let Some(osc8) = &active_osc8 {
                                    current_line.push_str(osc8);
                                }
                                current_line.push_str(&active_sgr);
                                current_width = 0;
                            }
                            current_line.push(ch);
                            current_width += ch_width;
                        }
                    }
                }
            }
        } else {
            // Start a new line
            if !current_line.is_empty() {
                // Trim trailing whitespace from the line we're finishing
                let trimmed = current_line.trim_end().to_string();
                lines.push(trimmed);
            }
            current_line = word.to_string();
            current_width = word_width;
        }
    }

    if !current_line.is_empty() {
        lines.push(current_line);
    }

    if lines.is_empty() {
        lines.push(String::new());
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── visible_width ─────────────────────────────────────────────────────

    #[test]
    fn plain_ascii() {
        assert_eq!(visible_width("hello"), 5);
    }

    #[test]
    fn empty_string() {
        assert_eq!(visible_width(""), 0);
    }

    #[test]
    fn ansi_color_ignored() {
        assert_eq!(visible_width("\x1b[31mred\x1b[0m"), 3);
    }

    #[test]
    fn cjk_double_width() {
        assert_eq!(visible_width("日本語"), 6); // 3 chars × 2 width
    }

    #[test]
    fn mixed_ansi_and_text() {
        assert_eq!(visible_width("\x1b[1m\x1b[32mHello\x1b[0m World"), 11);
    }

    #[test]
    fn osc_hyperlink_ignored() {
        // OSC 8 hyperlink: \x1b]8;;url\x07text\x1b]8;;\x07
        let s = "\x1b]8;;https://example.com\x07link\x1b]8;;\x07";
        assert_eq!(visible_width(s), 4); // only "link" is visible
    }

    // ── truncate_to_width ─────────────────────────────────────────────────

    #[test]
    fn truncate_short_string_unchanged() {
        assert_eq!(truncate_to_width("hi", 10, None), "hi");
    }

    #[test]
    fn truncate_with_ellipsis() {
        let result = truncate_to_width("Hello, World!", 8, Some("..."));
        assert!(visible_width(&result) <= 8);
        assert!(result.contains("..."));
    }

    #[test]
    fn truncate_preserves_ansi() {
        let input = "\x1b[31mRedText\x1b[0m";
        let result = truncate_to_width(input, 3, None);
        // Should contain the ANSI start but truncated content
        assert!(result.contains("\x1b[31m"));
        assert!(visible_width(&result) <= 3);
    }

    // ── wrap_text ─────────────────────────────────────────────────────────

    #[test]
    fn wrap_short_text_single_line() {
        let lines = wrap_text("hello", 80);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0], "hello");
    }

    #[test]
    fn wrap_at_word_boundary() {
        let lines = wrap_text("hello world", 6);
        assert!(lines.len() >= 2);
        for line in &lines {
            assert!(visible_width(line) <= 6);
        }
    }

    #[test]
    fn wrap_long_word_breaks() {
        let lines = wrap_text("abcdefghij", 5);
        assert!(lines.len() >= 2);
        for line in &lines {
            assert!(visible_width(line) <= 5, "line '{}' is too wide", line);
        }
    }

    #[test]
    fn visible_width_control_chars_zero() {
        assert_eq!(visible_width("\x01\x02\x03"), 0);
    }

    #[test]
    fn visible_width_osc_with_st_terminator() {
        // OSC terminated with ST (\x1b\\)
        let s = "\x1b]0;title\x1b\\visible";
        assert_eq!(visible_width(s), 7); // "visible"
    }

    #[test]
    fn truncate_to_zero_width() {
        let result = truncate_to_width("hello", 0, None);
        assert_eq!(visible_width(&result), 0);
    }

    #[test]
    fn truncate_exact_width() {
        let result = truncate_to_width("hello", 5, None);
        assert!(result.contains("hello"));
    }

    #[test]
    fn truncate_cjk_respects_double_width() {
        let result = truncate_to_width("日本語テスト", 6, None);
        assert!(visible_width(&result) <= 6);
    }

    #[test]
    fn wrap_empty_string() {
        let lines = wrap_text("", 80);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0], "");
    }

    #[test]
    fn wrap_exact_width() {
        let lines = wrap_text("12345", 5);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn wrap_with_ansi() {
        let lines = wrap_text("\x1b[31mhello world\x1b[0m", 6);
        assert!(lines.len() >= 2);
    }

    #[test]
    fn wrap_splits_on_embedded_newlines() {
        // A multi-line hyphenated list must become separate lines, never a
        // single line with embedded '\n' that the terminal staircases.
        let lines = wrap_text("- hi\n- how\n- you", 80);
        assert_eq!(lines, vec!["- hi", "- how", "- you"]);
        for line in &lines {
            assert!(
                !line.contains('\n'),
                "wrapped line leaked a newline: {line:?}"
            );
        }
    }

    #[test]
    fn wrap_preserves_blank_lines_between_paragraphs() {
        let lines = wrap_text("a\n\nb", 80);
        assert_eq!(lines, vec!["a", "", "b"]);
    }

    #[test]
    fn wrap_trailing_newline_yields_trailing_blank() {
        let lines = wrap_text("a\n", 80);
        assert_eq!(lines, vec!["a", ""]);
    }

    #[test]
    fn wrap_each_segment_still_word_wraps() {
        // Newline splitting must not bypass width wrapping of long segments.
        let lines = wrap_text("hello world\nfoo", 6);
        assert!(lines.len() >= 3, "expected wrapped segments: {lines:?}");
        assert_eq!(lines.last().unwrap(), "foo");
        for line in &lines {
            assert!(visible_width(line) <= 6);
        }
    }

    #[test]
    fn visible_width_emoji() {
        // Emoji characters are typically width 2
        let w = visible_width("🦀");
        assert!((1..=2).contains(&w));
    }

    #[test]
    fn truncate_with_empty_ellipsis() {
        let result = truncate_to_width("hello world", 5, Some(""));
        assert!(visible_width(&result) <= 5);
    }
}
