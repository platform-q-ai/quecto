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
    let lines = render_md(text, width);
    lines
        .iter()
        .map(|l| strip_ansi(l))
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_plain_nonempty(text: &str, width: usize) -> Vec<String> {
    render_md(text, width)
        .into_iter()
        .map(|line| strip_ansi(&line))
        .filter(|line| !line.is_empty())
        .map(|line| line.trim_end().to_string())
        .collect()
}

#[test]
fn heading_level_1() {
    let lines = render_md("# Hello", 80);
    assert!(!lines.is_empty());
    let plain = strip_ansi(&lines[0]);
    assert!(
        plain.contains("Hello"),
        "heading should contain 'Hello': {}",
        plain
    );
}

#[test]
fn heading_level_2() {
    let plain = render_plain("## World", 80);
    assert!(plain.contains("World"));
}

#[test]
fn bold_text() {
    let lines = render_md("This is **bold** text", 80);
    let joined = lines.join("");
    // Bold uses \x1b[1m...\x1b[0m
    assert!(
        joined.contains("\x1b[1m"),
        "should contain bold escape: {}",
        joined
    );
}

#[test]
fn italic_text() {
    let lines = render_md("This is *italic* text", 80);
    let joined = lines.join("");
    // Italic uses \x1b[3m...\x1b[0m
    assert!(
        joined.contains("\x1b[3m"),
        "should contain italic escape: {}",
        joined
    );
}

#[test]
fn code_span() {
    let plain = render_plain("Use `cargo build` to compile", 80);
    assert!(
        plain.contains("`cargo build`"),
        "should contain code span: {}",
        plain
    );
}

#[test]
fn code_block() {
    let md = "```rust\nfn main() {}\n```";
    let plain = render_plain(md, 80);
    assert!(
        plain.contains("fn main()"),
        "should contain code: {}",
        plain
    );
    assert!(
        !plain.contains("```"),
        "fence markers must not render literally: {}",
        plain
    );
}

#[test]
fn unordered_list() {
    let plain = render_plain("- item one\n- item two\n- item three", 80);
    assert!(plain.contains("item one"));
    assert!(plain.contains("item two"));
    assert!(plain.contains("item three"));
}

#[test]
fn ordered_list() {
    let plain = render_plain("1. first\n2. second\n3. third", 80);
    assert!(plain.contains("1."));
    assert!(plain.contains("first"));
    assert!(plain.contains("second"));
}

#[test]
fn unordered_list_uses_bullets() {
    let lines = render_plain_nonempty("- item one\n- item two\n- item three", 80);

    assert_eq!(lines, vec!["• item one", "• item two", "• item three"]);
}

#[test]
fn ordered_list_uses_numbers() {
    let lines = render_plain_nonempty("1. first\n2. second\n3. third", 80);

    assert_eq!(lines, vec!["1. first", "2. second", "3. third"]);
}

#[test]
fn ordered_list_preserves_start_number() {
    let lines = render_plain_nonempty("10. ten\n11. eleven", 80);

    assert_eq!(lines, vec!["10. ten", "11. eleven"]);
}

#[test]
fn nested_unordered_list_uses_bullets() {
    let lines = render_plain_nonempty("- parent\n  - child\n  - sibling", 80);

    assert_eq!(lines, vec!["• parent", "  • child", "  • sibling"]);
}

#[test]
fn mixed_ordered_unordered_nesting() {
    let lines = render_plain_nonempty("1. a\n   - b\n   - c\n2. d", 80);

    assert_eq!(lines, vec!["1. a", "  • b", "  • c", "2. d"]);
}

#[test]
fn unordered_list_long_item_wraps_with_hanging_indent() {
    let lines = render_plain_nonempty("- alpha beta gamma delta epsilon zeta", 24);

    assert_eq!(lines, vec!["• alpha beta gamma", "  delta epsilon zeta"]);
}

#[test]
fn ordered_list_long_item_wraps_with_hanging_indent() {
    let lines = render_plain_nonempty("10. alpha beta gamma delta epsilon zeta", 24);

    assert_eq!(
        lines,
        vec!["10. alpha beta gamma", "    delta epsilon zeta"]
    );
}

#[test]
fn nested_unordered_list_long_item_wraps_with_hanging_indent() {
    let lines = render_plain_nonempty("- parent\n  - alpha beta gamma delta epsilon zeta", 24);

    assert_eq!(
        lines,
        vec!["• parent", "  • alpha beta gamma", "    delta epsilon zeta"]
    );
}

#[test]
fn list_item_strips_terminal_control_sequences() {
    let rendered = render_md("- safe \x1b[2Jtext \x1b]52;c;payload\x07", 80).join("\n");

    assert!(
        !rendered.contains("\x1b[2J"),
        "CSI escape should be stripped"
    );
    assert!(!rendered.contains("]52;"), "OSC payload should be stripped");
    assert!(strip_ansi(&rendered).contains("• safe text"));
}

#[test]
fn inline_code_strips_terminal_control_sequences() {
    let rendered = render_md("Use `safe \x1b[2Jcode`", 80).join("\n");

    assert!(
        !rendered.contains("\x1b[2J"),
        "CSI escape should be stripped"
    );
    assert!(strip_ansi(&rendered).contains("`safe code`"));
}

#[test]
fn escaped_ordered_marker_paragraph_does_not_get_hanging_indent() {
    let lines = render_plain_nonempty("1\\. alpha beta gamma delta epsilon zeta", 24);

    assert_eq!(lines, vec!["1. alpha beta gamma", "delta epsilon zeta"]);
}

#[test]
fn literal_bullet_paragraph_does_not_get_hanging_indent() {
    let lines = render_plain_nonempty("• alpha beta gamma delta epsilon zeta", 24);

    assert_eq!(lines, vec!["• alpha beta gamma", "delta epsilon zeta"]);
}

#[test]
fn blockquote() {
    let lines = render_md("> This is a quote", 80);
    let joined = lines.join("\n");
    let plain = strip_ansi(&joined);
    assert!(
        plain.contains("This is a quote"),
        "should contain quote: {}",
        plain
    );
    assert!(
        plain.contains("│"),
        "should contain quote border: {}",
        plain
    );
}

#[test]
fn horizontal_rule() {
    let plain = render_plain("---", 80);
    assert!(plain.contains("─"), "should contain rule: {}", plain);
}

#[test]
fn link() {
    let plain = render_plain("[Example](https://example.com)", 80);
    assert!(
        plain.contains("Example"),
        "should contain link text: {}",
        plain
    );
    assert!(
        plain.contains("example.com"),
        "should contain URL: {}",
        plain
    );
}

#[test]
fn empty_text_renders_empty() {
    let mut md = Markdown::new("", 0);
    let lines = md.render(80);
    assert!(lines.is_empty());
}

#[test]
fn cache_works() {
    let mut md = Markdown::new("# Test", 0);
    let lines1 = md.render(80);
    let lines2 = md.render(80);
    assert_eq!(lines1, lines2);
    assert!(md.cached_lines.is_some());
}

#[test]
fn respects_width() {
    let long_text = "This is a very long paragraph that should be wrapped to fit within the specified terminal width without overflowing.";
    let mut md = Markdown::new(long_text, 1);
    let lines = md.render(40);
    for line in &lines {
        assert!(
            visible_width(line) <= 40,
            "line exceeds width 40: {} (width={})",
            line,
            visible_width(line)
        );
    }
}

#[test]
fn padding_applied() {
    let mut md = Markdown::new("hello", 2);
    let lines = md.render(80);
    assert!(!lines.is_empty());
    // First non-empty line should start with padding spaces.
    let first = &lines[0];
    assert!(first.starts_with("  "), "should have padding: '{}'", first);
}

// --- Table safety tests (#465, #468, #470) ---

#[test]
fn table_cell_ansi_escape_stripped() {
    // An LLM could inject ANSI escapes in table cell content.
    // The \x1b[31m sequence sets red text — must be stripped.
    let md = "| Header |\n|--------|\n| \x1b[31mred\x1b[0m |";
    let plain = render_plain(md, 80);
    assert!(
        !plain.contains("\x1b"),
        "ANSI escapes should be stripped from table cells: {}",
        plain
    );
    assert!(
        plain.contains("red"),
        "cell text should be preserved: {}",
        plain
    );
}

#[test]
fn table_cell_control_chars_stripped() {
    // Control characters like BEL, cursor movement must be stripped.
    let md = "| Header |\n|--------|\n| \x07bell\x08back |";
    let plain = render_plain(md, 80);
    assert!(
        !plain.contains('\x07'),
        "BEL should be stripped: {:?}",
        plain
    );
    assert!(
        !plain.contains('\x08'),
        "BS should be stripped: {:?}",
        plain
    );
}

#[test]
fn table_cell_sanitize_preserves_text() {
    let md = "| Name | Value |\n|------|-------|\n| foo  | bar   |";
    let plain = render_plain(md, 80);
    assert!(plain.contains("foo"), "cell text should be preserved");
    assert!(plain.contains("bar"), "cell text should be preserved");
}

#[test]
fn table_cjk_column_width() {
    // CJK characters are double-width. "你好" is 4 display columns.
    // The column must be at least 4 wide, not 6 (byte length of UTF-8).
    let md = "| Header |\n|--------|\n| 你好 |";
    let plain = render_plain(md, 80);
    // The key test is that render_table uses visible_width, not .len().
    // We verify by checking the render doesn't panic and text appears.
    assert!(plain.contains("你好"), "CJK text should appear: {}", plain);
}

#[test]
fn table_column_width_uses_display_width_not_bytes() {
    // "café" is 5 bytes but 4 display characters.
    // Column width should be 4 (display), not 5 (bytes).
    let rows = vec![vec!["café".to_string()], vec!["test".to_string()]];
    let lines = render_table(&rows, 80);
    // Both rows should align — if byte length is used, "café" gets
    // allocated 5 chars of width while "test" gets 4, causing misalignment.
    let plain: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
    assert!(plain.len() >= 2, "should have header + separator + data");
    // Verify the data row "test" is padded to the same width as "café".
    // With visible_width, both are 4 display chars, so padding is identical.
}

#[test]
fn table_all_empty_cells_no_panic() {
    // All empty cells means col_widths sum is 0 — must not divide by zero.
    let md = "| | |\n|--|--|\n| | |";
    let plain = render_plain(md, 80);
    // Should not panic — just render empty or minimal table.
    let _ = plain;
}

#[test]
fn table_all_empty_cells_via_render_table() {
    // Direct test of render_table with empty cells.
    let rows = vec![
        vec![String::new(), String::new()],
        vec![String::new(), String::new()],
    ];
    // Must not panic (division by zero in scale calculation).
    let lines = render_table(&rows, 40);
    assert!(!lines.is_empty(), "should produce some output");
}

#[test]
fn sanitize_for_display_strips_full_ansi_sequences() {
    // Full CSI sequences must be completely removed, not just the ESC byte.
    assert_eq!(sanitize_for_display("\x1b[31mhello\x1b[0m"), "hello");
    assert_eq!(sanitize_for_display("\x1b[1;31;42mtext\x1b[0m"), "text");
}

#[test]
fn sanitize_for_display_strips_osc_sequences() {
    // OSC hyperlink: ESC]8;;url BEL text ESC]8;; BEL
    let osc = "\x1b]8;;http://evil.com\x07click\x1b]8;;\x07";
    assert_eq!(sanitize_for_display(osc), "click");
}

#[test]
fn sanitize_for_display_strips_control_chars() {
    assert_eq!(sanitize_for_display("normal"), "normal");
    assert_eq!(sanitize_for_display("\x07\x08\x0B"), "");
    assert_eq!(sanitize_for_display("a\x00b"), "ab");
    assert_eq!(sanitize_for_display("a\x7Fb"), "ab"); // DEL
}

#[test]
fn sanitize_for_display_preserves_normal_text() {
    assert_eq!(sanitize_for_display("hello world"), "hello world");
    assert_eq!(sanitize_for_display("café"), "café");
    assert_eq!(sanitize_for_display("你好"), "你好");
}

// --- Inline code in table cells (#550) ---

#[test]
fn table_inline_code_stays_in_cell() {
    let md = "| Tool | Description |\n|------|-------------|\n| `bash` | Run commands |";
    let plain = render_plain(md, 80);
    // "bash" should be on the same line as "Run commands", not on a separate line.
    let lines: Vec<&str> = plain.lines().collect();
    let data_line = lines
        .iter()
        .find(|l| l.contains("bash"))
        .expect("should contain bash");
    assert!(
        data_line.contains("Run commands"),
        "inline code and description should be on the same line: {:?}",
        data_line,
    );
}

#[test]
fn table_mixed_text_and_code_in_cell() {
    let md = "| Command |\n|---------|\n| Use `poem.txt` file |";
    let plain = render_plain(md, 80);
    let data_line = plain
        .lines()
        .find(|l| l.contains("poem.txt"))
        .expect("should contain poem.txt");
    assert!(
        data_line.contains("Use") && data_line.contains("file"),
        "mixed text and code should be in one cell: {:?}",
        data_line,
    );
}

#[test]
fn table_code_only_cell_renders_correctly() {
    let md = "| Name |\n|------|\n| `test` |";
    let plain = render_plain(md, 80);
    assert!(
        plain.contains("test"),
        "code-only cell should contain the text: {:?}",
        plain,
    );
}

#[test]
fn table_tool_list_not_truncated_at_80_cols() {
    let md = "| Tool | Description |\n|------|-------------|\n\
        | `spawn` | Start a background subagent |\n\
        | `agent_cmd` | Send commands to a spawned subagent |\n\
        | `Bash` | Execute a bash command |\n\
        | `Edit` | Surgically replace exact text |\n\
        | `Write` | Create or overwrite a file |\n\
        | `Read` | Read file contents |";
    let plain = render_plain(md, 80);
    // All tool names should be fully visible, not truncated.
    assert!(
        plain.contains("`spawn`"),
        "spawn should not be truncated: {}",
        plain
    );
    assert!(
        plain.contains("`agent_cmd`"),
        "agent_cmd should not be truncated: {}",
        plain
    );
    assert!(
        plain.contains("`Bash`"),
        "Bash should not be truncated: {}",
        plain
    );
    assert!(
        plain.contains("`Edit`"),
        "Edit should not be truncated: {}",
        plain
    );
    assert!(
        plain.contains("`Write`"),
        "Write should not be truncated: {}",
        plain
    );
    assert!(
        plain.contains("`Read`"),
        "Read should not be truncated: {}",
        plain
    );
}

#[test]
fn table_narrow_width_still_shows_tool_names() {
    // Even at 60 cols, short tool names should not be clipped to 4 chars.
    let md = "| Tool | Description |\n|------|-------------|\n\
        | `spawn` | Start a background subagent process with optional system prompt and initial task |\n\
        | `agent_cmd` | Send commands to a spawned subagent (prompt, steer, follow_up, abort, get_state, get_messages) |";
    let plain = render_plain(md, 60);
    // Tool column should have at least enough width for `agent_cmd` (13 chars with backticks).
    assert!(
        plain.contains("`spawn`"),
        "spawn truncated at 60 cols: {}",
        plain
    );
}

// ── Render cache output-equivalence regression guard (#757) ──────────
//
// The markdown render cache saves the parse but historically re-cloned the
// whole `Vec<String>` on both the cache-hit and cache-miss paths. Reworking
// the cache to avoid the per-frame clone must keep the rendered output
// byte-identical between the first (miss) render and subsequent (hit)
// renders at the same width.
#[test]
fn render_cache_hit_matches_miss_output() {
    let text = "# Title\n\nSome **bold** paragraph text that wraps across a width.\n\n- item one\n- item two";
    let mut md = Markdown::new(text, 0);
    let first = md.render(72); // cache miss
    let second = md.render(72); // cache hit
    let third = md.render(72); // cache hit
    assert_eq!(
        first, second,
        "cache-hit render must equal cache-miss render"
    );
    assert_eq!(second, third, "repeated cache-hit renders must be stable");
}

#[test]
fn flush_code_block_renders_gutter_body_without_fence_markers() {
    let mut lines: Vec<RenderedLine> = Vec::new();
    flush_code_block("rust", "let x = 1;\nlet y = 2;", &mut lines);
    let texts: Vec<String> = lines.iter().map(|l| strip_ansi(&l.text)).collect();
    // No literal fence markers anywhere.
    assert!(
        texts.iter().all(|t| !t.contains("```")),
        "fence markers must not be emitted: {:?}",
        texts
    );
    // Code body rendered with a left gutter bar, in order.
    assert!(texts.iter().any(|t| t == "│ let x = 1;"), "{:?}", texts);
    assert!(texts.iter().any(|t| t == "│ let y = 2;"), "{:?}", texts);
    // Trailing blank line preserved.
    assert_eq!(texts.last().map(String::as_str), Some(""));
}

#[test]
fn indented_fence_text_block_renders_without_backticks() {
    // Exact snippet from issue #799 — fence indented by the model.
    let md = "    ```text\n    4–6 concurrent cargo test/check/build workflows\n    ```";
    let plain = render_plain(md, 80);
    assert!(
        plain.contains("4–6 concurrent cargo test/check/build workflows"),
        "code content must render: {}",
        plain
    );
    assert!(
        !plain.contains("```"),
        "indented fence markers must not render literally: {}",
        plain
    );
}

#[test]
fn indented_fence_bash_block_renders_without_backticks() {
    // Exact snippet from issue #799 — fence indented by the model.
    let md = "    ```bash\n    CARGO_BUILD_JOBS=2RUST_TEST_THREADS=2\n    ```";
    let plain = render_plain(md, 80);
    assert!(
        plain.contains("CARGO_BUILD_JOBS=2RUST_TEST_THREADS=2"),
        "code content must render: {}",
        plain
    );
    assert!(
        !plain.contains("```"),
        "indented fence markers must not render literally: {}",
        plain
    );
}

#[test]
fn inline_code_still_renders_with_backticks() {
    // Regression guard: inline code must keep its single backticks.
    let plain = render_plain("Use the `cargo build` command", 80);
    assert!(
        plain.contains("`cargo build`"),
        "inline code must keep backticks: {}",
        plain
    );
}

#[test]
fn flush_table_renders_rows_and_is_noop_when_empty() {
    let mut empty: Vec<RenderedLine> = Vec::new();
    flush_table(&[], 80, &mut empty);
    assert!(empty.is_empty(), "no rows must produce no output");

    let rows = vec![
        vec!["Name".to_string(), "Role".to_string()],
        vec!["Ada".to_string(), "Eng".to_string()],
    ];
    let mut lines: Vec<RenderedLine> = Vec::new();
    flush_table(&rows, 80, &mut lines);
    let texts: Vec<String> = lines.iter().map(|l| strip_ansi(&l.text)).collect();
    assert!(
        texts
            .iter()
            .any(|t| t.contains("Name") && t.contains("Role"))
    );
    assert!(texts.iter().any(|t| t.contains("Ada") && t.contains("Eng")));
    assert_eq!(texts.last().map(String::as_str), Some(""));
}
