use super::*;

// ── visible_width ─────────────────────────────────────────────────────

#[test]
fn plain_ascii() {
    assert_eq!(visible_width("hello"), 5);
}

#[test]
fn empty_string() {
    assert_eq!(visible_width(""), 0);
}

#[test]
fn ansi_color_ignored() {
    assert_eq!(visible_width("\x1b[31mred\x1b[0m"), 3);
}

#[test]
fn cjk_double_width() {
    assert_eq!(visible_width("日本語"), 6); // 3 chars × 2 width
}

#[test]
fn mixed_ansi_and_text() {
    assert_eq!(visible_width("\x1b[1m\x1b[32mHello\x1b[0m World"), 11);
}

#[test]
fn osc_hyperlink_ignored() {
    // OSC 8 hyperlink: \x1b]8;;url\x07text\x1b]8;;\x07
    let s = "\x1b]8;;https://example.com\x07link\x1b]8;;\x07";
    assert_eq!(visible_width(s), 4); // only "link" is visible
}

// ── truncate_to_width ─────────────────────────────────────────────────

#[test]
fn truncate_short_string_unchanged() {
    assert_eq!(truncate_to_width("hi", 10, None), "hi");
}

#[test]
fn truncate_with_ellipsis() {
    let result = truncate_to_width("Hello, World!", 8, Some("..."));
    assert!(visible_width(&result) <= 8);
    assert!(result.contains("..."));
}

#[test]
fn truncate_preserves_ansi() {
    let input = "\x1b[31mRedText\x1b[0m";
    let result = truncate_to_width(input, 3, None);
    // Should contain the ANSI start but truncated content
    assert!(result.contains("\x1b[31m"));
    assert!(visible_width(&result) <= 3);
}

// ── wrap_text ─────────────────────────────────────────────────────────

#[test]
fn wrap_short_text_single_line() {
    let lines = wrap_text("hello", 80);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0], "hello");
}

#[test]
fn wrap_at_word_boundary() {
    let lines = wrap_text("hello world", 6);
    assert!(lines.len() >= 2);
    for line in &lines {
        assert!(visible_width(line) <= 6);
    }
}

#[test]
fn wrap_long_word_breaks() {
    let lines = wrap_text("abcdefghij", 5);
    assert!(lines.len() >= 2);
    for line in &lines {
        assert!(visible_width(line) <= 5, "line '{}' is too wide", line);
    }
}

#[test]
fn visible_width_control_chars_zero() {
    assert_eq!(visible_width("\x01\x02\x03"), 0);
}

#[test]
fn visible_width_osc_with_st_terminator() {
    // OSC terminated with ST (\x1b\\)
    let s = "\x1b]0;title\x1b\\visible";
    assert_eq!(visible_width(s), 7); // "visible"
}

#[test]
fn truncate_to_zero_width() {
    let result = truncate_to_width("hello", 0, None);
    assert_eq!(visible_width(&result), 0);
}

#[test]
fn truncate_with_ellipsis_never_exceeds_width_when_ellipsis_too_wide() {
    let result = truncate_to_width("hello", 0, Some("..."));
    assert_eq!(visible_width(&result), 0);
    assert!(crate::components::ansi::strip_ansi(&result).is_empty());
}

#[test]
fn truncate_closes_active_osc8_when_cut_before_closer() {
    let input = "\x1b]8;;https://example.com\x07abcdef\x1b]8;;\x07";
    let result = truncate_to_width(input, 3, None);
    assert!(
        result.starts_with("\x1b]8;;https://example.com\x07abc"),
        "{result:?}"
    );
    assert!(result.ends_with("\x1b]8;;\x07"), "{result:?}");
}

#[test]
fn sanitize_truncate_width_does_not_scan_unbounded_prefix() {
    let input = format!("{}visible tail", "\x01".repeat(10_000));
    let result = sanitize_truncate_width_with_ellipsis(&input, 8, "…");
    assert_eq!(result, "visible…");
}

#[test]
fn truncate_exact_width() {
    let result = truncate_to_width("hello", 5, None);
    assert!(result.contains("hello"));
}

#[test]
fn truncate_cjk_respects_double_width() {
    let result = truncate_to_width("日本語テスト", 6, None);
    assert!(visible_width(&result) <= 6);
}

#[test]
fn wrap_long_word_preserves_sgr_active_before_word() {
    let lines = wrap_text("\x1b[31mabc defghijkl", 6);
    assert!(
        lines.len() > 1,
        "test must hard-wrap the long word: {lines:?}"
    );
    assert!(
        lines[1].starts_with("\x1b[31m"),
        "continuation should reopen active red SGR: {lines:?}"
    );
}

#[test]
fn wrap_empty_string() {
    let lines = wrap_text("", 80);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0], "");
}

#[test]
fn wrap_exact_width() {
    let lines = wrap_text("12345", 5);
    assert_eq!(lines.len(), 1);
}

#[test]
fn wrap_with_ansi() {
    let lines = wrap_text("\x1b[31mhello world\x1b[0m", 6);
    assert!(lines.len() >= 2);
}

#[test]
fn wrap_splits_on_embedded_newlines() {
    // A multi-line hyphenated list must become separate lines, never a
    // single line with embedded '\n' that the terminal staircases.
    let lines = wrap_text("- hi\n- how\n- you", 80);
    assert_eq!(lines, vec!["- hi", "- how", "- you"]);
    for line in &lines {
        assert!(
            !line.contains('\n'),
            "wrapped line leaked a newline: {line:?}"
        );
    }
}

#[test]
fn wrap_preserves_blank_lines_between_paragraphs() {
    let lines = wrap_text("a\n\nb", 80);
    assert_eq!(lines, vec!["a", "", "b"]);
}

#[test]
fn wrap_trailing_newline_yields_trailing_blank() {
    let lines = wrap_text("a\n", 80);
    assert_eq!(lines, vec!["a", ""]);
}

#[test]
fn wrap_each_segment_still_word_wraps() {
    // Newline splitting must not bypass width wrapping of long segments.
    let lines = wrap_text("hello world\nfoo", 6);
    assert!(lines.len() >= 3, "expected wrapped segments: {lines:?}");
    assert_eq!(lines.last().unwrap(), "foo");
    for line in &lines {
        assert!(visible_width(line) <= 6);
    }
}

#[test]
fn visible_width_emoji() {
    // Emoji characters are typically width 2
    let w = visible_width("🦀");
    assert!((1..=2).contains(&w));
}

#[test]
fn truncate_with_empty_ellipsis() {
    let result = truncate_to_width("hello world", 5, Some(""));
    assert!(visible_width(&result) <= 5);
}
