//! Tests for file preview rendering helpers.

use super::*;

fn strip_ansi(s: &str) -> String {
    // Strip CSI sequences (\x1b[...m) and bare ESC.
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            // Skip the escape sequence.
            if chars.peek() == Some(&'[') {
                chars.next(); // consume '['
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            } else if chars.peek() == Some(&']') {
                chars.next();
                while let Some(c) = chars.next() {
                    if c == '\x07' {
                        break;
                    }
                    if c == '\x1b' {
                        // ST terminator: ESC backslash
                        if chars.peek() == Some(&'\\') {
                            chars.next();
                        }
                        break;
                    }
                }
            } else {
                chars.next();
            }
        } else {
            out.push(ch);
        }
    }
    out
}

#[test]
fn render_file_preview_short_content_all_lines() {
    let mut lines = Vec::new();
    render_file_preview(&mut lines, "line1\nline2\nline3", false, 80, false);
    assert_eq!(lines.len(), 3, "short content should show all lines");
    assert_eq!(strip_ansi(&lines[0]), "line1");
    assert_eq!(strip_ansi(&lines[1]), "line2");
    assert_eq!(strip_ansi(&lines[2]), "line3");
}

#[test]
fn render_file_preview_long_content_collapsed() {
    let content: String = (1..=20)
        .map(|i| format!("line{i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut lines = Vec::new();
    render_file_preview(&mut lines, &content, false, 80, false);
    // FILE_PREVIEW_LINES=10, so 10 content lines + 1 "more lines" hint = 11.
    assert_eq!(
        lines.len(),
        11,
        "collapsed long content should show 10 lines + hint"
    );
    let last = strip_ansi(&lines[10]);
    assert!(
        last.contains("10 more lines"),
        "should show remaining count: {last}"
    );
    assert!(last.contains("Ctrl+O"), "should mention expand key: {last}");
}

#[test]
fn render_file_preview_long_content_expanded() {
    let content: String = (1..=20)
        .map(|i| format!("line{i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut lines = Vec::new();
    render_file_preview(&mut lines, &content, true, 80, false);
    // Expanded → all 20 lines, no hint.
    assert_eq!(lines.len(), 20, "expanded should show all lines");
}

#[test]
fn render_file_preview_exactly_at_limit() {
    // FILE_PREVIEW_LINES=10 — content with exactly 10 lines should show all
    // (no hint, because total <= limit).
    let content: String = (1..=10)
        .map(|i| format!("line{i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut lines = Vec::new();
    render_file_preview(&mut lines, &content, false, 80, false);
    assert_eq!(lines.len(), 10, "content at limit should show all lines");
}

#[test]
fn render_file_preview_one_over_limit() {
    // 11 lines → 10 shown + 1 hint.
    let content: String = (1..=11)
        .map(|i| format!("line{i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut lines = Vec::new();
    render_file_preview(&mut lines, &content, false, 80, false);
    assert_eq!(lines.len(), 11, "11 lines → 10 + hint");
    let last = strip_ansi(&lines[10]);
    assert!(last.contains("1 more lines"), "should say 1 more: {last}");
}

#[test]
fn render_file_preview_empty_content() {
    let mut lines = Vec::new();
    render_file_preview(&mut lines, "", false, 80, false);
    // Empty string → .lines() yields zero items.
    assert!(lines.is_empty(), "empty content should produce no lines");
}

#[test]
fn render_file_preview_error_styling() {
    let mut lines = Vec::new();
    render_file_preview(&mut lines, "error line", false, 80, true);
    // Error lines should be styled with error color (red).
    assert!(
        lines[0].contains("\x1b[31m"),
        "error content should use red color: {}",
        lines[0]
    );
}

#[test]
fn render_file_preview_normal_styling() {
    let mut lines = Vec::new();
    render_file_preview(&mut lines, "normal line", false, 80, false);
    // Normal output should use tool_output color (not red).
    assert!(
        !lines[0].contains("\x1b[31m"),
        "non-error content should not use red: {}",
        lines[0]
    );
}

#[test]
fn render_file_preview_truncates_long_lines() {
    let long_line = "x".repeat(200);
    let mut lines = Vec::new();
    render_file_preview(&mut lines, &long_line, false, 20, false);
    // Should be truncated to width 20 (visible width).
    let visible = crate::components::utils::visible_width(&lines[0]);
    assert!(
        visible <= 20,
        "long line should be truncated to width 20, got {visible}: {}",
        lines[0]
    );
}

#[test]
fn render_file_preview_no_trailing_newline() {
    let mut lines = Vec::new();
    render_file_preview(&mut lines, "a\nb\nc", false, 80, false);
    assert_eq!(lines.len(), 3, "content without trailing newline");
    assert_eq!(strip_ansi(&lines[2]), "c");
}
