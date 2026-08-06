//! Environment selection shows container info ONLY in the main pane
//! (#1369 follow-up), split from `app_subagent_environment_tests.rs`
//! (750-line cap). Drives the REAL render path through the headless harness.

use super::app_subagent_environment_tests::{env_agent_json, state_changed_line};
use super::tui_harness::*;
use crate::protocol::client::Event;
use crate::shell::keys::Key;

// ── Environment selection shows container info ONLY (#1369 follow-up) ──────

/// Seed the master conversation with a distinctive marker via the real
/// token-stream + turn-end path, so suppression can be asserted against real
/// transcript content rather than an empty chat.
fn seed_master_transcript(h: &mut TuiHarness) {
    h.event(Event::Token {
        token: "PARENT_CONVERSATION_MARKER".to_string(),
    });
    h.event_line(
        &serde_json::json!({
            "type": "turn_end",
            "message": {"role": "assistant", "content": "PARENT_CONVERSATION_MARKER"},
        })
        .to_string(),
    );
}

#[tokio::test]
async fn selecting_an_environment_row_suppresses_the_parent_conversation() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    seed_master_transcript(&mut h);
    h.event_line(&state_changed_line(vec![
        env_agent_json("impl", "C2"),
        env_agent_json("rev", "C2"),
    ]));
    assert!(
        h.main_pane().contains("PARENT_CONVERSATION_MARKER"),
        "precondition: the master transcript renders before selection"
    );

    let rows = panel_rows(&h.left_panel());
    let target = rows
        .iter()
        .position(|l| l.contains("C2") && !l.contains("impl") && !l.contains("rev"))
        .expect("selectable environment row");
    h.press(Key::Tab);
    for _ in 0..target {
        h.press(Key::Down);
    }
    h.press(Key::Enter);

    let top = h.main_pane();
    assert!(
        !top.contains("PARENT_CONVERSATION_MARKER"),
        "no parent transcript may render beneath the environment info:\n{top}"
    );
    // The pane is clearly marked as container information …
    assert!(
        top.contains("Container environment"),
        "the pane must carry a clear container-info header:\n{top}"
    );
    // … and lists the environment's members.
    assert!(top.contains("members:"), "member roster renders:\n{top}");
    for member in ["impl", "rev"] {
        assert!(
            top.contains(member),
            "member {member} must be listed in the container info:\n{top}"
        );
    }
    // Returning to a member agent restores a conversation pane. Deterministic:
    // re-focus the panel (Enter handed focus back to the main pane), pin that
    // Down actually moved the highlight onto the member row, then
    // unconditionally assert the container-info pane is dismissed.
    h.press(Key::Tab);
    h.press(Key::Down);
    h.press(Key::Enter);
    assert_eq!(
        h.app_mut().panel_highlight_index(),
        target + 1,
        "Down must move the highlight from the environment row to its first member"
    );
    let top = h.main_pane();
    assert!(
        !top.contains("Container environment"),
        "selecting an agent dismisses the container-info pane:\n{top}"
    );
}

#[tokio::test]
async fn environment_body_lists_per_member_socket_modes() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    let mut direct = env_agent_json("impl", "C2");
    direct["environment"]["socketMode"] = serde_json::json!("direct");
    let proxy = env_agent_json("rev", "C2");
    h.event_line(&state_changed_line(vec![direct, proxy]));

    let rows = panel_rows(&h.left_panel());
    let target = rows
        .iter()
        .position(|l| l.contains("C2") && !l.contains("impl") && !l.contains("rev"))
        .expect("selectable environment row");
    h.press(Key::Tab);
    for _ in 0..target {
        h.press(Key::Down);
    }
    h.press(Key::Enter);

    let top = h.main_pane();
    for needle in ["socket: direct", "socket: proxy"] {
        assert!(
            top.contains(needle),
            "per-member socket modes must render in the member roster:\n{top}"
        );
    }
}

#[tokio::test]
async fn environment_body_stays_head_anchored_on_short_terminals() {
    // PR #1401 review: overflow used to tail-slice the environment body like a
    // conversation, dropping the "Container environment" header and the
    // container-info disclaimer first and leaving an unlabeled member roster.
    // The body is head-anchored instead: header + disclaimer always survive
    // and the roster tail is elided with a marker.
    let mut h = TuiHarness::sized(120, 16).await;
    h.event(Event::AgentStart);
    let agents: Vec<serde_json::Value> = (0..12)
        .map(|i| env_agent_json(&format!("member{i:02}"), "C1"))
        .collect();
    h.event_line(&state_changed_line(agents));

    let rows = panel_rows(&h.left_panel());
    let target = rows
        .iter()
        .position(|l| l.contains("C1") && !l.contains("member"))
        .unwrap_or_else(|| panic!("no selectable environment row for C1:\n{}", rows.join("\n")));
    h.press(Key::Tab);
    for _ in 0..target {
        h.press(Key::Down);
    }
    h.press(Key::Enter);

    let top = h.main_pane();
    assert!(
        top.contains("Container environment"),
        "the container-info header must survive overflow truncation:\n{top}"
    );
    assert!(
        top.contains("container info only"),
        "the container-info disclaimer must survive overflow truncation:\n{top}"
    );
    assert!(
        top.contains("more container-info lines"),
        "overflow must elide the roster tail with a marker, not the header:\n{top}"
    );
}
