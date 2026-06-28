//! Tests for `chat_render.rs` helper functions and tool renderers (issue #729).

use super::*;
use std::borrow::Cow;

// ── extract_path ────────────────────────────────────────────────────────

#[test]
fn extract_path_from_path_field() {
    let args = Some(serde_json::json!({"path": "/src/main.rs"}));
    assert_eq!(extract_path(&args), "/src/main.rs");
}

#[test]
fn extract_path_from_file_path_field() {
    let args = Some(serde_json::json!({"file_path": "/lib.rs"}));
    assert_eq!(extract_path(&args), "/lib.rs");
}

#[test]
fn extract_path_none_args_returns_empty() {
    assert_eq!(extract_path(&None), "");
}

#[test]
fn extract_path_no_path_key_returns_empty() {
    let args = Some(serde_json::json!({"command": "ls"}));
    assert_eq!(extract_path(&args), "");
}

#[test]
fn extract_path_strips_control_chars() {
    let args = Some(serde_json::json!({"path": "src/\u{0007}main.rs"}));
    assert_eq!(extract_path(&args), "src/main.rs");
}

// ── extract_best_arg ────────────────────────────────────────────────────

#[test]
fn extract_best_arg_finds_command() {
    let v = serde_json::json!({"command": "ls -la"});
    assert_eq!(extract_best_arg(&v), "ls -la");
}

#[test]
fn extract_best_arg_finds_path() {
    let v = serde_json::json!({"path": "/etc/hosts"});
    assert_eq!(extract_best_arg(&v), "/etc/hosts");
}

#[test]
fn extract_best_arg_finds_query() {
    let v = serde_json::json!({"query": "SELECT 1"});
    assert_eq!(extract_best_arg(&v), "SELECT 1");
}

#[test]
fn extract_best_arg_finds_url() {
    let v = serde_json::json!({"url": "https://example.com"});
    assert_eq!(extract_best_arg(&v), "https://example.com");
}

#[test]
fn extract_best_arg_finds_content() {
    let v = serde_json::json!({"content": "hello world"});
    assert_eq!(extract_best_arg(&v), "hello world");
}

#[test]
fn extract_best_arg_priority_command_over_path() {
    let v = serde_json::json!({"command": "echo", "path": "/tmp"});
    assert_eq!(extract_best_arg(&v), "echo");
}

#[test]
fn extract_best_arg_empty_when_no_known_keys() {
    let v = serde_json::json!({"unknown": "value"});
    assert_eq!(extract_best_arg(&v), "");
}

#[test]
fn extract_best_arg_truncates_long_values() {
    let long = "x".repeat(100);
    let v = serde_json::json!({"command": long});
    let result = extract_best_arg(&v);
    assert!(result.ends_with("..."));
    assert!(result.chars().count() <= 63); // 60 + "..."
}

#[test]
fn extract_best_arg_strips_control_chars() {
    let v = serde_json::json!({"command": "echo\u{0007}hello"});
    assert_eq!(extract_best_arg(&v), "echohello");
}

// ── style_diff_line ─────────────────────────────────────────────────────

#[test]
fn style_diff_line_addition_is_green() {
    let result = style_diff_line("+added line");
    assert!(result.contains('\x1b'));
    assert!(result.contains("+added line"));
}

#[test]
fn style_diff_line_deletion_is_red() {
    let result = style_diff_line("-removed line");
    assert!(result.contains('\x1b'));
    assert!(result.contains("-removed line"));
}

#[test]
fn style_diff_line_context_is_default() {
    let result = style_diff_line(" context line");
    assert!(result.contains(" context line"));
}

// ── sanitize ────────────────────────────────────────────────────────────

#[test]
fn sanitize_strips_all_control_chars() {
    assert_eq!(sanitize("hello\u{0007}world\u{001b}"), "helloworld");
}

#[test]
fn sanitize_preserves_normal_text() {
    assert_eq!(sanitize("hello world 123"), "hello world 123");
}

#[test]
fn sanitize_empty_string() {
    assert_eq!(sanitize(""), "");
}

#[test]
fn sanitize_preserves_unicode() {
    assert_eq!(sanitize("héllo 世界"), "héllo 世界");
}

// ── expand_tabs ─────────────────────────────────────────────────────────

#[test]
fn expand_tabs_no_tabs_unchanged() {
    let s = "hello world";
    assert!(matches!(expand_tabs(s), Cow::Borrowed(_)));
    assert_eq!(expand_tabs(s), "hello world");
}

#[test]
fn expand_tabs_single_tab_at_start() {
    assert_eq!(expand_tabs("\thello"), "        hello");
}

#[test]
fn expand_tabs_tab_after_text() {
    // "ab" is 2 cols, tab advances to col 8 → 6 spaces
    assert_eq!(expand_tabs("ab\thello"), "ab      hello");
}

#[test]
fn expand_tabs_multiple_tabs() {
    // First tab → 8 spaces (col 0→8), second tab → 8 spaces (col 8→16)
    assert_eq!(expand_tabs("\t\t"), "                ");
}

#[test]
fn expand_tabs_tab_at_column_8() {
    // "abcdefgh" is 8 cols, tab advances to col 16 → 8 spaces
    assert_eq!(expand_tabs("abcdefgh\tx"), "abcdefgh        x");
}

#[test]
fn expand_tabs_preserves_ansi_escapes() {
    let input = "\x1b[31mred\x1b[0m\thello";
    let result = expand_tabs(input);
    assert!(result.contains("\x1b[31mred\x1b[0m"));
    assert!(result.contains("hello"));
    // Tab should have been expanded to spaces.
    assert!(!result.contains('\t'));
}

#[test]
fn expand_tabs_cjk_width_aware() {
    // '日' is width 2, so "日\t" → col 2, tab advances 6 to col 8
    let result = expand_tabs("日\thello");
    assert_eq!(result, "日      hello");
}

// ── truncate_with_ellipsis ──────────────────────────────────────────────

#[test]
fn truncate_short_string_no_ellipsis() {
    assert_eq!(truncate_with_ellipsis("hi", 10), "hi");
}

#[test]
fn truncate_exact_length_no_ellipsis() {
    assert_eq!(truncate_with_ellipsis("hello", 5), "hello");
}

#[test]
fn truncate_long_string_appends_ellipsis() {
    let result = truncate_with_ellipsis("hello world", 5);
    assert_eq!(result, "hello...");
}

#[test]
fn truncate_empty_string() {
    assert_eq!(truncate_with_ellipsis("", 5), "");
}

#[test]
fn truncate_zero_max_chars() {
    let result = truncate_with_ellipsis("hello", 0);
    assert_eq!(result, "...");
}

#[test]
fn truncate_counts_chars_not_bytes() {
    // "héllo" is 5 chars, 6 bytes — truncate at 3 chars
    let result = truncate_with_ellipsis("héllo", 3);
    assert_eq!(result, "hél...");
}

// ── render_tool_execution smoke tests ───────────────────────────────────

#[test]
fn render_tool_pending() {
    let lines = render_tool_execution(ToolRenderArgs {
        tool_name: "bash",
        args_json: &Some(serde_json::json!({"command": "ls"})),
        result: None,
        is_error: false,
        duration_ms: None,
        expanded: false,
        width: 80,
    });
    assert!(!lines.is_empty());
    // bash renders "$ ls" header; check for the command, not the tool name.
    let joined = lines.join("\n");
    assert!(joined.contains("$ ls"), "should contain command: {joined}");
}

#[test]
fn render_tool_success() {
    let lines = render_tool_execution(ToolRenderArgs {
        tool_name: "read",
        args_json: &Some(serde_json::json!({"path": "/etc/hosts"})),
        result: Some("file contents"),
        is_error: false,
        duration_ms: Some(42),
        expanded: false,
        width: 80,
    });
    assert!(!lines.is_empty());
    // read renders the path in the header.
    let joined = lines.join("\n");
    assert!(
        joined.contains("/etc/hosts"),
        "should contain path: {joined}"
    );
}

/// Security (#865): a sub-agent-influenced `agent_cmd` result body must have
/// its terminal control sequences stripped before rendering, so a malicious
/// sub-agent cannot inject ANSI/OSC escapes into the operator's terminal.
#[test]
fn render_tool_result_body_strips_terminal_control() {
    let lines = render_tool_execution(ToolRenderArgs {
        tool_name: "agent_cmd",
        args_json: &Some(serde_json::json!({"agent_id": "worker", "command": "get_state"})),
        // Injected: OSC 52 clipboard write + cursor move + title spoof.
        result: Some("status \u{1b}]52;c;ZXZpbA==\u{7}ok\u{1b}[2J\u{1b}]0;pwned\u{7}done"),
        is_error: false,
        duration_ms: Some(1),
        expanded: true,
        width: 200,
    });
    let joined = lines.join("\n");
    // Visible text survives; the raw injected escape introducers do not.
    assert!(joined.contains("status"), "visible text kept: {joined:?}");
    assert!(
        !joined.contains("\u{1b}]52"),
        "OSC clipboard escape must be stripped: {joined:?}"
    );
    assert!(
        !joined.contains("\u{1b}]0;"),
        "OSC title escape must be stripped: {joined:?}"
    );
    assert!(
        !joined.contains("\u{1b}[2J"),
        "CSI erase-screen escape must be stripped: {joined:?}"
    );
}

#[test]
fn render_tool_error() {
    let lines = render_tool_execution(ToolRenderArgs {
        tool_name: "write",
        args_json: &Some(serde_json::json!({"path": "/forbidden"})),
        result: Some("permission denied"),
        is_error: true,
        duration_ms: None,
        expanded: false,
        width: 80,
    });
    assert!(!lines.is_empty());
    let joined = lines.join("\n");
    assert!(
        joined.contains("/forbidden"),
        "should contain path: {joined}"
    );
}

#[test]
fn render_tool_expanded_includes_result() {
    let lines = render_tool_execution(ToolRenderArgs {
        tool_name: "bash",
        args_json: &Some(serde_json::json!({"command": "echo hi"})),
        result: Some("hi\n"),
        is_error: false,
        duration_ms: None,
        expanded: true,
        width: 80,
    });
    let joined = lines.join("\n");
    assert!(
        joined.contains("hi"),
        "expanded render should include result: {joined}"
    );
}

// ── render_file_preview ─────────────────────────────────────────────────

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
    let visible = crate::interface::utils::visible_width(&lines[0]);
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
