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

fn render_plain(chat: &mut Chat, width: usize) -> String {
    let lines = chat.render(width);
    lines
        .iter()
        .map(|l| strip_ansi(l))
        .collect::<Vec<_>>()
        .join("\n")
}

// ── Basic rendering ──────────────────────────────────────────────

#[test]
fn empty_chat_renders_empty() {
    let mut chat = Chat::new();
    assert!(chat.render(80).is_empty());
}

#[test]
fn user_message_rendered() {
    let mut chat = Chat::new();
    chat.add_entry(ChatEntry::User {
        text: "Hello".to_string(),
    });
    let plain = render_plain(&mut chat, 80);
    assert!(
        plain.contains("Hello"),
        "should contain user message: {}",
        plain
    );
}

#[test]
fn streaming_tokens() {
    let mut chat = Chat::new();
    chat.append_token("Hello");
    chat.append_token(" world");
    let plain = render_plain(&mut chat, 80);
    assert!(
        plain.contains("Hello world"),
        "should contain streamed text: {}",
        plain
    );
}

#[test]
fn finalize_assistant_stops_cursor() {
    let mut chat = Chat::new();
    chat.append_token("Done");
    chat.finalize_assistant();
    let lines = chat.render(80);
    let joined = lines.join("");
    assert!(
        !joined.contains('▌'),
        "finalized message should not have cursor"
    );
}

#[test]
fn entry_count() {
    let mut chat = Chat::new();
    assert_eq!(chat.entry_count(), 0);
    chat.add_entry(ChatEntry::User {
        text: "hi".to_string(),
    });
    assert_eq!(chat.entry_count(), 1);
}

// ── Unified tool execution (#510) ────────────────────────────────

#[test]
fn tool_start_creates_single_entry() {
    let mut chat = Chat::new();
    chat.start_tool(
        "c-1".into(),
        "bash".into(),
        r#"{"command":"ls -la"}"#.into(),
    );
    assert_eq!(chat.entry_count(), 1);
}

#[test]
fn tool_complete_updates_in_place() {
    let mut chat = Chat::new();
    chat.start_tool(
        "c-1".into(),
        "bash".into(),
        r#"{"command":"ls -la"}"#.into(),
    );
    chat.complete_tool("c-1", "file.txt", false, Some(42));
    // Still just one entry, not two.
    assert_eq!(chat.entry_count(), 1);
}

#[test]
fn bash_tool_shows_command_header() {
    let mut chat = Chat::new();
    chat.start_tool(
        "c-1".into(),
        "bash".into(),
        r#"{"command":"ls -la"}"#.into(),
    );
    chat.complete_tool("c-1", "file.txt", false, Some(42));
    let plain = render_plain(&mut chat, 80);
    assert!(plain.contains("$ ls -la"), "should show command: {}", plain);
    assert!(plain.contains("42ms"), "should show duration: {}", plain);
}

#[test]
fn bash_tool_shows_output() {
    let mut chat = Chat::new();
    chat.start_tool("c-1".into(), "bash".into(), r#"{"command":"ls"}"#.into());
    chat.complete_tool("c-1", "file1.txt\nfile2.txt", false, None);
    let plain = render_plain(&mut chat, 80);
    assert!(plain.contains("file1.txt"), "should show output: {}", plain);
}

#[test]
fn bash_collapsed_shows_tail() {
    let mut chat = Chat::new();
    chat.start_tool(
        "c-1".into(),
        "bash".into(),
        r#"{"command":"cargo test"}"#.into(),
    );
    let output: String = (0..50)
        .map(|i| format!("line {}", i))
        .collect::<Vec<_>>()
        .join("\n");
    chat.complete_tool("c-1", &output, false, None);
    let plain = render_plain(&mut chat, 80);
    // Should show the LAST lines (tail), not the first.
    assert!(
        plain.contains("line 49"),
        "should show last line: {}",
        plain
    );
    assert!(
        plain.contains("line 45"),
        "should show near-last line: {}",
        plain
    );
    // Should show count of hidden earlier lines.
    assert!(
        plain.contains("earlier lines"),
        "should show hidden count: {}",
        plain
    );
    assert!(
        plain.contains("Ctrl+O"),
        "should show expand hint: {}",
        plain
    );
    // Should NOT show early lines.
    assert!(
        !plain.contains("line 0"),
        "should NOT show first line: {}",
        plain
    );
}

#[test]
fn bash_expanded_shows_all() {
    let mut chat = Chat::new();
    chat.start_tool(
        "c-1".into(),
        "bash".into(),
        r#"{"command":"cargo test"}"#.into(),
    );
    let output: String = (0..50)
        .map(|i| format!("line {}", i))
        .collect::<Vec<_>>()
        .join("\n");
    chat.complete_tool("c-1", &output, false, None);
    chat.tool_expanded = true;
    let plain = render_plain(&mut chat, 80);
    assert!(
        plain.contains("line 0"),
        "should show first line: {}",
        plain
    );
    assert!(
        plain.contains("line 49"),
        "should show last line: {}",
        plain
    );
    assert!(
        !plain.contains("earlier lines"),
        "should NOT show hidden count: {}",
        plain
    );
}

#[test]
fn read_tool_shows_path_and_content() {
    let mut chat = Chat::new();
    chat.start_tool(
        "c-1".into(),
        "read".into(),
        r#"{"path":"src/main.rs"}"#.into(),
    );
    chat.complete_tool(
        "c-1",
        "fn main() {\n    println!(\"hello\");\n}",
        false,
        None,
    );
    let plain = render_plain(&mut chat, 80);
    assert!(plain.contains("read"), "should show tool name: {}", plain);
    assert!(plain.contains("src/main.rs"), "should show path: {}", plain);
    assert!(
        plain.contains("fn main()"),
        "should show content: {}",
        plain
    );
}

#[test]
fn read_collapsed_shows_head_with_count() {
    let mut chat = Chat::new();
    chat.start_tool("c-1".into(), "read".into(), r#"{"path":"big.rs"}"#.into());
    let content: String = (0..30)
        .map(|i| format!("line {}", i))
        .collect::<Vec<_>>()
        .join("\n");
    chat.complete_tool("c-1", &content, false, None);
    let plain = render_plain(&mut chat, 80);
    assert!(plain.contains("line 0"), "should show first line");
    assert!(plain.contains("line 9"), "should show 10th line");
    assert!(!plain.contains("line 10"), "should NOT show 11th line");
    assert!(plain.contains("more lines"), "should show count: {}", plain);
}

#[test]
fn write_tool_shows_path_and_content() {
    let mut chat = Chat::new();
    chat.start_tool(
        "c-1".into(),
        "write".into(),
        r#"{"path":"src/lib.rs","content":"pub fn hello() {}\n"}"#.into(),
    );
    chat.complete_tool("c-1", "Written successfully", false, None);
    let plain = render_plain(&mut chat, 80);
    assert!(plain.contains("write"), "should show tool name: {}", plain);
    assert!(plain.contains("src/lib.rs"), "should show path: {}", plain);
    assert!(
        plain.contains("pub fn hello"),
        "should show content: {}",
        plain
    );
}

#[test]
fn edit_tool_shows_diff() {
    let mut chat = Chat::new();
    chat.start_tool(
        "c-1".into(),
        "edit".into(),
        r#"{"path":"src/main.rs"}"#.into(),
    );
    chat.complete_tool("c-1", "+added\n-removed\n context", false, None);
    let lines = chat.render(80);
    let joined = lines.join("");
    // Green for added.
    assert!(joined.contains("\x1b[32m"), "added should be green");
    // Red for removed.
    assert!(joined.contains("\x1b[31m"), "removed should be red");
}

#[test]
fn edit_tool_shows_path() {
    let mut chat = Chat::new();
    chat.start_tool(
        "c-1".into(),
        "edit".into(),
        r#"{"path":"src/main.rs"}"#.into(),
    );
    chat.complete_tool("c-1", "+added", false, None);
    let plain = render_plain(&mut chat, 80);
    assert!(plain.contains("edit"), "should show tool name");
    assert!(plain.contains("src/main.rs"), "should show path: {}", plain);
}

#[test]
fn generic_tool_shows_name_and_summary() {
    let mut chat = Chat::new();
    chat.start_tool(
        "c-1".into(),
        "web_fetch".into(),
        r#"{"url":"https://example.com"}"#.into(),
    );
    chat.complete_tool("c-1", "HTML content here", false, None);
    let plain = render_plain(&mut chat, 80);
    assert!(
        plain.contains("web_fetch"),
        "should show tool name: {}",
        plain
    );
    assert!(
        plain.contains("https://example.com"),
        "should show url: {}",
        plain
    );
}

// ── Background colors ────────────────────────────────────────────

#[test]
fn running_tool_has_pending_bg() {
    let mut chat = Chat::new();
    chat.start_tool("c-1".into(), "bash".into(), r#"{"command":"ls"}"#.into());
    let lines = chat.render(80);
    let tool_lines: Vec<_> = lines.iter().filter(|l| !l.is_empty()).collect();
    assert!(!tool_lines.is_empty());
    assert!(
        tool_lines.iter().any(|l| l.contains(theme::BG_PENDING)),
        "should have pending bg: {:?}",
        tool_lines
    );
}

#[test]
fn success_tool_has_success_bg() {
    let mut chat = Chat::new();
    chat.start_tool("c-1".into(), "bash".into(), r#"{"command":"ls"}"#.into());
    chat.complete_tool("c-1", "ok", false, None);
    let lines = chat.render(80);
    let tool_lines: Vec<_> = lines.iter().filter(|l| !l.is_empty()).collect();
    assert!(
        tool_lines.iter().any(|l| l.contains(theme::BG_SUCCESS)),
        "should have success bg: {:?}",
        tool_lines
    );
}

#[test]
fn error_tool_has_error_bg() {
    let mut chat = Chat::new();
    chat.start_tool("c-1".into(), "bash".into(), r#"{"command":"ls"}"#.into());
    chat.complete_tool("c-1", "command not found", true, None);
    let lines = chat.render(80);
    let tool_lines: Vec<_> = lines.iter().filter(|l| !l.is_empty()).collect();
    assert!(
        tool_lines.iter().any(|l| l.contains(theme::BG_ERROR)),
        "should have error bg: {:?}",
        tool_lines
    );
}

// ── Subagent rendering ───────────────────────────────────────────

#[test]
fn spawn_tool_shows_agent_label() {
    let mut chat = Chat::new();
    chat.start_tool(
        "c-1".into(),
        "spawn".into(),
        r#"{"agent_id":"reviewer","task":"Review PR"}"#.into(),
    );
    let plain = render_plain(&mut chat, 80);
    assert!(plain.contains("reviewer"), "should show agent: {}", plain);
    assert!(plain.contains("Review PR"), "should show task: {}", plain);
}

#[test]
fn agent_cmd_shows_command_and_target() {
    let mut chat = Chat::new();
    chat.start_tool(
        "c-1".into(),
        "agent_cmd".into(),
        r#"{"command":"prompt","agent_id":"reviewer"}"#.into(),
    );
    let plain = render_plain(&mut chat, 80);
    assert!(plain.contains("prompt"), "should show command: {}", plain);
    assert!(plain.contains("reviewer"), "should show agent: {}", plain);
}

// ── Width compliance ─────────────────────────────────────────────

#[test]
fn tool_lines_respect_width() {
    let mut chat = Chat::new();
    chat.start_tool(
        "c-1".into(),
        "bash".into(),
        r#"{"command":"very long command string here"}"#.into(),
    );
    let output: String = (0..20)
        .map(|i| format!("line {} with some content here", i))
        .collect::<Vec<_>>()
        .join("\n");
    chat.complete_tool("c-1", &output, false, Some(42));
    let lines = chat.render(40);
    for line in &lines {
        assert!(
            visible_width(line) <= 40,
            "line exceeds width: {} (width={})",
            strip_ansi(line),
            visible_width(line)
        );
    }
}

#[test]
fn respects_width() {
    let mut chat = Chat::new();
    chat.add_entry(ChatEntry::User {
        text:
            "A very long message that should be wrapped to fit within the width constraint properly"
                .to_string(),
    });
    let lines = chat.render(40);
    for line in &lines {
        assert!(
            visible_width(line) <= 40,
            "line exceeds width: {} (width={})",
            strip_ansi(line),
            visible_width(line)
        );
    }
}

// ── Scroll tests (from #500) ─────────────────────────────────────

#[test]
fn scroll_offset_not_artificially_clamped() {
    let mut chat = Chat::new();
    let long_text = (0..100)
        .map(|i| format!("Line {}", i))
        .collect::<Vec<_>>()
        .join("\n");
    chat.add_entry(ChatEntry::Assistant {
        text: long_text,
        streaming: false,
    });
    chat.scroll_up(50);
    assert!(
        chat.scroll_offset >= 50,
        "scroll_offset should be at least 50, got {}",
        chat.scroll_offset
    );
}

fn chat_with_streaming_history() -> Chat {
    let mut chat = Chat::new();
    for i in 0..30 {
        chat.add_entry(ChatEntry::User {
            text: format!("history line {i}"),
        });
    }
    chat.append_token("initial streamed response");
    chat
}

#[test]
fn render_cache_reuses_unchanged_assistant_entry() {
    let mut chat = Chat::new();
    chat.add_entry(ChatEntry::Assistant {
        text: "# Cached\n\nunchanged markdown".to_string(),
        streaming: false,
    });

    let first = chat.render(80);
    let cached_after_first = chat.render_cache[0]
        .as_ref()
        .expect("first render should cache entry")
        .clone();
    let second = chat.render(80);

    assert_eq!(second, first);
    let cached_after_second = chat.render_cache[0]
        .as_ref()
        .expect("second render should keep cache");
    assert_eq!(cached_after_second.width, 80);
    assert_eq!(cached_after_second.lines, cached_after_first.lines);
}

#[test]
fn render_cache_invalidates_streaming_entry_when_token_appends() {
    let mut chat = Chat::new();
    chat.append_token("first");
    let _ = chat.render(80);
    assert!(chat.render_cache[0].is_some());

    chat.append_token(" second");

    assert!(
        chat.render_cache[0].is_none(),
        "appending to a streaming assistant entry must invalidate cached markdown"
    );
    let rendered = render_plain(&mut chat, 80);
    assert!(rendered.contains("first second"));
}

#[test]
fn render_cache_invalidates_tool_entry_on_completion() {
    let mut chat = Chat::new();
    chat.start_tool(
        "tool-1".into(),
        "bash".into(),
        r#"{"command":"echo hi"}"#.into(),
    );
    let _ = chat.render(80);
    assert!(chat.render_cache[0].is_some());

    chat.complete_tool("tool-1", "hi", false, Some(7));

    assert!(
        chat.render_cache[0].is_none(),
        "completed tool output must invalidate the pending tool render"
    );
    let rendered = render_plain(&mut chat, 80);
    assert!(rendered.contains("hi"));
}

#[test]
fn scroll_down_from_scrolled_position() {
    let mut chat = Chat::new();
    for i in 0..30 {
        chat.add_entry(ChatEntry::User {
            text: format!("Message number {}", i),
        });
    }
    let full = chat.render(80);
    chat.scroll_up(15);
    let scrolled_up = chat.render(80);
    assert!(scrolled_up.len() < full.len());
    chat.scroll_down(10);
    let after_down = chat.render(80);
    assert!(
        after_down.len() > scrolled_up.len(),
        "scrolling down should show more lines"
    );
}

#[test]
fn scrolled_streaming_viewport_stays_anchored_when_tokens_add_lines() {
    let mut chat = chat_with_streaming_history();

    let height = 10;
    chat.set_viewport_height(height);
    chat.scroll_up(15);
    let before = chat.render(80);

    chat.append_token("\nnew streamed line 1\nnew streamed line 2\nnew streamed line 3");
    let after = chat.render(80);

    assert_eq!(
        after, before,
        "streaming output should not drag a scrolled viewport toward the bottom"
    );
    assert!(
        chat.scroll_offset > 15,
        "scroll offset should grow with streamed content while scrolled away from bottom"
    );
}

#[test]
fn scrolled_viewport_stays_anchored_when_tool_entries_arrive() {
    let mut chat = chat_with_streaming_history();

    let height = 10;
    chat.set_viewport_height(height);
    chat.scroll_up(15);
    let before = chat.render(80);

    chat.start_tool(
        "tool-1".into(),
        "bash".into(),
        r#"{"command":"echo hi"}"#.into(),
    );
    let after = chat.render(80);

    assert_eq!(
        after, before,
        "tool output should not drag a scrolled viewport during an active response"
    );
}

#[test]
fn viewport_clamps_to_full_oldest_page_instead_of_blank() {
    let mut chat = chat_with_streaming_history();
    let height = 10;
    chat.set_viewport_height(height);
    chat.scroll_up(10_000);
    let before = chat.render(80);

    assert_eq!(
        before.len(),
        height,
        "oldest scrollback view should be full"
    );
    assert!(before.iter().any(|line| line.contains("history line 0")));

    chat.append_token("\nnew streamed line 1\nnew streamed line 2\nnew streamed line 3");
    let after = chat.render(80);

    assert_eq!(after.len(), height, "streaming should not shrink to blank");
    assert_eq!(after, before, "oldest full page should remain anchored");
}

// ── Tab expansion in tool output ─────────────────────────────────

#[test]
fn expand_tabs_no_tab_is_unchanged() {
    assert_eq!(expand_tabs("hello world"), "hello world");
}

#[test]
fn expand_tabs_leading_tab_fills_to_stop() {
    // Column 0 → next 8-col stop is 8 spaces.
    assert_eq!(expand_tabs("\tx"), format!("{}x", " ".repeat(8)));
}

#[test]
fn expand_tabs_mid_line_advances_to_next_stop() {
    // "ab" occupies cols 0-1; tab fills to col 8 (6 spaces).
    assert_eq!(expand_tabs("ab\tc"), format!("ab{}c", " ".repeat(6)));
}

#[test]
fn expand_tabs_multiple_tabs() {
    // col0 tab → 8 spaces (col 8); next tab → 8 spaces (col 16).
    assert_eq!(expand_tabs("\t\tx"), format!("{}x", " ".repeat(16)));
}

#[test]
fn expand_tabs_preserves_ansi_without_consuming_columns() {
    // The color escape must pass through and not affect tab-stop math.
    let input = "\x1b[31m\tx";
    assert_eq!(expand_tabs(input), format!("\x1b[31m{}x", " ".repeat(8)));
}

#[test]
fn expand_tabs_result_has_no_tab_characters() {
    let out = expand_tabs("a\tb\tc\td");
    assert!(!out.contains('\t'));
}

// ── Concatenation cache output-equivalence regression guard (#757) ───
//
// `Chat::render` rebuilds and re-clones the full conversation buffer on every
// frame. Caching the concatenated `all_lines` and invalidating only the
// changed tail (streaming/tool updates touch the suffix) must not change the
// rendered output: a cache-hit render must equal a cache-miss render, and a
// streamed token must only extend the tail while leaving prior lines intact.
#[test]
fn render_is_stable_across_repeated_calls() {
    let mut chat = Chat::new();
    chat.add_entry(ChatEntry::User {
        text: "hello".into(),
    });
    chat.add_entry(ChatEntry::Assistant {
        text: "world".into(),
        streaming: false,
    });
    let first = chat.render(80); // cache miss
    let second = chat.render(80); // cache hit (no changes)
    assert_eq!(
        first, second,
        "unchanged conversation must render identically"
    );
}

#[test]
fn streaming_append_only_extends_the_tail() {
    let mut chat = Chat::new();
    chat.add_entry(ChatEntry::User { text: "ask".into() });
    chat.append_token("partial answer");
    let before = render_plain(&mut chat, 80);
    chat.append_token(" continued");
    let after = render_plain(&mut chat, 80);
    // Every line above the streamed assistant tail (the user turn and the
    // blank separators) must be preserved verbatim; only the last assistant
    // line grows as tokens arrive. Comparing line vectors avoids coupling to
    // the trailing streaming-cursor glyph that lives on that final line.
    let before_lines: Vec<&str> = before.lines().collect();
    let after_lines: Vec<&str> = after.lines().collect();
    let head = before_lines.len() - 1;
    assert_eq!(
        before_lines[..head],
        after_lines[..head],
        "streaming should extend the tail, not rewrite prior history:\nbefore={before:?}\nafter={after:?}"
    );
    assert!(after.contains("partial answer continued"));
}
