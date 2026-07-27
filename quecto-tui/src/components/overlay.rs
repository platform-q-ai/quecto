//! Overlay compositing helpers — splice overlay content on top of base lines.
//!
//! The live render path (`compose_frame` in `app_methods.rs`) composites the
//! model / resume / rewind selectors manually by splicing overlay text into the
//! base content lines at a computed row/col position. These ANSI-aware helpers
//! own that splicing.

use crate::components::ansi::{AnsiSegment, ansi_segments};
use crate::components::utils::visible_width;
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
#[path = "overlay_tests.rs"]
mod tests;
