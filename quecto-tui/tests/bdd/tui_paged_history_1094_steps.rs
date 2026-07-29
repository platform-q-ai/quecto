//! Step definitions for `tui_paged_history.feature` oversized #1094 cases.

use super::*;
use crate::tui_paged_history_steps::{
    PagedHistoryState, active_chat_text, arm_own_get_messages_if_needed, drain, drive,
    get_messages_response, init_harness,
};

#[given("the TUI is attached to a session containing an oversized history message")]
fn given_attached_oversized_stub(world: &mut TuiWorld) {
    init_harness(world);
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let full: String = (0..quecto_line_io::PROTOCOL_LINE_CAP_BYTES + 1024)
        .map(|idx| ALPHABET[idx % ALPHABET.len()] as char)
        .collect();
    let full_len = full.len();
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
                "contentLength": full_len,
            },
        ],
        "before": null,
        "hasMoreBefore": false,
    });
    drive(world, |h| {
        arm_own_get_messages_if_needed(h, "attach-backfill");
        h.event(get_messages_response("attach-backfill", page));
    });
}

#[then("the TUI should retrieve the oversized history message without disconnecting")]
fn then_oversized_retrieve_without_disconnect(world: &mut TuiWorld) {
    // Observe the APP's reassembled transcript body, not a test-local fixture.
    let full = world
        .tui_paged
        .stub_full
        .clone()
        .expect("oversized fixture");
    let texts = drive(world, |h| h.master_assistant_texts());
    assert!(
        texts.iter().any(|text| text == &full),
        "all bounded pages must be delivered and reassembled exactly; \
         got assistant entry lengths {:?} (expected one of length {})",
        texts.iter().map(String::len).collect::<Vec<_>>(),
        full.len()
    );
    let frame = active_chat_text(world);
    assert!(
        !frame.contains("recall available"),
        "the recalled body must replace the collapsed stub"
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
