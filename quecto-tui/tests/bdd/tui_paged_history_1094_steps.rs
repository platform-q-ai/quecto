//! Step definitions for `tui_paged_history.feature` oversized #1094 cases.

use super::*;
use crate::tui_paged_history_steps::{
    PagedHistoryState, active_chat_text, drain, drive, get_messages_response, init_harness,
};

#[given("the TUI is attached to a session containing an oversized history message")]
fn given_attached_oversized_stub(world: &mut TuiWorld) {
    init_harness(world);
    let full = "abcdefghijklmnopqrstuvwxyz".to_string();
    world.tui_paged = PagedHistoryState {
        page_size: 3,
        stub_id: Some("oversized-stub".into()),
        stub_full: Some(full),
        ..Default::default()
    };
    let page = serde_json::json!({
        "messages": [
            {"id": "u0", "role": "user", "content": "a question"},
            {
                "id": "oversized-stub",
                "role": "assistant",
                "content": "[assistant stub — recall available]",
                "collapsed": true,
                "contentLength": 26,
            },
        ],
        "before": null,
        "hasMoreBefore": false,
    });
    drive(world, |h| {
        h.event(get_messages_response("attach-backfill", page));
    });
}

#[then("the TUI should retrieve the oversized history message without disconnecting")]
fn then_oversized_retrieve_without_disconnect(world: &mut TuiWorld) {
    let full = world.tui_paged.stub_full.clone().unwrap();
    let frame = active_chat_text(world);
    assert!(
        frame.contains(&full),
        "oversized recalled content should be visible without disconnecting:\n{frame}"
    );
}

#[then("the TUI should keep the session connection open")]
fn then_tui_session_connection_open(world: &mut TuiWorld) {
    let _ = drain(world);
    drive(world, |h| {
        h.submit("connection probe after oversized recall");
    });
    let commands = drain(world);
    assert!(
        commands
            .iter()
            .any(|line| serde_json::from_str::<serde_json::Value>(line)
                .ok()
                .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(str::to_string))
                == Some("prompt".into())),
        "the live command channel must accept a post-recall prompt probe; commands={commands:?}"
    );
}
