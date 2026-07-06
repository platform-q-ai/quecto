//! Shared list-row renderer for the TUI's list/overlay surfaces (#997): the
//! select list, slash-command autocomplete, `@files` autocomplete and model
//! selector all draw the same shape — a windowed list, a `→ ` prefix on the
//! selected row, an accent label, an optional dim description column, and a
//! dim `(sel/total)` indicator on overflow. Windowing AND the indicator live
//! here ONCE; surfaces differ only via indent, [`DescriptionMode`], [`ListRow`].

use crate::interface::components::list_navigator::ListNavigator;
use crate::interface::theme;
use crate::interface::utils::{truncate_to_width, visible_width};

/// One renderable row: the display label exactly as the surface shows it
/// (with any `/`/`@` sigil), plus per-row decorations.
#[derive(Debug, Clone)]
pub struct ListRow {
    /// Display label (unstyled); the helper applies accent/dim styling.
    pub label: String,
    /// Dim right-hand column (command description, provider name), if any.
    pub description: Option<String>,
    /// Suffix drawn after the label but OUTSIDE the alignment column (the
    /// model selector's ` ●` current-model marker); empty when absent.
    pub marker: &'static str,
    /// Render the label dim and never accent it (`@files` loading placeholder).
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
#[derive(Debug, Clone, Copy)]
pub enum DescriptionMode {
    /// Column aligned to the widest VISIBLE label, capped at 32 (#757:
    /// off-screen items are never drawn, so widening the column for them
    /// would waste a full-list scan every frame). The description is truncated
    /// to the remaining width and dropped below `min_desc_width` (select list).
    AlignedWindow { min_desc_width: usize },
    /// Column aligned to a caller-cached label width (the model selector's
    /// `cached_max_label_width`, recomputed only on filter change — never a
    /// per-frame full-filtered-list scan, #757). The description is always
    /// drawn; the whole LINE is truncated to `width`. `label_width: 0` gives a
    /// fixed two-space gap (the slash-command autocomplete's layout).
    AlignedCached { label_width: usize },
}

/// Render the full list surface for `items`: window to the navigator's visible
/// range, build each visible row with `to_row`, and emit the rows plus — when
/// the window overflows — the dim `  (selected+1/total)` indicator line
/// (always two cells from the left margin, regardless of `indent`).
///
/// Behavior contract (characterized against the pre-#997 renderers): selected
/// row prefix `→ ` after `indent`, others two spaces; selected label accented
/// unless `dim_label` (dim labels are never accented); `marker` sits between
/// label and description gap, OUTSIDE the alignment column (the description
/// shifts by the marker width on that row, exactly as today); every emitted
/// line fits `width`.
#[expect(clippy::too_many_arguments, reason = "single shared entry point")]
pub fn render_windowed<T>(
    items: &[T],
    nav: &ListNavigator,
    max_visible: usize,
    width: usize,
    indent: &str,
    mode: DescriptionMode,
    to_row: impl Fn(&T) -> ListRow,
) -> Vec<String> {
    let range = nav.visible_range(items.len(), max_visible);
    let selected = nav.selected();
    let rows: Vec<ListRow> = items[range.clone()].iter().map(to_row).collect();
    let mut lines = Vec::with_capacity(rows.len() + 1);

    // `AlignedWindow` column: widest VISIBLE label only, capped at 32 (#757)
    // — `rows` holds just the window, never the full list.
    let window_label_width = match mode {
        DescriptionMode::AlignedWindow { .. } => rows
            .iter()
            .map(|r| visible_width(&r.label))
            .max()
            .unwrap_or(10)
            .min(32),
        _ => 0,
    };

    for (offset, row) in rows.iter().enumerate() {
        let is_sel = range.start + offset == selected;
        let prefix = if is_sel { "→ " } else { "  " };
        let label = if row.dim_label {
            theme::dim(&row.label)
        } else if is_sel {
            theme::accent(&row.label)
        } else {
            row.label.clone()
        };
        let label_vis = visible_width(&row.label);
        let mut line = format!("{}{}{}{}", indent, prefix, label, row.marker);

        // Dim column; `AlignedWindow` drops it below `min_desc_width`.
        if let Some(desc) = &row.description {
            let column = match mode {
                DescriptionMode::AlignedWindow { min_desc_width } => {
                    let gap = window_label_width.saturating_sub(label_vis) + 2;
                    let desc_start = visible_width(indent) + 2 + label_vis + gap;
                    let desc_width = width.saturating_sub(desc_start + 1);
                    (desc_width > min_desc_width)
                        .then(|| (gap, truncate_to_width(desc, desc_width, Some(""))))
                }
                DescriptionMode::AlignedCached { label_width } => {
                    Some((label_width.saturating_sub(label_vis) + 2, desc.clone()))
                }
            };
            if let Some((gap, desc)) = column {
                line.push_str(&" ".repeat(gap));
                line.push_str(&theme::dim(&desc));
            }
        }
        lines.push(truncate_to_width(&line, width, None));
    }

    if range.start > 0 || range.end < items.len() {
        let info = format!("  ({}/{})", selected + 1, items.len());
        lines.push(truncate_to_width(&theme::dim(&info), width, None));
    }

    lines
}

#[cfg(test)]
mod tests {
    /// Test-local style bundle (the prod API takes indent + mode directly).
    struct RowStyle {
        indent: &'static str,
        description: DescriptionMode,
    }

    use super::*;
    use crate::interface::ansi::strip_ansi;

    fn render_rows(
        rows: &[ListRow],
        nav: &ListNavigator,
        total: usize,
        max_visible: usize,
        width: usize,
        style: &RowStyle,
    ) -> Vec<String> {
        // Rebuild a full item list whose visible window is `rows`, so tests can
        // keep describing the window contents directly.
        let range = nav.visible_range(total, max_visible);
        let mut items: Vec<ListRow> = (0..total)
            .map(|i| ListRow::plain(format!("pad{i}")))
            .collect();
        for (offset, row) in rows.iter().enumerate() {
            items[range.start + offset] = row.clone();
        }
        render_windowed(
            &items,
            nav,
            max_visible,
            width,
            style.indent,
            style.description,
            |r| r.clone(),
        )
    }

    fn rows(labels: &[&str]) -> Vec<ListRow> {
        labels.iter().map(|l| ListRow::plain(*l)).collect()
    }

    fn plain_style() -> RowStyle {
        RowStyle {
            indent: "",
            description: DescriptionMode::AlignedCached { label_width: 0 },
        }
    }

    #[test]
    fn windows_rows_to_max_visible_plus_indicator() {
        let nav = ListNavigator::new();
        let window = nav.visible_range(5, 3);
        assert_eq!(window, 0..3, "window should cover the first 3 of 5 rows");
        let lines = render_rows(&rows(&["a", "b", "c"]), &nav, 5, 3, 40, &plain_style());
        assert_eq!(
            lines.len(),
            4,
            "3 visible rows + the overflow indicator line"
        );
    }

    #[test]
    fn render_windowed_windows_and_maps_items() {
        let nav = ListNavigator::new();
        let items = ["a", "b", "c", "d", "e"];
        let style = plain_style();
        let lines = render_windowed(&items, &nav, 3, 40, style.indent, style.description, |i| {
            ListRow::plain(*i)
        });
        assert_eq!(lines.len(), 4, "3 windowed rows + indicator");
        assert_eq!(strip_ansi(lines.last().unwrap()), "  (1/5)");
    }

    #[test]
    fn overflow_indicator_rendered_inside_helper() {
        let nav = ListNavigator::new();
        let lines = render_rows(&rows(&["a", "b", "c"]), &nav, 5, 3, 40, &plain_style());
        assert_eq!(
            strip_ansi(lines.last().unwrap()),
            "  (1/5)",
            "indicator must be emitted by the helper, dim, as `  (sel/total)`"
        );
    }

    #[test]
    fn no_indicator_when_everything_visible() {
        let nav = ListNavigator::new();
        let lines = render_rows(&rows(&["a", "b"]), &nav, 2, 5, 40, &plain_style());
        assert_eq!(lines.len(), 2, "no indicator when the window covers all");
    }

    #[test]
    fn selected_row_gets_arrow_prefix_and_accent() {
        let nav = ListNavigator::new();
        let lines = render_rows(&rows(&["a", "b"]), &nav, 2, 5, 40, &plain_style());
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
        let lines = render_rows(&[row], &nav, 1, 5, 60, &plain_style());
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
        let lines = render_rows(&[a, b], &nav, 2, 5, 60, &style);
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
        let lines = render_rows(&[a], &nav, 1, 5, 20, &style);
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
        let lines = render_rows(&[a], &nav, 1, 12, 23, &style);
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
        let lines = render_rows(&[other, cur], &nav, 2, 12, 60, &style);
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
        let lines = render_rows(&[row], &nav, 1, 5, 40, &plain_style());
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
    fn no_indicator_when_total_equals_max_visible() {
        // Exact boundary: total == max_visible must NOT show the indicator.
        let nav = ListNavigator::new();
        let lines = render_rows(
            &rows(&["a", "b", "c", "d", "e"]),
            &nav,
            5,
            5,
            40,
            &plain_style(),
        );
        assert_eq!(
            lines.len(),
            5,
            "all 5 rows drawn, no indicator at total==max"
        );
        assert!(
            !strip_ansi(lines.last().unwrap()).contains("(1/5)"),
            "no overflow indicator when the window covers exactly all rows"
        );
    }

    #[test]
    fn aligned_window_keeps_description_just_above_min_width() {
        // One cell wider than the drop test: desc_width = 21 - (2+5+2) - 1 = 11
        // > 10 → the description IS rendered, truncated to 11 cells.
        let mut a = ListRow::plain("alpha");
        a.description = Some("a description".into());
        let nav = ListNavigator::new();
        let style = RowStyle {
            indent: "",
            description: DescriptionMode::AlignedWindow { min_desc_width: 10 },
        };
        let lines = render_rows(&[a], &nav, 1, 5, 21, &style);
        assert_eq!(
            strip_ansi(&lines[0]),
            "→ alpha  a descripti",
            "just above the minimum the description is truncated, not dropped"
        );
        assert!(visible_width(&lines[0]) <= 21);
    }

    #[test]
    fn aligned_window_label_column_caps_at_32_cells() {
        // #757: the alignment column is capped at 32 cells even when a visible
        // label is wider; the short row's description starts at 32+gap and the
        // long row keeps the minimum 2-cell gap.
        let long_label = "x".repeat(36);
        let mut long = ListRow::plain(long_label.clone());
        long.description = Some("LD".into());
        let mut short = ListRow::plain("short");
        short.description = Some("SD".into());
        let nav = ListNavigator::new();
        let style = RowStyle {
            indent: "",
            description: DescriptionMode::AlignedWindow { min_desc_width: 10 },
        };
        let lines = render_rows(&[short, long], &nav, 2, 5, 80, &style);
        // Short label (5): gap = 32-5+2 = 29 spaces.
        assert_eq!(
            strip_ansi(&lines[0]),
            format!("→ short{}SD", " ".repeat(29)),
            "column aligns to the 32-cell cap, not the 36-cell label"
        );
        // Long label exceeds the cap: gap saturates to the minimum 2 cells.
        assert_eq!(strip_ansi(&lines[1]), format!("  {long_label}  LD"));
    }

    #[test]
    fn aligned_window_label_column_uses_actual_width_below_cap() {
        // Companion just-below-cap case: a 30-cell label aligns to 30, not 32.
        let label30 = "y".repeat(30);
        let mut wide = ListRow::plain(label30.clone());
        wide.description = Some("WD".into());
        let mut short = ListRow::plain("short");
        short.description = Some("SD".into());
        let nav = ListNavigator::new();
        let style = RowStyle {
            indent: "",
            description: DescriptionMode::AlignedWindow { min_desc_width: 10 },
        };
        let lines = render_rows(&[short, wide], &nav, 2, 5, 80, &style);
        assert_eq!(
            strip_ansi(&lines[0]),
            format!("→ short{}SD", " ".repeat(27)),
            "below the cap the column tracks the widest visible label exactly"
        );
    }

    #[test]
    fn indicator_tracks_selection_position() {
        let mut nav = ListNavigator::new();
        for _ in 0..3 {
            nav.move_next(5);
        }
        let items: Vec<ListRow> = (0..5).map(|i| ListRow::plain(format!("r{i}"))).collect();
        let style = plain_style();
        let lines = render_windowed(&items, &nav, 3, 40, style.indent, style.description, |r| {
            r.clone()
        });
        assert_eq!(
            strip_ansi(lines.last().unwrap()),
            "  (4/5)",
            "indicator must show the 1-based selection over the full total"
        );
    }
}
