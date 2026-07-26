//! Table wrapping and column-alignment tests (#1018 / PR #1038): long cells
//! must wrap vertically inside their own column, keeping other columns
//! aligned under their headers, within the viewport, and free of bleed.

use super::*;

fn strip_ansi(s: &str) -> String {
    let mut result = String::new();
    let mut in_escape = false;
    for ch in s.chars() {
        if in_escape {
            if ch.is_ascii_alphabetic() || ch == '~' {
                in_escape = false;
            }
        } else if ch == '\x1b' {
            in_escape = true;
        } else {
            result.push(ch);
        }
    }
    result
}

fn render_md(text: &str, width: usize) -> Vec<String> {
    let mut md = Markdown::new(text, 0);
    md.render(width)
}

fn render_plain(text: &str, width: usize) -> String {
    render_md(text, width)
        .iter()
        .map(|l| strip_ansi(l))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn table_long_cell_wraps_to_viewport_without_losing_text() {
    let md = "| Name | Value |\n| --- | --- |\n| key | alpha-beta-gamma-delta-epsilon-zeta |";
    let rendered = render_plain(md, 32);
    let joined_visible_lines: String = rendered.lines().map(str::trim).collect();
    assert!(
        joined_visible_lines.contains("alpha-beta-gamma-delta-epsilon-zeta"),
        "long table cell should remain readable without truncation: {rendered:?}"
    );
    for line in rendered.lines() {
        assert!(
            visible_width(line) <= 32,
            "rendered table line must fit width 32, got {}: {line:?}\n{rendered}",
            visible_width(line)
        );
    }
}

/// A long unbroken cell in a NON-last column must wrap inside its own column:
/// later cells stay aligned under their headers instead of being pushed right
/// or interleaved with the long cell's wrapped continuation rows (PR #1038
/// review: the joined-row wrapping approach bled columns into each other).
#[test]
fn table_long_first_column_cell_keeps_later_columns_aligned() {
    let md = "| Path | Status | Notes |\n| --- | --- | --- |\n| alpha-beta-gamma-delta-epsilon-zeta | ok | fine |";
    let rendered = render_plain(md, 40);
    let lines: Vec<&str> = rendered.lines().collect();
    let header = lines[0];
    let status_col = header.find("Status").expect("header shows Status");
    let notes_col = header.find("Notes").expect("header shows Notes");
    let data_row = lines
        .iter()
        .find(|l| l.contains("ok"))
        .expect("data row shows the ok cell");
    assert_eq!(
        data_row.find("ok"),
        Some(status_col),
        "Status cell must stay aligned under its header:\n{rendered}"
    );
    assert_eq!(
        data_row.find("fine"),
        Some(notes_col),
        "Notes cell must stay aligned under its header:\n{rendered}"
    );
    // The long cell wraps vertically inside its own column, so its full text
    // is read column-major: concatenate the first-column slice of each row.
    let col_a: String = lines[2..]
        .iter()
        .map(|l| l.get(..status_col.min(l.len())).unwrap_or(l).trim())
        .collect();
    assert!(
        col_a.contains("alpha-beta-gamma-delta-epsilon-zeta"),
        "long first cell should remain readable within its column: {rendered:?}"
    );
    for line in rendered.lines() {
        assert!(
            visible_width(line) <= 40,
            "rendered table line must fit width 40, got {}: {line:?}",
            visible_width(line)
        );
    }
}

/// Two over-long cells in the same row must each wrap within their own column
/// — the tail of one cell and the head of the other must never share a
/// physical line region outside their columns.
#[test]
fn table_two_long_cells_wrap_within_their_own_columns() {
    let md = "| A | B |\n| --- | --- |\n| alpha-beta-gamma-delta-epsilon-zeta-eta | one-two-three-four-five-six-seven-eight |";
    let rendered = render_plain(md, 32);
    let lines: Vec<&str> = rendered.lines().collect();
    let header = lines[0];
    let b_col = header.find('B').expect("header shows B");
    for line in &lines[2..] {
        // Column A's region must contain only column A content: nothing from
        // cell B ("one-two-...") may appear left of column B's offset.
        let left = &line[..b_col.min(line.len())];
        assert!(
            !left.contains("one-") && !left.contains("two-"),
            "cell B content bled into column A's region: {line:?}\n{rendered}"
        );
        // Column B's region must not contain cell A content.
        let right = line.get(b_col..).unwrap_or("");
        assert!(
            !right.contains("alpha") && !right.contains("gamma"),
            "cell A content bled into column B's region: {line:?}\n{rendered}"
        );
    }
    // Cells wrap vertically inside their columns — read each column top to
    // bottom to recover the complete cell text.
    let col_a: String = lines[2..]
        .iter()
        .map(|l| l.get(..b_col.min(l.len())).unwrap_or(l).trim())
        .collect();
    let col_b: String = lines[2..]
        .iter()
        .map(|l| l.get(b_col..).unwrap_or("").trim())
        .collect();
    assert!(
        col_a.contains("alpha-beta-gamma-delta-epsilon-zeta-eta"),
        "cell A retained in its column: {rendered:?}"
    );
    assert!(
        col_b.contains("one-two-three-four-five-six-seven-eight"),
        "cell B retained in its column: {rendered:?}"
    );
    for line in rendered.lines() {
        assert!(visible_width(line) <= 32, "line exceeds width 32: {line:?}");
    }
}

/// A cell long enough to hit the per-cell row cap must be cut with an
/// ellipsis WITHOUT dropping the other columns' cells (PR #1038 review: the
/// joined-row cap truncated away entire neighbouring columns) and without
/// emitting an orphaned ANSI-reset-only line.
#[test]
fn table_row_cap_preserves_other_columns() {
    let long = "z".repeat(300);
    let md = format!("| A | B |\n| --- | --- |\n| {long} | victor |");
    let rendered = render_plain(&md, 32);
    assert!(
        rendered.contains("victor"),
        "neighbouring cell must survive the long-cell row cap: {rendered:?}"
    );
    assert!(
        rendered.contains("..."),
        "capped cell should signal the cut with an ellipsis: {rendered:?}"
    );
    for line in rendered.lines() {
        assert!(visible_width(line) <= 32, "line exceeds width 32: {line:?}");
    }
    // No orphaned blank/reset-only rows inside the table: check the RAW
    // render line by line (a reset-only line must FAIL this check, so no
    // pre-filtering of empty lines before asserting).
    let raw_lines = render_md(&md, 32);
    let content_idx: Vec<usize> = raw_lines
        .iter()
        .enumerate()
        .filter(|(_, l)| !strip_ansi(l).trim().is_empty())
        .map(|(i, _)| i)
        .collect();
    let (first, last) = (content_idx[0], *content_idx.last().unwrap());
    for line in &raw_lines[first..=last] {
        assert!(
            !strip_ansi(line).trim().is_empty(),
            "table must not emit content-free rows: {raw_lines:?}"
        );
    }
}

/// Degenerate width: a 3-column table rendered far below its natural width
/// must not panic and must keep every line within the viewport.
#[test]
fn table_degenerate_narrow_width_stays_within_viewport() {
    let md = "| Alpha | Beta | Gamma |\n| --- | --- | --- |\n| one | two | three |";
    let rendered = render_plain(md, 8);
    assert!(!rendered.is_empty(), "table should still render at width 8");
    for line in rendered.lines() {
        assert!(
            visible_width(line) <= 8,
            "line exceeds width 8, got {}: {line:?}\n{rendered}",
            visible_width(line)
        );
    }
}

/// Double-width CJK content in a long cell must wrap within the viewport —
/// display columns, not chars or bytes.
#[test]
fn table_long_cjk_cell_wraps_within_viewport() {
    let long: String = "世界".repeat(10);
    let md = format!("| Name | Value |\n| --- | --- |\n| key | {long} |");
    let rendered = render_plain(&md, 30);
    assert!(
        rendered.contains("世界"),
        "CJK cell content should render: {rendered:?}"
    );
    for line in rendered.lines() {
        assert!(
            visible_width(line) <= 30,
            "line exceeds width 30, got {}: {line:?}\n{rendered}",
            visible_width(line)
        );
    }
}

/// A ragged data row must not panic or overflow the viewport. Note:
/// pulldown-cmark normalises rows to the header's column count, dropping
/// extra cells before they reach the renderer — this pins that policy plus
/// graceful layout of the row that has FEWER cells than the header.
#[test]
fn table_ragged_rows_stay_within_viewport() {
    let md = "| A | B |\n| --- | --- |\n| one | two | three |\n| only |";
    let rendered = render_plain(md, 32);
    assert!(
        rendered.contains("one") && rendered.contains("only"),
        "ragged rows should still render their kept cells: {rendered:?}"
    );
    for line in rendered.lines() {
        assert!(
            visible_width(line) <= 32,
            "line exceeds width 32, got {}: {line:?}\n{rendered}",
            visible_width(line)
        );
    }
}

/// Word-wrappable (spaced) cell content wraps within its column and keeps
/// short rows in other columns aligned.
#[test]
fn table_wordy_cell_wraps_within_column() {
    let md = "| K | Description | V |\n| --- | --- | --- |\n| k1 | this is a rather long description with many words in it | v1 |\n| k2 | short | v2 |";
    let rendered = render_plain(md, 40);
    let lines: Vec<&str> = rendered.lines().collect();
    let header = lines[0];
    let v_col = header.find('V').expect("header shows V");
    for needle in ["v1", "v2"] {
        let row = lines
            .iter()
            .find(|l| l.contains(needle))
            .unwrap_or_else(|| panic!("row with {needle} rendered:\n{rendered}"));
        assert_eq!(
            row.find(needle),
            Some(v_col),
            "V cell {needle} must stay aligned under its header:\n{rendered}"
        );
    }
    for line in rendered.lines() {
        assert!(visible_width(line) <= 40, "line exceeds width 40: {line:?}");
    }
}
