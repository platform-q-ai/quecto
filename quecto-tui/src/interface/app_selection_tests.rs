//! Tests for mouse selection highlight helpers (issue #729).

use super::app_selection::{
    SelectionAnchor, TextSelection, apply_line_highlight, apply_selection_highlight,
    selection_range,
};

// ── selection_range ─────────────────────────────────────────────────────

#[test]
fn selection_range_forward_same_row() {
    let sel = TextSelection {
        start: SelectionAnchor { col: 2, row: 1 },
        end: SelectionAnchor { col: 8, row: 1 },
    };
    let (sr, sc, er, ec) = selection_range(&sel);
    assert_eq!((sr, sc, er, ec), (1, 2, 1, 8));
}

#[test]
fn selection_range_backward_same_row() {
    // Dragged right-to-left on the same row — start/end should be swapped.
    let sel = TextSelection {
        start: SelectionAnchor { col: 8, row: 1 },
        end: SelectionAnchor { col: 2, row: 1 },
    };
    let (sr, sc, er, ec) = selection_range(&sel);
    assert_eq!((sr, sc, er, ec), (1, 2, 1, 8));
}

#[test]
fn selection_range_forward_multi_row() {
    let sel = TextSelection {
        start: SelectionAnchor { col: 0, row: 1 },
        end: SelectionAnchor { col: 5, row: 3 },
    };
    let (sr, sc, er, ec) = selection_range(&sel);
    assert_eq!((sr, sc, er, ec), (1, 0, 3, 5));
}

#[test]
fn selection_range_backward_multi_row() {
    // Dragged bottom-to-top — rows should be swapped.
    let sel = TextSelection {
        start: SelectionAnchor { col: 5, row: 3 },
        end: SelectionAnchor { col: 0, row: 1 },
    };
    let (sr, sc, er, ec) = selection_range(&sel);
    assert_eq!((sr, sc, er, ec), (1, 0, 3, 5));
}

#[test]
fn selection_range_same_position() {
    // Start == end (a click without drag).
    let sel = TextSelection {
        start: SelectionAnchor { col: 3, row: 2 },
        end: SelectionAnchor { col: 3, row: 2 },
    };
    let (sr, sc, er, ec) = selection_range(&sel);
    assert_eq!((sr, sc, er, ec), (2, 3, 2, 3));
}

#[test]
fn selection_range_zero_cols_same_row() {
    let sel = TextSelection {
        start: SelectionAnchor { col: 0, row: 0 },
        end: SelectionAnchor { col: 0, row: 0 },
    };
    let (sr, sc, er, ec) = selection_range(&sel);
    assert_eq!((sr, sc, er, ec), (0, 0, 0, 0));
}

// ── apply_line_highlight ────────────────────────────────────────────────

#[test]
fn apply_line_highlight_plain_text() {
    let result = apply_line_highlight("hello world", 0, 5);
    assert!(
        result.contains("\x1b[7m"),
        "should contain reverse-video start"
    );
    assert!(
        result.contains("\x1b[27m"),
        "should contain reverse-video end"
    );
    // The visible text should still be "hello world" when ANSI is stripped.
    let plain = super::app_methods::strip_ansi(&result);
    assert_eq!(plain, "hello world");
}

#[test]
fn apply_line_highlight_partial_range() {
    let result = apply_line_highlight("hello world", 2, 7);
    assert!(result.contains("\x1b[7m"));
    assert!(result.contains("\x1b[27m"));
    // The reverse video should wrap "llo w" (cols 2-6).
    assert!(
        result.contains("\x1b[7mllo w\x1b[27m"),
        "should highlight cols 2-6: {result:?}"
    );
}

#[test]
fn apply_line_highlight_full_line() {
    let result = apply_line_highlight("hello", 0, 5);
    assert!(result.contains("\x1b[7mhello\x1b[27m"));
}

#[test]
fn apply_line_highlight_empty_range() {
    // start_col == end_col → no highlight.
    let result = apply_line_highlight("hello", 3, 3);
    assert_eq!(result, "hello");
}

#[test]
fn apply_line_highlight_start_after_end() {
    // start_col > end_col → no highlight.
    let result = apply_line_highlight("hello", 4, 2);
    assert_eq!(result, "hello");
}

#[test]
fn apply_line_highlight_with_ansi_escapes() {
    let input = "\x1b[31mred\x1b[0m text";
    let result = apply_line_highlight(input, 0, 8);
    // Should not break the ANSI escape, and should highlight visible chars.
    assert!(result.contains("\x1b[7m"));
    assert!(result.contains("\x1b[27m"));
    // The ANSI color codes should be preserved.
    assert!(result.contains("\x1b[31m"));
}

#[test]
fn apply_line_highlight_preserves_osc_sequences() {
    // OSC sequences (title setting) should pass through untouched.
    let input = "\x1b]0;title\x07hello";
    let result = apply_line_highlight(input, 0, 5);
    assert!(
        result.contains("\x1b]0;title\x07"),
        "OSC should be preserved"
    );
    assert!(result.contains("\x1b[7m"));
}

#[test]
fn apply_line_highlight_end_beyond_line_closes_highlight() {
    // If end_col > visible length, the highlight should still close.
    let result = apply_line_highlight("hi", 0, 100);
    assert!(result.contains("\x1b[7m"));
    assert!(result.contains("\x1b[27m"));
}

// ── apply_selection_highlight ───────────────────────────────────────────

#[test]
fn apply_selection_highlight_none_selection_no_change() {
    let mut lines = vec!["line0".to_string(), "line1".to_string()];
    apply_selection_highlight(&None, &mut lines, 0);
    assert_eq!(lines[0], "line0");
    assert_eq!(lines[1], "line1");
}

#[test]
fn apply_selection_highlight_single_row() {
    let mut lines = vec!["hello world".to_string()];
    let sel = Some(TextSelection {
        start: SelectionAnchor { col: 0, row: 0 },
        end: SelectionAnchor { col: 5, row: 0 },
    });
    apply_selection_highlight(&sel, &mut lines, 0);
    assert!(
        lines[0].contains("\x1b[7m"),
        "should have reverse video: {}",
        lines[0]
    );
}

#[test]
fn apply_selection_highlight_multi_row() {
    let mut lines = vec![
        "line zero".to_string(),
        "line one".to_string(),
        "line two".to_string(),
    ];
    let sel = Some(TextSelection {
        start: SelectionAnchor { col: 2, row: 0 },
        end: SelectionAnchor { col: 3, row: 2 },
    });
    apply_selection_highlight(&sel, &mut lines, 0);
    // All three rows should be highlighted.
    for (i, line) in lines.iter().enumerate() {
        assert!(
            line.contains("\x1b[7m"),
            "row {i} should have highlight: {line}"
        );
    }
}

#[test]
fn apply_selection_highlight_out_of_bounds_rows_ignored() {
    let mut lines = vec!["only line".to_string()];
    let sel = Some(TextSelection {
        start: SelectionAnchor { col: 0, row: 0 },
        end: SelectionAnchor { col: 5, row: 5 },
    });
    // Should not panic.
    apply_selection_highlight(&sel, &mut lines, 0);
    assert!(lines[0].contains("\x1b[7m"));
}

#[test]
fn apply_selection_highlight_empty_lines() {
    let mut lines: Vec<String> = vec![];
    let sel = Some(TextSelection {
        start: SelectionAnchor { col: 0, row: 0 },
        end: SelectionAnchor { col: 5, row: 0 },
    });
    // Should not panic.
    apply_selection_highlight(&sel, &mut lines, 0);
    assert!(lines.is_empty());
}

#[test]
fn apply_selection_highlight_backward_selection() {
    let mut lines = vec!["hello world".to_string()];
    // Drag right-to-left.
    let sel = Some(TextSelection {
        start: SelectionAnchor { col: 10, row: 0 },
        end: SelectionAnchor { col: 0, row: 0 },
    });
    apply_selection_highlight(&sel, &mut lines, 0);
    assert!(lines[0].contains("\x1b[7m"));
}

#[test]
fn apply_selection_highlight_first_row_partial() {
    let mut lines = vec!["abcdefghij".to_string(), "klmnopqrst".to_string()];
    // Select from col 5 of row 0 to col 5 of row 1.
    let sel = Some(TextSelection {
        start: SelectionAnchor { col: 5, row: 0 },
        end: SelectionAnchor { col: 5, row: 1 },
    });
    apply_selection_highlight(&sel, &mut lines, 0);
    // Row 0: highlight from col 5 to end.
    assert!(lines[0].contains("\x1b[7m"), "row 0 should be highlighted");
    // Row 1: highlight from col 0 to col 5.
    assert!(lines[1].contains("\x1b[7m"), "row 1 should be highlighted");
}

#[test]
fn multi_row_selection_highlight_stays_out_of_sidepanel() {
    let panel_prefix = "P".repeat(28);
    let mut lines = vec![
        format!("{panel_prefix}first body row"),
        format!("{panel_prefix}middle body row"),
        format!("{panel_prefix}final body row"),
    ];
    let sel = Some(TextSelection {
        start: SelectionAnchor { col: 30, row: 0 },
        end: SelectionAnchor { col: 35, row: 2 },
    });

    apply_selection_highlight(&sel, &mut lines, 28);

    for (row_idx, line) in lines.iter().enumerate() {
        assert_first_reverse_video_starts_at_or_after(line, 28, row_idx);
    }
}

fn assert_first_reverse_video_starts_at_or_after(line: &str, min_col: u16, row_idx: usize) {
    let Some(byte_idx) = line.find("\x1b[7m") else {
        panic!("row {row_idx} should be highlighted: {line:?}");
    };
    let visible_before_highlight = super::app_methods::strip_ansi(&line[..byte_idx]);
    let start_col = visible_before_highlight.chars().count() as u16;
    assert!(
        start_col >= min_col,
        "row {row_idx} highlight starts at visible col {start_col}, before body col {min_col}: {line:?}"
    );
}
