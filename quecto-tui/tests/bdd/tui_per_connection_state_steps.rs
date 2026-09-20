//! Step definitions for `tui_per_connection_state.feature` (#2044).
//!
//! A single connection owns pending request state. Correlation ids need only be
//! unique, and an incoming response resolves a pending request only when its id
//! is an exact match.

use super::*;
use quecto_tui::protocol::client::Event;
use quecto_tui::shell::app::tui_harness::TuiHarness;

fn harness(world: &mut TuiWorld) -> &mut TuiHarness {
    &mut world.tui_parity.as_mut().expect("TUI harness").0
}

#[given("a resume response arrives on the TUI connection")]
#[when("a resume response arrives on the TUI connection")]
fn resume_response_arrives_on_tui_connection(world: &mut TuiWorld) {
    let minted = {
        let h = harness(world);
        h.event(Event::Response {
            id: Some("resume".into()),
            command: "resume_session".into(),
            success: true,
            data: Some(serde_json::json!({ "session": "s-2044" })),
            error: None,
        });
        h.app_mut()
            .test_pending_resume_messages_id()
            .expect("resume_session response mints a pending solicited id")
            .to_string()
    };
    world.tui_minted_correlation_id = Some(minted);
}

#[then(
    "the solicited transcript fetch should have a correlation id distinct from the resume response"
)]
fn minted_fetch_has_distinct_id(world: &mut TuiWorld) {
    let id = world
        .tui_minted_correlation_id
        .as_deref()
        .expect("a correlation id was minted");
    assert_ne!(
        id, "resume",
        "the follow-up request must not reuse the response correlation id (#2044)"
    );
    assert!(!id.is_empty(), "a pending request must have a non-empty id");
}

#[when("a transcript response arrives bearing a non-matching id")]
fn transcript_response_bears_non_matching_id(world: &mut TuiWorld) {
    let minted = world
        .tui_minted_correlation_id
        .as_deref()
        .expect("a correlation id was minted");
    let unrelated = format!("unrelated-{minted}");
    assert_ne!(unrelated, minted, "the fixture id must be unrelated");
    let h = harness(world);
    h.event(Event::Response {
        id: Some(unrelated),
        command: "get_messages".into(),
        success: true,
        data: Some(serde_json::json!({
            "messages": [
                {"role": "user", "content": "an unrelated transcript"},
            ]
        })),
        error: None,
    });
}

#[then("the pending transcript request id should remain unchanged")]
fn pending_transcript_request_id_remains_unchanged(world: &mut TuiWorld) {
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
        "an unrelated response must not resolve the pending transcript fetch (#2044)"
    );
}
