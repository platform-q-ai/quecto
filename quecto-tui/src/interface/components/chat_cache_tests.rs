use super::*;
use crate::interface::ansi::strip_ansi;

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

#[test]
fn long_viewport_render_keeps_rendered_line_cache_bounded() {
    let mut chat = chat_with_long_history(200);
    let height = 8;
    chat.set_viewport_height(height);

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
        chat.cached_rendered_line_count() <= height * 8,
        "render cache should be bounded near the viewport, cached {} rendered lines",
        chat.cached_rendered_line_count()
    );
}

#[test]
fn evicted_history_rerenders_when_scrolled_back_without_losing_position() {
    let mut chat = chat_with_long_history(200);
    let height = 8;
    chat.set_viewport_height(height);
    let _ = chat.render(80);

    chat.scroll_up(10_000);
    let oldest = render_plain(&mut chat, 80);

    assert!(
        oldest.contains("history line 0"),
        "oldest history should re-render on demand after eviction: {oldest}"
    );
    assert_eq!(
        chat.scroll_offset(),
        392,
        "scroll offset should clamp to the oldest full viewport"
    );
    assert!(
        chat.cached_rendered_line_count() <= height * 8,
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
        streaming: false,
    });
    chat.set_viewport_height(8);

    let tail = render_plain(&mut chat, 80);

    assert!(
        tail.contains("wrapped history line 999"),
        "large entry tail should render: {tail}"
    );
    assert!(
        chat.cached_rendered_line_count() <= 8 * 5,
        "large overlapping entries should retain only the viewport window, cached {} rendered lines",
        chat.cached_rendered_line_count()
    );
}

#[test]
fn cache_eviction_preserves_visible_transcript_content() {
    let mut bounded = chat_with_long_history(200);
    bounded.set_viewport_height(8);
    let bounded_tail = render_plain(&mut bounded, 80);
    assert!(
        bounded.cached_rendered_line_count() <= 8 * 8,
        "render cache should be bounded before comparing visible content"
    );

    let mut uncached = chat_with_long_history(200);
    let full_lines: Vec<String> = uncached
        .render(80)
        .into_iter()
        .map(|l| strip_ansi(&l))
        .collect();
    let expected_tail = full_lines[full_lines.len() - 8..].join("\n");

    assert_eq!(
        bounded_tail, expected_tail,
        "cache eviction must not change visible tail content"
    );

    bounded.scroll_up(64);
    let bounded_history = render_plain(&mut bounded, 80);
    assert!(
        bounded.cached_rendered_line_count() <= 8 * 8,
        "history comparison should also keep the render cache bounded"
    );
    let history_end = full_lines.len() - 64;
    let expected_history = full_lines[history_end - 8..history_end].join("\n");

    assert_eq!(
        bounded_history, expected_history,
        "on-demand history rendering must match a normal render"
    );
}
