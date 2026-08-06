//! Shared panel-chrome text helpers for test suites.
//!
//! The footer-hint filter and tree-stalk glyph set live here — next to the
//! render harness — so panel assertions in quecto-tui's unit tests, its BDD
//! suite, and the quecto-agentic-harness BDD suite all track the render
//! code from one place instead of drifting apart in verbatim copies.

/// A panel row's text after the selection column and tree-stalk characters —
/// the row's own label region, badge included.
pub fn after_stalk(row: &str) -> &str {
    row.trim_start_matches(['▌', ' ', '│', '├', '└'])
}

/// Column (in characters) where a panel row's own label region begins —
/// strictly larger for rows nested one level deeper than their parent row
/// (each nesting level adds an ancestor-continuation cell before the
/// connector), so tests can assert real nesting depth rather than mere row
/// ordering. Character-based so the one-cell selection marker (`▌` vs space)
/// cannot skew the comparison.
pub fn label_depth(row: &str) -> usize {
    row.chars().count() - after_stalk(row).chars().count()
}

/// Non-empty panel row lines, excluding the bottom key-hint line.
pub fn panel_rows(panel: &str) -> Vec<String> {
    panel
        .lines()
        .map(|l| l.trim_end().to_string())
        .filter(|l| !l.trim().is_empty() && !l.contains("⇥ pane"))
        .collect()
}
