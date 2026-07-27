//! Terminal text utilities — visible width, truncation, word wrapping.
//!
//! All width calculations are ANSI-aware: escape sequences have zero visual
//! width. CJK characters are correctly counted as width 2.

use crate::components::ansi::{AnsiSegment, ansi_segments};
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

    let use_ellipsis = ell_width <= max_width;
    let target = if use_ellipsis {
        max_width - ell_width
    } else {
        max_width
    };

    let mut result = String::new();
    let mut width = 0;
    let mut active_osc8: Option<String> = None;
    let mut active_sgr = String::new();
    'outer: for seg in ansi_segments(s) {
        match seg {
            // Escape sequences are preserved verbatim and never count as width.
            AnsiSegment::Escape(esc) => {
                track_active_escape(esc, &mut active_osc8, &mut active_sgr);
                result.push_str(esc);
            }
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

    if use_ellipsis {
        result.push_str(ell);
    }
    close_active_controls(&mut result, &active_osc8, &active_sgr);
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

/// Sanitize, then truncate by visible display width with an ellipsis on overflow.
pub fn sanitize_truncate_width_with_ellipsis(s: &str, max_width: usize, ellipsis: &str) -> String {
    let ell_width = visible_width(ellipsis);
    let use_ellipsis = ell_width <= max_width;
    let target = if use_ellipsis {
        max_width - ell_width
    } else {
        max_width
    };
    let mut out = String::with_capacity(s.len().min(max_width.saturating_add(ellipsis.len())));
    let mut width = 0usize;
    let mut truncated = false;

    'outer: for seg in ansi_segments(s) {
        if let AnsiSegment::Text(text) = seg {
            for ch in text.chars() {
                if !crate::components::ansi::keep_char(ch, false) {
                    continue;
                }
                let ch_width = ch.width().unwrap_or(0);
                if ch_width > 0 && width + ch_width > target {
                    truncated = true;
                    break 'outer;
                }
                out.push(ch);
                width += ch_width;
            }
        }
    }

    if truncated && use_ellipsis {
        out.push_str(ellipsis);
    }
    out
}

/// Sanitize, then truncate by Unicode scalar count with an ellipsis on overflow.
pub fn sanitize_truncate_chars_with_ellipsis(s: &str, max_chars: usize, ellipsis: &str) -> String {
    let (mut out, truncated) = crate::components::ansi::sanitize_control_truncated(s, max_chars);
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

fn close_active_controls(line: &mut String, active_osc8: &Option<String>, active_sgr: &str) {
    if !active_sgr.is_empty() {
        line.push_str("\x1b[0m");
    }
    if active_osc8.is_some() {
        line.push_str("\x1b]8;;\x07");
    }
}

fn reopen_active_controls(line: &mut String, active_osc8: &Option<String>, active_sgr: &str) {
    if let Some(osc8) = active_osc8 {
        line.push_str(osc8);
    }
    line.push_str(active_sgr);
}

fn push_wrapped_line(
    lines: &mut Vec<String>,
    current_line: &mut String,
    active_osc8: &Option<String>,
    active_sgr: &str,
) {
    let mut line = current_line.trim_end().to_string();
    close_active_controls(&mut line, active_osc8, active_sgr);
    lines.push(line);
    current_line.clear();
}

fn wrap_segment(s: &str, max_width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current_line = String::new();
    let mut current_width = 0;
    let mut active_osc8: Option<String> = None;
    let mut active_sgr = String::new();

    for word in s.split_inclusive(|c: char| c.is_whitespace()) {
        let word_width = visible_width(word);

        if current_width + word_width <= max_width {
            for seg in ansi_segments(word) {
                match seg {
                    AnsiSegment::Escape(esc) => {
                        track_active_escape(esc, &mut active_osc8, &mut active_sgr);
                        current_line.push_str(esc);
                    }
                    AnsiSegment::Text(text) => current_line.push_str(text),
                }
            }
            current_width += word_width;
        } else if word_width > max_width {
            // Word is longer than one line — break it. Preserve escape
            // sequences atomically so wrapping cannot split SGR/OSC controls,
            // and carry any SGR/OSC state that was already active before this
            // long word began.
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
                                close_active_controls(&mut current_line, &active_osc8, &active_sgr);
                                lines.push(current_line);
                                current_line = String::new();
                                reopen_active_controls(
                                    &mut current_line,
                                    &active_osc8,
                                    &active_sgr,
                                );
                                current_width = 0;
                            }
                            current_line.push(ch);
                            current_width += ch_width;
                        }
                    }
                }
            }
        } else {
            // Start a new line at a word boundary. Close active controls before
            // the physical line boundary, then reopen them on the continuation
            // so OSC 8/SGR never leak sideways and never drop on spaced labels.
            if !current_line.is_empty() {
                push_wrapped_line(&mut lines, &mut current_line, &active_osc8, &active_sgr);
            }
            reopen_active_controls(&mut current_line, &active_osc8, &active_sgr);
            for seg in ansi_segments(word) {
                match seg {
                    AnsiSegment::Escape(esc) => {
                        track_active_escape(esc, &mut active_osc8, &mut active_sgr);
                        current_line.push_str(esc);
                    }
                    AnsiSegment::Text(text) => current_line.push_str(text),
                }
            }
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
#[path = "utils_tests.rs"]
mod tests;
