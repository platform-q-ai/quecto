//! Review round 2 of the picker's search in the shell (#2010): an Enter is
//! owed only to a typed SEARCH, never to a listing (R2-T1), and never past
//! the first timeout (R2-T3); and an answer for another scope or a lost
//! connection says what happened (R2-T8).
use super::super::super::super::app_paged_history_tests::harness;
use super::super::super::super::tui_harness::TuiHarness;
use super::super::super::super::*;
use super::tests::{answer, open_picker, sent, type_text};
use crate::sessions::session_search::ANSWER_TIMEOUT;
use serde_json::json;

fn key(h: &mut TuiHarness, key: Key) {
    h.app_mut().handle_resume_selector_key(&key);
}

async fn searches(h: &mut TuiHarness) -> Vec<serde_json::Value> {
    sent(&h.drain_commands().await, "search_session_metadata")
}

fn listing(rows: serde_json::Value) -> serde_json::Value {
    json!({ "sessions": rows })
}

fn answer_listing(h: &mut TuiHarness, rows: serde_json::Value) {
    let id = h.app_mut().ac().sessions.pending_list_id.clone().unwrap();
    h.app_mut().handle_response(
        Some(id),
        "list_sessions".into(),
        true,
        Some(listing(rows)),
        None,
    );
}

/// The reviewer's live case: `/resume ⏎ ⏎` before the first listing arrives.
#[tokio::test]
async fn an_enter_typed_before_the_listing_arrives_resumes_nothing() {
    let mut h = harness().await;
    h.app_mut().ac_mut().agent_connected = true;
    h.app_mut().send_list_sessions();
    key(&mut h, Key::Enter);
    key(&mut h, Key::Enter);
    answer_listing(
        &mut h,
        json!([{"key": "cli:newest", "title": "NEWEST", "homeVersion": "h1-00000000000000aa",
                "resumeEligible": true}]),
    );
    let commands = h.drain_commands().await;
    assert!(sent(&commands, "resume_session").is_empty(), "{commands:?}");
    assert!(h.app_mut().ac().sessions.resume_selector.is_some());
    assert!(h.full_frame().contains("NEWEST"));
    // Nor is an Enter owed to the listing an emptied box asks for.
    key(&mut h, Key::Tab);
    key(&mut h, Key::Tab);
    type_text(&mut h, "n");
    let request = searches(&mut h).await[0].clone();
    answer(&mut h, &request, "FOUND", json!({}));
    key(&mut h, Key::Backspace);
    key(&mut h, Key::Enter);
    key(&mut h, Key::Enter);
    answer_listing(
        &mut h,
        json!([{"key": "cli:newest", "title": "NEWEST", "homeVersion": "h1-00000000000000aa"}]),
    );
    let commands = h.drain_commands().await;
    assert!(sent(&commands, "resume_session").is_empty(), "{commands:?}");
}

/// An owed Enter is on screen, and a retried flight never pays it.
#[tokio::test]
async fn an_owed_enter_is_shown_and_withdrawn_at_the_first_timeout() {
    let mut h = harness().await;
    open_picker(&mut h).await;
    type_text(&mut h, "zebra");
    let _ = searches(&mut h).await;
    assert!(!h.full_frame().contains("will open"), "nothing owed yet");
    key(&mut h, Key::Enter);
    key(&mut h, Key::Enter);
    let frame = h.full_frame();
    assert!(
        frame.contains("Searching… ⏎ will open the top match"),
        "{frame}"
    );
    let start = tokio::time::Instant::now();
    assert!(
        h.app_mut()
            .service_search_timeout(start + ANSWER_TIMEOUT * 2)
    );
    let retry = searches(&mut h).await[0].clone();
    let frame = h.full_frame();
    assert!(
        frame.contains("Sessions · Searching…") && !frame.contains("will open"),
        "{frame}"
    );
    answer(&mut h, &retry, "ZEBRA", json!({}));
    let commands = h.drain_commands().await;
    assert!(sent(&commands, "resume_session").is_empty(), "{commands:?}");
    assert!(h.full_frame().contains("ZEBRA"), "the rows are shown");
}

/// A focus change (Tab), a click outside the picker or a disconnect withdraws
/// the owed Enter.
#[tokio::test]
async fn a_focus_change_and_a_disconnect_each_withdraw_the_owed_enter() {
    let mut h = harness().await;
    open_picker(&mut h).await;
    type_text(&mut h, "zebra");
    key(&mut h, Key::Enter);
    key(&mut h, Key::Enter);
    key(&mut h, Key::Tab);
    let commands = h.drain_commands().await;
    let first = sent(&commands, "search_session_metadata")[0].clone();
    answer(&mut h, &first, "PROGRESS", json!({}));
    let latest = searches(&mut h).await[0].clone();
    answer(&mut h, &latest, "ZEBRA", json!({}));
    assert!(sent(&h.drain_commands().await, "resume_session").is_empty());

    // A click outside the picker (the agents pane, left of the body).
    let mut h = harness().await;
    open_picker(&mut h).await;
    type_text(&mut h, "z");
    key(&mut h, Key::Enter);
    key(&mut h, Key::Enter);
    assert!(h.app_mut().frame_split().0 > 0, "the agents pane is shown");
    key(&mut h, Key::MousePress(0, 0));
    let request = searches(&mut h).await[0].clone();
    answer(&mut h, &request, "Z", json!({}));
    assert!(sent(&h.drain_commands().await, "resume_session").is_empty());

    let mut h = harness().await;
    open_picker(&mut h).await;
    type_text(&mut h, "z");
    key(&mut h, Key::Enter);
    key(&mut h, Key::Enter);
    let dead = searches(&mut h).await[0].clone();
    h.app_mut().mark_agent_disconnected_for_test();
    h.app_mut().ac_mut().agent_connected = true;
    answer(&mut h, &dead, "DEAD", json!({}));
    assert!(sent(&h.drain_commands().await, "resume_session").is_empty());
}

/// R2-T8: an answer echoing another scope is not an "unknown error", and a
/// listing lost with the connection was never a search.
#[tokio::test]
async fn a_foreign_scope_echo_and_a_lost_listing_say_what_happened() {
    let mut h = harness().await;
    open_picker(&mut h).await;
    type_text(&mut h, "z");
    let mut request = searches(&mut h).await[0].clone();
    request["scope"] = json!("global");
    answer(&mut h, &request, "GLOBAL", json!({}));
    let notes = h.notification_messages().join("\n");
    assert!(!notes.contains("unknown error"), "{notes}");
    let frame = h.full_frame();
    assert!(
        !frame.contains("GLOBAL") && frame.contains("Sessions · No answer"),
        "{frame}"
    );
    key(&mut h, Key::Enter);
    key(&mut h, Key::Enter);
    assert!(sent(&h.drain_commands().await, "resume_session").is_empty());

    let mut h = harness().await;
    h.app_mut().ac_mut().agent_connected = true;
    h.app_mut().send_list_sessions();
    h.app_mut().mark_agent_disconnected_for_test();
    let frame = h.full_frame();
    assert!(
        frame.contains("Sessions · Disconnected") && !frame.contains("did not answer"),
        "{frame}"
    );
}

/// R2-T3: an Enter is never paid more than one answer window after it was
/// pressed — not even when every flight is answered just in time (the text
/// typed ahead of the first answer is a second flight with its own window).
#[tokio::test(start_paused = true)]
async fn an_owed_enter_expires_one_answer_window_after_it_was_pressed() {
    let mut h = harness().await;
    open_picker(&mut h).await;
    type_text(&mut h, "z");
    let first = searches(&mut h).await[0].clone();
    type_text(&mut h, "e");
    key(&mut h, Key::Enter);
    key(&mut h, Key::Enter);
    assert!(h.full_frame().contains("will open the top match"));
    let pressed = tokio::time::Instant::now();
    // The first flight is answered late but in time: progress, and the text
    // typed ahead goes out with a window of its own.
    tokio::time::advance(ANSWER_TIMEOUT - std::time::Duration::from_secs(1)).await;
    answer(&mut h, &first, "ZULU", json!({}));
    let second = searches(&mut h).await[0].clone();
    assert!(
        h.full_frame().contains("will open the top match"),
        "still in its window"
    );
    let wake = h.app_mut().next_idle_service_deadline();
    assert_eq!(
        wake,
        Some(pressed + ANSWER_TIMEOUT),
        "the loop wakes for it"
    );
    tokio::time::advance(std::time::Duration::from_secs(1)).await;
    let now = tokio::time::Instant::now();
    assert!(
        h.app_mut().service_search_timeout(now),
        "the cue is repainted away"
    );
    let frame = h.full_frame();
    assert!(
        frame.contains("Sessions · Searching…") && !frame.contains("will open"),
        "{frame}"
    );
    assert!(
        searches(&mut h).await.is_empty(),
        "the flight itself is not overdue"
    );
    answer(&mut h, &second, "ZEBRA", json!({}));
    let commands = h.drain_commands().await;
    assert!(sent(&commands, "resume_session").is_empty(), "{commands:?}");
    assert!(h.full_frame().contains("ZEBRA"));
    // Inside the window the same sequence is paid: the type-ahead still works.
    key(&mut h, Key::Tab);
    key(&mut h, Key::Tab);
    type_text(&mut h, "b");
    let third = searches(&mut h).await[0].clone();
    key(&mut h, Key::Tab);
    key(&mut h, Key::Enter);
    tokio::time::advance(ANSWER_TIMEOUT - std::time::Duration::from_secs(1)).await;
    answer(&mut h, &third, "ZEBRA", json!({}));
    let commands = h.drain_commands().await;
    assert_eq!(sent(&commands, "resume_session").len(), 1, "{commands:?}");
}
