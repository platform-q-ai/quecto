//! Overlay compositing helpers — splice overlay content on top of base lines.
//!
//! The live render path (`compose_frame` in `app_methods.rs`) composites the
//! model / resume / rewind selectors manually by splicing overlay text into the
//! base content lines at a computed row/col position. These ANSI-aware helpers
//! own that splicing.

use crate::interface::ansi::{AnsiSegment, ansi_segments};
use crate::interface::utils::visible_width;
use unicode_width::UnicodeWidthChar;

/// Splice overlay content into a base line at the given column.
///
/// ANSI-aware: properly resets attributes at splice boundaries.
pub fn splice_line(
    base: &str,
    overlay: &str,
    start_col: usize,
    overlay_width: usize,
    total_width: usize,
) -> String {
    let base_width = visible_width(base);

    // Build: [before][overlay][after]
    let before = if start_col > 0 {
        if base_width >= start_col {
            // Take the first start_col visible characters from base.
            take_visible_chars(base, start_col)
        } else {
            // Base is shorter than start_col — pad with spaces.
            let mut s = base.to_string();
            let pad = start_col - base_width;
            s.push_str(&" ".repeat(pad));
            s
        }
    } else {
        String::new()
    };

    let after_start = start_col + overlay_width;
    let after = if after_start < total_width && after_start < base_width {
        skip_visible_chars(base, after_start)
    } else {
        String::new()
    };

    format!("{}\x1b[0m{}\x1b[0m{}", before, overlay, after)
}

/// Take the first `n` visible characters from a string (ANSI-aware).
fn take_visible_chars(s: &str, n: usize) -> String {
    let mut result = String::new();
    let mut width = 0;

    'outer: for seg in ansi_segments(s) {
        match seg {
            AnsiSegment::Escape(esc) => result.push_str(esc),
            AnsiSegment::Text(text) => {
                for ch in text.chars() {
                    let cw = ch.width().unwrap_or(0);
                    if width + cw > n {
                        break 'outer;
                    }
                    result.push(ch);
                    width += cw;
                }
            }
        }
    }

    // Pad if we didn't reach n.
    while width < n {
        result.push(' ');
        width += 1;
    }

    result
}

/// Skip the first `n` visible characters and return the rest (ANSI-aware).
fn skip_visible_chars(s: &str, n: usize) -> String {
    let mut width = 0;
    let mut byte_offset = 0;

    'outer: for seg in ansi_segments(s) {
        match seg {
            AnsiSegment::Escape(esc) => byte_offset += esc.len(),
            AnsiSegment::Text(text) => {
                for ch in text.chars() {
                    let cw = ch.width().unwrap_or(0);
                    if width + cw > n {
                        break 'outer;
                    }
                    width += cw;
                    byte_offset += ch.len_utf8();
                }
            }
        }
    }

    s[byte_offset..].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splice_line_basic() {
        let result = splice_line("AAAAAAAAAA", "XX", 3, 2, 10);
        let plain: String = result.chars().filter(|c| !c.is_control()).collect();
        assert!(plain.contains("XX"), "should contain overlay: {}", plain);
    }

    #[test]
    fn take_visible_chars_basic() {
        assert_eq!(take_visible_chars("hello world", 5), "hello");
    }

    #[test]
    fn take_visible_chars_with_ansi() {
        let s = "\x1b[31mhello\x1b[0m world";
        let result = take_visible_chars(s, 5);
        assert!(result.contains("hello"));
    }

    #[test]
    fn skip_visible_chars_basic() {
        assert_eq!(skip_visible_chars("hello world", 6), "world");
    }

    #[test]
    fn splice_line_preserves_surrounding_content() {
        // Overlay 2 chars at col 3, width 10 → chars 0-2, overlay, chars 5+
        let result = splice_line("AAAAAAAAAA", "XX", 3, 2, 10);
        let plain: String = result.chars().filter(|c| !c.is_control()).collect();
        assert!(
            plain.starts_with("AAA"),
            "prefix should be preserved: {plain}"
        );
        assert!(plain.contains("XX"), "overlay should appear: {plain}");
        assert!(
            plain.ends_with("AAAAA"),
            "suffix should be preserved: {plain}"
        );
    }

    #[test]
    fn splice_line_with_ansi_base() {
        let base = "\x1b[31mAAAAAAAAAA\x1b[0m";
        let result = splice_line(base, "XX", 3, 2, 10);
        let plain: String = result.chars().filter(|c| !c.is_control()).collect();
        assert!(
            plain.contains("XX"),
            "overlay should appear through ANSI: {plain}"
        );
    }
}
