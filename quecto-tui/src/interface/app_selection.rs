//! Mouse text-selection types and helpers, extracted from `app.rs` (#528, #546).
//!
//! Kept in a separate module to keep `app.rs` under the line-count gate.

use unicode_width::UnicodeWidthChar;

pub(super) fn display_col_to_char_idx(chars: &[char], target_col: usize) -> usize {
    let mut vis_col = 0usize;
    for (idx, ch) in chars.iter().enumerate() {
        let next_col = vis_col.saturating_add(ch.width().unwrap_or(0));
        if next_col > target_col {
            return idx;
        }
        vis_col = next_col;
    }
    chars.len()
}

/// Mouse selection anchor for click-and-drag text copy (#528).
#[derive(Debug, Clone, Copy)]
pub(super) struct SelectionAnchor {
    pub(super) col: u16,
    pub(super) row: u16,
}

/// Active text selection (from mouse press to release) (#528).
#[derive(Debug, Clone)]
pub(super) struct TextSelection {
    /// Where the mouse was pressed.
    pub(super) start: SelectionAnchor,
    /// Current drag position (updated on mouse motion).
    pub(super) end: SelectionAnchor,
}

/// Normalize a selection into (start_row, start_col, end_row, end_col) order (#546).
/// Ensures start ≤ end regardless of drag direction.
pub(super) fn selection_range(sel: &TextSelection) -> (u16, u16, u16, u16) {
    let (sr, sc, er, ec) = if sel.start.row < sel.end.row
        || (sel.start.row == sel.end.row && sel.start.col <= sel.end.col)
    {
        (sel.start.row, sel.start.col, sel.end.row, sel.end.col)
    } else {
        (sel.end.row, sel.end.col, sel.start.row, sel.start.col)
    };
    (sr, sc, er, ec)
}

/// Apply mouse selection highlight to rendered lines (#546).
///
/// `body_start_col` is the visible column where selectable body text begins in
/// the final composed frame. Multi-row selections normally continue at column 0,
/// but split-pane frames reserve those leading columns for the sidepanel; clamp
/// every highlighted range to the body so selection never paints over chrome.
pub(super) fn apply_selection_highlight(
    selection: &Option<TextSelection>,
    lines: &mut [String],
    body_start_col: u16,
) {
    let Some(sel) = selection else { return };
    let (sr, sc, er, ec) = selection_range(sel);
    for row_idx in sr..=er {
        if (row_idx as usize) < lines.len() {
            let line_start = if row_idx == sr { sc } else { 0 }.max(body_start_col);
            let line_end = if row_idx == er {
                ec
            } else {
                crate::interface::utils::visible_width(&lines[row_idx as usize]) as u16
            }
            .max(body_start_col);
            lines[row_idx as usize] =
                apply_line_highlight(&lines[row_idx as usize], line_start, line_end);
        }
    }
}

/// Apply reverse-video highlighting to a range of visible columns in a line (#546).
///
/// Takes a rendered line (may contain ANSI escapes) and highlights columns
/// `start_col..end_col` (0-indexed, exclusive end) by wrapping visible chars
/// in that range with `\x1b[7m` (reverse) and `\x1b[27m` (reverse off).
///
/// Theme helpers (`dim`, `accent`, …) wrap markers with a full SGR reset
/// (`\x1b[0m`). A bare pass-through of that reset would kill reverse video for
/// the rest of the line, so only the gutter/bullet stayed highlighted while
/// the following body text did not (#1146 follow-up). When we are inside the
/// selection span, any escape that clears reverse is followed by a re-assert
/// of `\x1b[7m` — the same pattern `theme::apply_bg` uses for box backgrounds.
pub(super) fn apply_line_highlight(line: &str, start_col: u16, end_col: u16) -> String {
    use crate::interface::ansi::{AnsiSegment, ansi_segments};

    if start_col >= end_col {
        return line.to_string();
    }
    let mut result = String::with_capacity(line.len() + 20);
    let mut vis_col: u16 = 0;
    let mut highlighted = false;

    for seg in ansi_segments(line) {
        match seg {
            // Pass through ANSI escape sequences without counting columns.
            // Re-assert reverse when an SGR would clear it mid-selection.
            AnsiSegment::Escape(esc) => {
                result.push_str(esc);
                if highlighted && sgr_clears_reverse(esc) {
                    result.push_str("\x1b[7m");
                }
            }
            AnsiSegment::Text(text) => {
                for ch in text.chars() {
                    let ch_width = ch.width().unwrap_or(0) as u16;
                    let next_col = vis_col.saturating_add(ch_width);
                    let intersects_selection =
                        ch_width > 0 && next_col > start_col && vis_col < end_col;

                    if intersects_selection && !highlighted {
                        result.push_str("\x1b[7m");
                        highlighted = true;
                    } else if !intersects_selection && highlighted {
                        result.push_str("\x1b[27m");
                        highlighted = false;
                    }

                    result.push(ch);
                    vis_col = next_col;

                    if highlighted && vis_col >= end_col {
                        result.push_str("\x1b[27m");
                        highlighted = false;
                    }
                }
            }
        }
    }
    if highlighted {
        result.push_str("\x1b[27m");
    }
    result
}

/// Whether a CSI SGR escape leaves reverse-video off, so a live selection
/// highlight must re-assert `\x1b[7m` after it.
///
/// Tracks net effect in parameter order: full reset (`0` / empty) and
/// reverse-off (`27`) clear it; reverse-on (`7`) sets it. Non-SGR escapes
/// (OSC, cursor, …) never touch reverse and return false.
fn sgr_clears_reverse(esc: &str) -> bool {
    let Some(params) = esc
        .strip_prefix("\x1b[")
        .and_then(|rest| rest.strip_suffix('m'))
    else {
        return false;
    };
    if params.is_empty() {
        return true; // `\x1b[m` == full reset
    }
    let mut reverse_off = false;
    let mut it = params.split(';');
    while let Some(p) = it.next() {
        let code: u16 = if p.is_empty() {
            0
        } else {
            p.parse().unwrap_or(u16::MAX)
        };
        match code {
            0 => reverse_off = true,
            7 => reverse_off = false,
            27 => reverse_off = true,
            // Consume extended-colour value params so components are not
            // misread as attribute codes (same discipline as apply_bg).
            38 | 48 => match it.next().map(|x| x.parse::<u16>().unwrap_or(u16::MAX)) {
                Some(5) => {
                    it.next();
                }
                Some(2) => {
                    it.next();
                    it.next();
                    it.next();
                }
                _ => {}
            },
            _ => {}
        }
    }
    reverse_off
}
