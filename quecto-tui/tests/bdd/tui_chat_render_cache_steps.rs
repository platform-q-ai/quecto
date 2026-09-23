//! Step definitions for `tui_chat_render_cache.feature` (#981).

use crate::TuiWorld;
use cucumber::{given, then, when};
use quecto_tui::components::ansi::strip_ansi;
use quecto_tui::components::chat::{Chat, ChatEntry};
use quecto_tui::components::component::Component;

const ENTRY_COUNT: usize = 200;
const VIEWPORT_HEIGHT: usize = 8;
const WIDTH: usize = 80;

fn render_plain(chat: &mut Chat) -> String {
    chat.render(WIDTH)
        .iter()
        .map(|line| strip_ansi(line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn long_history_chat() -> Chat {
    let mut chat = Chat::new();
    for i in 0..ENTRY_COUNT {
        chat.add_entry(ChatEntry::User {
            text: format!("history line {i}"),
        });
    }
    chat
}

fn chat(world: &mut TuiWorld) -> &mut Chat {
    world.tui_chat.as_mut().expect("chat initialized")
}

#[given("the chat transcript is much longer than the visible viewport")]
fn given_long_transcript(world: &mut TuiWorld) {
    let mut chat = long_history_chat();
    chat.set_viewport_height(VIEWPORT_HEIGHT);
    world.tui_chat = Some(chat);
    world.tui_chat_cache_tail = None;
}

#[given("the latest conversation window has been rendered")]
fn given_latest_rendered(world: &mut TuiWorld) {
    let tail = render_plain(chat(world));
    world.tui_chat_cache_tail = Some(tail);
}

#[when("the user scrolls back to older history")]
fn when_scrolls_back(world: &mut TuiWorld) {
    let chat = chat(world);
    chat.scroll_up(10_000);
    let older = render_plain(chat);
    world.tui_chat_cache_tail = Some(older);
}

#[then("the older conversation window is rendered correctly")]
fn then_older_window_renders(world: &mut TuiWorld) {
    let rendered = world
        .tui_chat_cache_tail
        .as_ref()
        .expect("older render captured");
    assert!(
        rendered.contains("history line 0"),
        "oldest history should re-render on demand after eviction: {rendered}"
    );
    assert!(
        chat(world).cached_rendered_line_count() <= chat(world).rendered_line_retention_bound()
    );
}

#[then("the scroll position still identifies the requested history")]
fn then_scroll_position_correct(world: &mut TuiWorld) {
    let total_lines = long_history_chat().render(WIDTH).len();
    assert_eq!(
        chat(world).scroll_offset(),
        total_lines - VIEWPORT_HEIGHT,
        "scroll offset should clamp to the oldest full viewport"
    );
}
