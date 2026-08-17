use super::*;
use crate::components::ansi::strip_ansi;

fn render_plain(chat: &mut Chat, width: usize) -> String {
    let lines = chat.render(width);
    lines
        .iter()
        .map(|l| strip_ansi(l))
        .collect::<Vec<_>>()
        .join("\n")
}

fn chat_with_long_history(entry_count: usize) -> Chat {
    let mut chat = Chat::new();
    for i in 0..entry_count {
        chat.add_entry(ChatEntry::User {
            text: format!("history line {i}"),
        });
    }
    chat
}

/// Full uncached render of the same history, ANSI-stripped.
fn baseline_lines(entry_count: usize, width: usize) -> Vec<String> {
    chat_with_long_history(entry_count)
        .render(width)
        .into_iter()
        .map(|l| strip_ansi(&l))
        .collect()
}

#[test]
fn long_viewport_render_keeps_rendered_line_cache_bounded() {
    let mut chat = chat_with_long_history(200);
    chat.set_viewport_height(8);

    let latest = render_plain(&mut chat, 80);

    assert!(
        latest.contains("history line 199"),
        "latest viewport should render the tail of the transcript: {latest}"
    );
    assert_eq!(
        chat.entry_count(),
        200,
        "cache eviction must not truncate raw transcript entries"
    );
    assert!(
        chat.cached_rendered_line_count() <= chat.rendered_line_retention_bound(),
        "render cache should stay within the retention window, cached {} of max {}",
        chat.cached_rendered_line_count(),
        chat.rendered_line_retention_bound()
    );
}

#[test]
fn evicted_history_rerenders_when_scrolled_back_without_losing_position() {
    let height = 8;
    let mut chat = chat_with_long_history(200);
    chat.set_viewport_height(height);
    let _ = chat.render(80);

    chat.scroll_up(10_000);
    let oldest = render_plain(&mut chat, 80);

    assert!(
        oldest.contains("history line 0"),
        "oldest history should re-render on demand after eviction: {oldest}"
    );
    let total_lines = baseline_lines(200, 80).len();
    assert_eq!(
        chat.scroll_offset(),
        total_lines - height,
        "scroll offset should clamp to the oldest full viewport"
    );
    assert!(
        chat.cached_rendered_line_count() <= chat.rendered_line_retention_bound(),
        "scrollback re-render should not repopulate the whole transcript cache"
    );
}

#[test]
fn large_entry_retains_only_nearby_rendered_lines() {
    let mut chat = Chat::new();
    let long_message = (0..1_000)
        .map(|i| format!("wrapped history line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    chat.add_entry(ChatEntry::Assistant {
        text: long_message,
        thinking: Vec::new(),
        streaming: false,
    });
    chat.set_viewport_height(8);

    let tail = render_plain(&mut chat, 80);

    assert!(
        tail.contains("wrapped history line 999"),
        "large entry tail should render: {tail}"
    );
    assert!(
        chat.cached_rendered_line_count() <= chat.rendered_line_retention_bound(),
        "large overlapping entries should retain only the viewport window, cached {} of max {}",
        chat.cached_rendered_line_count(),
        chat.rendered_line_retention_bound()
    );
}

#[test]
fn cache_eviction_preserves_visible_transcript_content() {
    let mut bounded = chat_with_long_history(200);
    bounded.set_viewport_height(8);
    let bounded_tail = render_plain(&mut bounded, 80);
    assert!(
        bounded.cached_rendered_line_count() <= bounded.rendered_line_retention_bound(),
        "render cache should be bounded before comparing visible content"
    );

    let full_lines = baseline_lines(200, 80);
    let expected_tail = full_lines[full_lines.len() - 8..].join("\n");

    assert_eq!(
        bounded_tail, expected_tail,
        "cache eviction must not change visible tail content"
    );

    bounded.scroll_up(64);
    let bounded_history = render_plain(&mut bounded, 80);
    assert!(
        bounded.cached_rendered_line_count() <= bounded.rendered_line_retention_bound(),
        "history comparison should also keep the render cache bounded"
    );
    let history_end = full_lines.len() - 64;
    let expected_history = full_lines[history_end - 8..history_end].join("\n");

    assert_eq!(
        bounded_history, expected_history,
        "on-demand history rendering must match a normal render"
    );
}

#[test]
fn width_change_after_eviction_matches_uncached_render() {
    let mut chat = chat_with_long_history(200);
    chat.set_viewport_height(8);
    let _ = chat.render(80);

    let narrow_tail = render_plain(&mut chat, 60);

    let full_lines = baseline_lines(200, 60);
    let expected_tail = full_lines[full_lines.len() - 8..].join("\n");
    assert_eq!(
        narrow_tail, expected_tail,
        "a width change after eviction must match an uncached render"
    );
    assert!(
        chat.cached_rendered_line_count() <= chat.rendered_line_retention_bound(),
        "a full dims-change rebuild must not repopulate the whole transcript cache, cached {}",
        chat.cached_rendered_line_count()
    );
}

#[test]
fn completing_a_tool_mid_transcript_after_eviction_keeps_content() {
    let build = |completed: bool| {
        let mut chat = Chat::new();
        chat.start_tool("t1".into(), "bash".into(), r#"{"command":"ls"}"#.into());
        if completed {
            chat.complete_tool("t1", "a\nb\nc", false, Some(7));
        }
        for i in 0..200 {
            chat.add_entry(ChatEntry::User {
                text: format!("history line {i}"),
            });
        }
        chat
    };

    let mut chat = build(false);
    chat.set_viewport_height(8);
    let _ = chat.render(80);
    chat.complete_tool("t1", "a\nb\nc", false, Some(7));

    chat.scroll_up(10_000);
    let top = render_plain(&mut chat, 80);

    let full_lines: Vec<String> = build(true)
        .render(80)
        .into_iter()
        .map(|l| strip_ansi(&l))
        .collect();
    let expected_top = full_lines[..8].join("\n");
    assert_eq!(
        top, expected_top,
        "completing a tool above evicted history must not corrupt offsets"
    );
    assert!(
        chat.cached_rendered_line_count() <= chat.rendered_line_retention_bound(),
        "mid-transcript invalidation must keep the render cache bounded"
    );
}

#[test]
fn scrolling_back_down_restores_the_live_tail() {
    let mut chat = chat_with_long_history(200);
    chat.set_viewport_height(8);
    let original_tail = render_plain(&mut chat, 80);

    chat.scroll_up(10_000);
    let _ = chat.render(80);
    chat.scroll_down(10_000);
    let restored_tail = render_plain(&mut chat, 80);

    assert_eq!(
        restored_tail, original_tail,
        "returning to the tail after scrollback must re-render it identically"
    );
    assert!(
        chat.cached_rendered_line_count() <= chat.rendered_line_retention_bound(),
        "round-trip scrolling must keep the render cache bounded"
    );
}

#[test]
fn scrolling_within_a_tall_entry_reuses_the_margin_instead_of_rerendering() {
    let mut chat = Chat::new();
    let long_message = (0..1_000)
        .map(|i| format!("wrapped history line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    chat.add_entry(ChatEntry::Assistant {
        text: long_message,
        thinking: Vec::new(),
        streaming: false,
    });
    let height = 8;
    chat.set_viewport_height(height);
    let _ = chat.render(80);
    chat.scroll_up(500);
    let _ = chat.render(80);

    chat.entry_builds = 0;
    for _ in 0..height {
        chat.scroll_up(1);
        let _ = chat.render(80);
    }
    assert_eq!(
        chat.entry_builds, 0,
        "scroll steps within the retention margin must not re-render the tall entry"
    );

    // Exhausting the margin re-renders once, then the margin refills.
    chat.scroll_up(height * (RENDER_CACHE_RETAIN_VIEWPORTS + 1));
    let _ = chat.render(80);
    assert!(
        chat.entry_builds <= 1,
        "moving past the margin should cost at most one amortized re-render, got {}",
        chat.entry_builds
    );
}

#[test]
fn toggling_tool_expand_after_eviction_matches_uncached_render() {
    let build = || {
        let mut chat = Chat::new();
        for i in 0..100 {
            chat.start_tool(format!("t{i}"), "bash".into(), r#"{"command":"ls"}"#.into());
            chat.complete_tool(&format!("t{i}"), "a\nb\nc\nd\ne", false, Some(7));
        }
        chat
    };

    let mut chat = build();
    chat.set_viewport_height(8);
    let _ = chat.render(80);
    chat.toggle_tool_expand();
    chat.scroll_up(30);
    let expanded_window = render_plain(&mut chat, 80);

    let mut baseline = build();
    baseline.toggle_tool_expand();
    let full_lines: Vec<String> = baseline
        .render(80)
        .into_iter()
        .map(|l| strip_ansi(&l))
        .collect();
    let window_end = full_lines.len() - 30;
    let expected = full_lines[window_end - 8..window_end].join("\n");
    assert_eq!(
        expanded_window, expected,
        "expanding tools after eviction must match an uncached expanded render"
    );
    assert!(
        chat.cached_rendered_line_count() <= chat.rendered_line_retention_bound(),
        "tool expansion must keep the render cache bounded"
    );
}

#[test]
fn streaming_with_eviction_stays_single_render_and_keeps_scroll_anchor() {
    let mut chat = chat_with_long_history(200);
    chat.set_viewport_height(8);
    chat.append_token("streamed start");
    let _ = chat.render(80);

    chat.entry_builds = 0;
    chat.append_token(" and more");
    let tail = render_plain(&mut chat, 80);
    assert!(
        tail.contains("streamed start and more"),
        "streamed tail should render: {tail}"
    );
    assert_eq!(
        chat.entry_builds, 1,
        "a streamed token must re-render only the tail entry"
    );
    assert!(
        chat.cached_rendered_line_count() <= chat.rendered_line_retention_bound(),
        "streaming must keep the render cache bounded"
    );

    // While scrolled into history, appended tokens must not move the viewport.
    chat.scroll_up(40);
    let anchored = render_plain(&mut chat, 80);
    chat.append_token(" trailing growth");
    let after_growth = render_plain(&mut chat, 80);
    assert_eq!(
        anchored, after_growth,
        "streaming growth below the viewport must not move an anchored scrollback view"
    );
}

#[test]
fn prepend_history_after_eviction_renders_prepended_content() {
    let mut chat = chat_with_long_history(200);
    chat.set_viewport_height(8);
    let original_tail = render_plain(&mut chat, 80);

    let prepended: Vec<ChatEntry> = (0..50)
        .map(|i| ChatEntry::User {
            text: format!("prepended line {i}"),
        })
        .collect();
    chat.prepend_history(prepended);

    let tail = render_plain(&mut chat, 80);
    assert_eq!(
        tail, original_tail,
        "prepending history must not disturb the visible tail"
    );

    chat.scroll_up(10_000);
    let top = render_plain(&mut chat, 80);
    assert!(
        top.contains("prepended line 0"),
        "prepended history should render at the top after eviction: {top}"
    );
    assert!(
        chat.cached_rendered_line_count() <= chat.rendered_line_retention_bound(),
        "prepend + scrollback must keep the render cache bounded"
    );
}

#[test]
fn replace_history_prefix_supersedes_partial_without_disturbing_live_tail() {
    // #1050: a trimmed busy-connect snapshot is later replaced by a fuller
    // attach-backfill; the prefix swap must keep the live stream intact.
    let mut chat = Chat::new();
    chat.add_entry(ChatEntry::User {
        text: "partial only".into(),
    });
    chat.add_entry(ChatEntry::Assistant {
        text: "partial reply".into(),
        thinking: Vec::new(),
        streaming: false,
    });
    chat.append_token("LIVE_TAIL");
    chat.set_viewport_height(8);
    let _ = render_plain(&mut chat, 80);

    chat.replace_history_prefix(
        2,
        vec![
            ChatEntry::User {
                text: "oldest restored".into(),
            },
            ChatEntry::Assistant {
                text: "oldest reply".into(),
                thinking: Vec::new(),
                streaming: false,
            },
            ChatEntry::User {
                text: "partial only".into(),
            },
            ChatEntry::Assistant {
                text: "partial reply".into(),
                thinking: Vec::new(),
                streaming: false,
            },
        ],
    );

    assert_eq!(chat.entry_count(), 5, "prefix grows; live token stays");
    let frame = render_plain(&mut chat, 80);
    assert!(
        frame.contains("LIVE_TAIL"),
        "live tail must survive prefix replace: {frame}"
    );
    chat.scroll_up(10_000);
    let top = render_plain(&mut chat, 80);
    assert!(
        top.contains("oldest restored"),
        "fuller prefix must render at the top: {top}"
    );
}

#[test]
fn prepend_history_merges_split_tool_result_at_page_boundary() {
    let mut chat = Chat::new();
    chat.add_entry(ChatEntry::ToolExecution {
        tool_call_id: "call-1".into(),
        tool_name: "bash".into(),
        parsed_args: None,
        args: String::new(),
        result: Some("newest page result".into()),
        is_error: false,
        duration_ms: None,
    });
    chat.prepend_history(vec![ChatEntry::ToolExecution {
        tool_call_id: "call-1".into(),
        tool_name: "bash".into(),
        parsed_args: serde_json::from_str(r#"{"command":"older call"}"#).ok(),
        args: r#"{"command":"older call"}"#.into(),
        result: None,
        is_error: false,
        duration_ms: None,
    }]);
    let text = chat
        .render(120)
        .iter()
        .map(|l| strip_ansi(l))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(text.matches("$ older call").count(), 1, "{text}");
    assert_eq!(text.matches("newest page result").count(), 1, "{text}");
}
