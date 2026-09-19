//! Review round 3 of the old-harness fallback (#2010 R3-T2): the verdict that
//! flips a connection to the fallback never settles the box on rows it does
//! not hold for the scope on screen — it asks for that listing and waits.
use super::super::super::super::app_paged_history_tests::harness;
use super::super::super::super::tui_harness::TuiHarness;
use super::super::super::super::*;
use super::tests::{open_picker, sent, type_text};
use serde_json::json;

const OLD: &str =
    "parse error: unknown variant `search_session_metadata`, expected one of `prompt`";

fn key(h: &mut TuiHarness, key: Key) {
    h.app_mut().handle_resume_selector_key(&key);
}

fn old_harness_verdict(h: &mut TuiHarness) {
    h.app_mut()
        .handle_response(None, "parse_error".into(), false, None, Some(OLD.into()));
}

fn answer_listing(h: &mut TuiHarness, rows: serde_json::Value) {
    let id = h.app_mut().ac().sessions.pending_list_id.clone().unwrap();
    let listed = json!({ "sessions": rows });
    h.app_mut()
        .handle_response(Some(id), "list_sessions".into(), true, Some(listed), None);
}

/// `/resume`, text typed before the connection's FIRST listing arrives, then
/// the old-harness verdict: no false "No sessions match".
#[tokio::test]
async fn text_typed_before_the_first_listing_waits_for_it_on_an_old_harness() {
    let mut h = harness().await;
    h.app_mut().ac_mut().agent_connected = true;
    h.app_mut().send_list_sessions();
    key(&mut h, Key::Tab);
    key(&mut h, Key::Tab);
    type_text(&mut h, "z");
    let _ = h.drain_commands().await;
    old_harness_verdict(&mut h);
    let frame = h.full_frame();
    assert!(
        frame.contains("Sessions · Loading…") && !frame.contains("No sessions match"),
        "{frame}"
    );
    let commands = h.drain_commands().await;
    let lists = sent(&commands, "list_sessions");
    assert_eq!(lists.len(), 1, "the scope on screen is listed again");
    assert_eq!(lists[0]["scope"], "local");
    key(&mut h, Key::Enter);
    key(&mut h, Key::Enter);
    answer_listing(
        &mut h,
        json!([{"key": "cli:z", "title": "zeta here"}, {"key": "cli:a", "title": "alpha"}]),
    );
    let frame = h.full_frame();
    assert!(
        frame.contains("zeta here") && !frame.contains("alpha") && !frame.contains("Loading…"),
        "{frame}"
    );
    let commands = h.drain_commands().await;
    assert!(sent(&commands, "resume_session").is_empty(), "{commands:?}");
}

/// A reopened Local picker never settles on the previous picker's All
/// Folders rows, and Enter resumes none of them.
#[tokio::test]
async fn a_reopened_picker_never_inherits_the_previous_scopes_rows() {
    let mut h = harness().await;
    open_picker(&mut h).await;
    key(&mut h, Key::BackTab);
    key(&mut h, Key::Right);
    answer_listing(
        &mut h,
        json!([{"key": "cli:g", "title": "zglobal elsewhere", "executionPath": "/other"}]),
    );
    key(&mut h, Key::Escape);
    assert!(
        h.app_mut().ac().sessions.listed.is_empty(),
        "closed: no rows"
    );
    h.app_mut().send_list_sessions();
    key(&mut h, Key::Tab);
    key(&mut h, Key::Tab);
    type_text(&mut h, "z");
    let _ = h.drain_commands().await;
    old_harness_verdict(&mut h);
    let frame = h.full_frame();
    assert!(
        frame.contains("Sessions · Loading…") && !frame.contains("zglobal"),
        "{frame}"
    );
    key(&mut h, Key::Enter);
    key(&mut h, Key::Enter);
    let commands = h.drain_commands().await;
    assert!(sent(&commands, "resume_session").is_empty(), "{commands:?}");
    answer_listing(&mut h, json!([{"key": "cli:l", "title": "zlocal"}]));
    let frame = h.full_frame();
    assert!(
        frame.contains("zlocal") && !frame.contains("zglobal"),
        "{frame}"
    );
}

/// …nor its listing of the SAME scope: a closed picker holds no listing, so
/// the reopened one waits for its own instead of filtering nothing.
#[tokio::test]
async fn a_reopened_picker_holds_no_listing_of_its_own_scope_either() {
    let mut h = harness().await;
    open_picker(&mut h).await;
    key(&mut h, Key::Escape);
    assert!(h.app_mut().ac().sessions.listed_scope.is_none());
    h.app_mut().send_list_sessions();
    key(&mut h, Key::Tab);
    key(&mut h, Key::Tab);
    type_text(&mut h, "l");
    let _ = h.drain_commands().await;
    old_harness_verdict(&mut h);
    let frame = h.full_frame();
    assert!(
        frame.contains("Sessions · Loading…") && !frame.contains("No sessions match"),
        "{frame}"
    );
    answer_listing(&mut h, json!([{"key": "cli:l", "title": "LISTED again"}]));
    assert!(h.full_frame().contains("LISTED again"));
}

/// Rows held for the OTHER scope (their listing was overtaken by a search)
/// are not the fallback's data either.
#[tokio::test]
async fn the_verdict_relists_when_the_rows_held_are_another_scopes() {
    let mut h = harness().await;
    open_picker(&mut h).await;
    key(&mut h, Key::BackTab);
    key(&mut h, Key::Right);
    key(&mut h, Key::Tab);
    type_text(&mut h, "l");
    let _ = h.drain_commands().await;
    old_harness_verdict(&mut h);
    let frame = h.full_frame();
    assert!(
        frame.contains("Sessions · Loading…") && !frame.contains("LISTED"),
        "{frame}"
    );
    let commands = h.drain_commands().await;
    assert_eq!(sent(&commands, "list_sessions")[0]["scope"], "global");
}
