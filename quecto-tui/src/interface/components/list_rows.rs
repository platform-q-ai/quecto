//! Shared list-row renderer for the TUI's list/overlay surfaces (#997).
//!
//! The select list, slash-command autocomplete, `@files` autocomplete and the
//! model selector all draw the same shape: a visible window over a list, a
//! `→ ` prefix on the selected row, an accent label, an optional dim
//! description column, and a dim `(sel/total)` indicator when rows overflow
//! the window. This module renders that shape ONCE; the surfaces differ only
//! through [`RowStyle`] (indent, description column behavior) and per-row data
//! ([`ListRow`]: pre-formatted label, marker, dim flag).
//!
//! Windowing AND the overflow indicator live inside the helper so no call site
//! re-implements either.

use std::ops::Range;

use crate::interface::components::list_navigator::ListNavigator;
use crate::interface::theme;
use crate::interface::utils::{truncate_to_width, visible_width};

/// One renderable row: the display label exactly as the surface shows it
/// (including any sigil such as `/` or `@`), plus per-row decorations.
#[derive(Debug, Clone)]
pub struct ListRow {
    /// Display label (unstyled); the helper applies accent/dim styling.
    pub label: String,
    /// Dim right-hand column (command description, provider name), if any.
    pub description: Option<String>,
    /// Suffix drawn after the label but OUTSIDE the alignment column, e.g. the
    /// model selector's ` ●` current-model marker. Empty when absent.
    pub marker: &'static str,
    /// Render the label dim and never accent it (the `@files` loading
    /// placeholder while no real file list is present).
    pub dim_label: bool,
}

impl ListRow {
    /// A plain row with no description, marker or dimming.
    pub fn plain(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            description: None,
            marker: "",
            dim_label: false,
        }
    }
}

/// How the dim description column is laid out — the one real difference
/// between the four surfaces.
#[derive(Debug, Clone)]
pub enum DescriptionMode {
    /// Fixed two-space gap after the label; the whole line is truncated to
    /// `width` (slash-command autocomplete).
    Inline,
    /// Column aligned to the widest VISIBLE label (window-only scan, capped at
    /// 32 — see #757: off-screen items are never drawn, so widening the column
    /// for them would waste a full-list scan every frame). The description is
    /// truncated to the remaining width and dropped entirely when fewer than
    /// `min_desc_width` cells remain (select list).
    AlignedWindow { min_desc_width: usize },
    /// Column aligned to a caller-cached label width (the model selector's
    /// `cached_max_label_width`, recomputed only when the filter changes —
    /// never a per-frame full-filtered-list scan, #757). The description is
    /// always drawn; the whole line is truncated to `width`.
    AlignedCached { label_width: usize },
}

/// Per-surface render style carried into [`render_list_rows`].
#[derive(Debug, Clone)]
pub struct RowStyle {
    /// Leading indent before the `→ `/`  ` prefix on every row (the model
    /// selector's 2-space inset; empty for the dropdowns).
    pub indent: &'static str,
    /// Description column layout.
    pub description: DescriptionMode,
}

/// The visible window the helper will draw for `total` rows, as decided by the
/// navigator — exposed so call sites can build only the `ListRow`s that will
/// actually be rendered.
pub fn visible_window(nav: &ListNavigator, total: usize, max_visible: usize) -> Range<usize> {
    nav.visible_range(total, max_visible)
}

/// Render the visible window of `rows` plus, when the window overflows, the
/// dim `  (selected+1/total)` indicator line. `rows` holds ONLY the rows for
/// the window given by [`visible_window`]; `total` is the full list length.
///
/// Behavior contract (characterized against the pre-#997 renderers):
/// - selected row prefix `→ `, others two spaces, after `style.indent`;
/// - selected label accented unless `dim_label`; dim labels are never accented;
/// - `marker` sits between label and description gap, outside the alignment
///   column (so the provider column shifts by the marker width on that row,
///   exactly as today);
/// - every emitted line fits `width`.
pub fn render_list_rows(
    rows: &[ListRow],
    nav: &ListNavigator,
    total: usize,
    max_visible: usize,
    width: usize,
    style: &RowStyle,
) -> Vec<String> {
    // The parameters fix the helper's contract; the body lands in GREEN (#997).
    let _ = (rows, nav, total, max_visible, width, style);
    let _ = (
        theme::dim(""),
        visible_width(""),
        truncate_to_width("", 0, None),
    );
    unimplemented!("issue #997: shared list-row renderer not yet implemented")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip_ansi(s: &str) -> String {
        let mut out = String::new();
        let mut esc = false;
        for ch in s.chars() {
            if esc {
                if ch.is_ascii_alphabetic() || ch == '~' {
                    esc = false;
                }
            } else if ch == '\x1b' {
                esc = true;
            } else {
                out.push(ch);
            }
        }
        out
    }

    fn rows(labels: &[&str]) -> Vec<ListRow> {
        labels.iter().map(|l| ListRow::plain(*l)).collect()
    }

    fn plain_style() -> RowStyle {
        RowStyle {
            indent: "",
            description: DescriptionMode::Inline,
        }
    }

    #[test]
    fn windows_rows_to_max_visible_plus_indicator() {
        let nav = ListNavigator::new();
        let window = visible_window(&nav, 5, 3);
        assert_eq!(window, 0..3, "window should cover the first 3 of 5 rows");
        let lines = render_list_rows(&rows(&["a", "b", "c"]), &nav, 5, 3, 40, &plain_style());
        assert_eq!(
            lines.len(),
            4,
            "3 visible rows + the overflow indicator line"
        );
    }

    #[test]
    fn overflow_indicator_rendered_inside_helper() {
        let nav = ListNavigator::new();
        let lines = render_list_rows(&rows(&["a", "b", "c"]), &nav, 5, 3, 40, &plain_style());
        assert_eq!(
            strip_ansi(lines.last().unwrap()),
            "  (1/5)",
            "indicator must be emitted by the helper, dim, as `  (sel/total)`"
        );
    }

    #[test]
    fn no_indicator_when_everything_visible() {
        let nav = ListNavigator::new();
        let lines = render_list_rows(&rows(&["a", "b"]), &nav, 2, 5, 40, &plain_style());
        assert_eq!(lines.len(), 2, "no indicator when the window covers all");
    }

    #[test]
    fn selected_row_gets_arrow_prefix_and_accent() {
        let nav = ListNavigator::new();
        let lines = render_list_rows(&rows(&["a", "b"]), &nav, 2, 5, 40, &plain_style());
        assert!(strip_ansi(&lines[0]).starts_with("→ a"));
        assert!(strip_ansi(&lines[1]).starts_with("  b"));
        assert!(
            lines[0].contains("\x1b[36m"),
            "selected label should be accented: {:?}",
            lines[0]
        );
        assert!(
            !lines[1].contains("\x1b[36m"),
            "unselected label must be unstyled: {:?}",
            lines[1]
        );
    }

    #[test]
    fn inline_description_uses_fixed_two_space_gap() {
        let mut row = ListRow::plain("/model");
        row.description = Some("Select model".into());
        let nav = ListNavigator::new();
        let lines = render_list_rows(&[row], &nav, 1, 5, 60, &plain_style());
        assert_eq!(strip_ansi(&lines[0]), "→ /model  Select model");
    }

    #[test]
    fn aligned_window_column_pads_to_widest_visible_label() {
        let mut a = ListRow::plain("alpha");
        a.description = Some("first".into());
        let mut b = ListRow::plain("gamma-long");
        b.description = Some("third".into());
        let nav = ListNavigator::new();
        let style = RowStyle {
            indent: "",
            description: DescriptionMode::AlignedWindow { min_desc_width: 10 },
        };
        let lines = render_list_rows(&[a, b], &nav, 2, 5, 60, &style);
        // Widest visible label = 10 → gap for "alpha" is 10-5+2 = 7 spaces.
        assert_eq!(strip_ansi(&lines[0]), "→ alpha       first");
        assert_eq!(strip_ansi(&lines[1]), "  gamma-long  third");
    }

    #[test]
    fn aligned_window_drops_description_below_min_width() {
        let mut a = ListRow::plain("alpha");
        a.description = Some("a description".into());
        let nav = ListNavigator::new();
        let style = RowStyle {
            indent: "",
            description: DescriptionMode::AlignedWindow { min_desc_width: 10 },
        };
        // desc_width = 20 - (2+5+2) - 1 = 10, not > 10 → label only (pre-#997
        // select-list behavior preserved exactly).
        let lines = render_list_rows(&[a], &nav, 1, 5, 20, &style);
        assert_eq!(strip_ansi(&lines[0]), "→ alpha");
    }

    #[test]
    fn aligned_cached_keeps_description_at_narrow_width() {
        let mut a = ListRow::plain("a-model");
        a.description = Some("ProvA".into());
        let nav = ListNavigator::new();
        let style = RowStyle {
            indent: "  ",
            description: DescriptionMode::AlignedCached { label_width: 13 },
        };
        // Model-selector mode: the description is never silently dropped; the
        // WHOLE LINE is truncated to width instead (pre-#997 behavior).
        let lines = render_list_rows(&[a], &nav, 1, 12, 23, &style);
        let plain = strip_ansi(&lines[0]);
        assert!(
            plain.starts_with("  → a-model        Pr"),
            "description must be truncated, not dropped: {plain:?}"
        );
        assert!(visible_width(&lines[0]) <= 23);
    }

    #[test]
    fn marker_sits_outside_alignment_column() {
        let mut cur = ListRow::plain("model-bb-long");
        cur.description = Some("ProvB".into());
        cur.marker = " ●";
        let mut other = ListRow::plain("a-model");
        other.description = Some("ProvA".into());
        let nav = ListNavigator::new();
        let style = RowStyle {
            indent: "  ",
            description: DescriptionMode::AlignedCached { label_width: 13 },
        };
        let lines = render_list_rows(&[other, cur], &nav, 2, 12, 60, &style);
        // Non-current row: gap pads 7-char label to 13 (+2).
        assert_eq!(strip_ansi(&lines[0]), "  → a-model        ProvA");
        // Current row with the longest id: marker appended after the label,
        // shifting the provider by exactly the marker width — today's pixels.
        assert_eq!(strip_ansi(&lines[1]), "    model-bb-long ●  ProvB");
    }

    #[test]
    fn dim_label_row_is_dim_and_never_accented() {
        let mut row = ListRow::plain("loading files…");
        row.dim_label = true;
        let nav = ListNavigator::new();
        let lines = render_list_rows(&[row], &nav, 1, 5, 40, &plain_style());
        assert_eq!(strip_ansi(&lines[0]), "→ loading files…");
        assert!(
            lines[0].contains("\x1b[2m"),
            "dim rows must use the dim SGR: {:?}",
            lines[0]
        );
        assert!(
            !lines[0].contains("\x1b[36m"),
            "dim rows must not be accented even when selected: {:?}",
            lines[0]
        );
    }

    #[test]
    fn indicator_tracks_selection_position() {
        let mut nav = ListNavigator::new();
        for _ in 0..3 {
            nav.move_next(5);
        }
        let window = visible_window(&nav, 5, 3);
        let visible: Vec<ListRow> = window
            .clone()
            .map(|i| ListRow::plain(format!("r{i}")))
            .collect();
        let lines = render_list_rows(&visible, &nav, 5, 3, 40, &plain_style());
        assert_eq!(
            strip_ansi(lines.last().unwrap()),
            "  (4/5)",
            "indicator must show the 1-based selection over the full total"
        );
    }
}
