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
