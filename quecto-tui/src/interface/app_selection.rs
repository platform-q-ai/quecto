//! Mouse text-selection types and helpers, extracted from `app.rs` (#528, #546).
//!
//! Kept in a separate module to keep `app.rs` under the line-count gate.

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
            AnsiSegment::Escape(esc) => result.push_str(esc),
            AnsiSegment::Text(text) => {
                for ch in text.chars() {
                    if vis_col == start_col && !highlighted {
                        result.push_str("\x1b[7m");
                        highlighted = true;
                    }
                    result.push(ch);
                    vis_col += 1;
                    if vis_col == end_col && highlighted {
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
