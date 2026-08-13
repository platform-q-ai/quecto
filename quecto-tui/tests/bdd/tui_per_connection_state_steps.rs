//! Step definitions for `tui_per_connection_state.feature` (#1463).
//!
//! Phase 2 of the multi-session TUI (epic #1467): connection-scoped state
//! moves off `App` into the per-tab connection structures (behind the
//! `active_conn()`/`active_conn_mut()` seam), and every minted correlation id
//! gains a connection namespace so a broadcast response can never match a
//! pending latch on another tab. The master tab is `TabId(0)`, so its
//! namespace prefix is `tab0:`. The prefix assertion is an explicit contract
//! pin of the id encoding; the second scenario pins the isolation the
//! encoding buys (a foreign-namespace response resolves nothing here).

use super::*;
use quecto_tui::protocol::client::Event;
use quecto_tui::shell::app::tui_harness::TuiHarness;

/// Namespace prefix minted correlation ids must carry for the master tab
/// (`TabId(0)`), pinned by #1463.
const MASTER_NAMESPACE: &str = "tab0:";

fn harness(world: &mut TuiWorld) -> &mut TuiHarness {
    &mut world.tui_parity.as_mut().expect("TUI harness").0
}

#[given("a resume response arrives on the master connection")]
#[when("a resume response arrives on the master connection")]
fn resume_response_arrives_on_master_connection(world: &mut TuiWorld) {
    let minted = {
        let h = harness(world);
        h.event(Event::Response {
            id: Some("resume".into()),
            command: "resume_session".into(),
            success: true,
            data: Some(serde_json::json!({ "session": "s-1463" })),
            error: None,
        });
        h.app_mut()
            .test_pending_resume_messages_id()
            .expect("resume_session response mints a pending solicited id")
            .to_string()
    };
    world.tui_minted_correlation_id = Some(minted);
}

#[then("the solicited transcript fetch it mints should carry the master connection's namespace")]
fn minted_fetch_carries_master_namespace(world: &mut TuiWorld) {
    let id = world
        .tui_minted_correlation_id
        .as_deref()
        .expect("a correlation id was minted");
    assert!(
        id.starts_with(MASTER_NAMESPACE),
        "minted correlation ids must be namespaced to their connection \
         (#1463): expected prefix {MASTER_NAMESPACE:?}, got {id:?}"
    );
}

#[when("a transcript response arrives bearing another connection's id")]
fn transcript_response_bears_foreign_connection_id(world: &mut TuiWorld) {
    let minted = world
        .tui_minted_correlation_id
        .clone()
        .expect("a correlation id was minted");
    // The same fetch, re-keyed to a different tab's namespace: strip this
    // tab's prefix if present so the step also fails meaningfully pre-#1463.
    let foreign = format!(
        "tab1:{}",
        minted.strip_prefix(MASTER_NAMESPACE).unwrap_or(&minted)
    );
    let h = harness(world);
    h.event(Event::Response {
        id: Some(foreign),
        command: "get_messages".into(),
        success: true,
        data: Some(serde_json::json!({
            "messages": [
                {"role": "user", "content": "another tab's transcript"},
            ]
        })),
        error: None,
    });
}

#[then("this tab's pending transcript fetch should remain unresolved")]
fn pending_transcript_fetch_remains_unresolved(world: &mut TuiWorld) {
    let expected = world
        .tui_minted_correlation_id
        .clone()
        .expect("a correlation id was minted");
    let h = harness(world);
    let pending = h
        .app_mut()
        .test_pending_resume_messages_id()
        .map(String::from);
    assert_eq!(
        pending.as_deref(),
        Some(expected.as_str()),
        "a response bearing another connection's id must not resolve this \
         tab's pending transcript fetch (#1463)"
    );
    let frame = h.full_frame();
    assert!(
        !frame.contains("another tab's transcript"),
        "a foreign-namespace transcript must not land in this tab's chat \
         (#1463):\n{frame}"
    );
}
