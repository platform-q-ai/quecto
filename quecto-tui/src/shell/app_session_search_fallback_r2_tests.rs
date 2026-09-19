//! Review round 2 of the old-harness fallback (#2010): only a `parse_error`
//! that rejects THE search command, with a search of ours in flight, means an
//! old harness (R2-T2); and the fallback filters the listing of the scope on
//! screen, never the previous scope's rows (R2-T4).
use super::super::super::super::app_paged_history_tests::harness;
use super::super::super::super::tui_harness::TuiHarness;
use super::super::super::super::*;
use super::tests::{answer, open_picker, sent, type_text};
use serde_json::json;

fn key(h: &mut TuiHarness, key: Key) {
    h.app_mut().handle_resume_selector_key(&key);
}

async fn searches(h: &mut TuiHarness) -> Vec<serde_json::Value> {
    sent(&h.drain_commands().await, "search_session_metadata")
}

fn answer_listing(h: &mut TuiHarness, rows: serde_json::Value) {
    let id = h.app_mut().ac().sessions.pending_list_id.clone().unwrap();
    let listed = json!({ "sessions": rows });
    h.app_mut()
        .handle_response(Some(id), "list_sessions".into(), true, Some(listed), None);
}

/// The new harness's own wording for ANY unknown command names every known
/// one — `search_session_metadata` among them — and is broadcast.
#[tokio::test]
async fn a_parse_error_about_another_command_is_not_an_old_harness() {
    let mut h = harness().await;
    open_picker(&mut h).await;
    type_text(&mut h, "z");
    let request = searches(&mut h).await[0].clone();
    let error = "parse error: unknown variant `frobnicate`, expected one of `prompt`, `steer`, \
                 `list_sessions`, `search_session_metadata`, `new_session` at line 1 column 20";
    h.app_mut()
        .handle_response(None, "parse_error".into(), false, None, Some(error.into()));
    assert!(!h.app_mut().ac().sessions.search_unsupported);
    assert!(h.app_mut().ac().sessions.search.is_in_flight());
    let notes = h.notification_messages().join("\n");
    assert!(!notes.contains("newer quecto harness"), "{notes}");
    answer(&mut h, &request, "ANSWERED", json!({}));
    assert!(h.full_frame().contains("ANSWERED"));
    // Master's wording for the search command itself, with no search of ours
    // in flight, is a peer's.
    let error = "parse error: unknown variant `search_session_metadata`, expected one of \
                 `prompt`, `steer` at line 1 column 33";
    h.app_mut()
        .handle_response(None, "parse_error".into(), false, None, Some(error.into()));
    assert!(!h.app_mut().ac().sessions.search_unsupported);
    // With one in flight it is ours — unless it carries another request's id.
    type_text(&mut h, "e");
    assert_eq!(searches(&mut h).await.len(), 1);
    let peer = Some("other-tab:resume-search-9".to_string());
    h.app_mut()
        .handle_response(peer, "parse_error".into(), false, None, Some(error.into()));
    assert!(!h.app_mut().ac().sessions.search_unsupported);
    h.app_mut()
        .handle_response(None, "parse_error".into(), false, None, Some(error.into()));
    assert!(h.app_mut().ac().sessions.search_unsupported);
}

/// Fallback mode: text typed while the new scope's listing is awaited is
/// applied to THAT listing when it arrives, never to the old scope's rows.
#[tokio::test]
async fn the_fallback_filters_the_listing_of_the_scope_on_screen() {
    let mut h = harness().await;
    open_picker(&mut h).await;
    type_text(&mut h, "l");
    assert_eq!(searches(&mut h).await.len(), 1);
    let error = "parse error: unknown variant `search_session_metadata`, expected one of `prompt`";
    h.app_mut()
        .handle_response(None, "parse_error".into(), false, None, Some(error.into()));
    key(&mut h, Key::BackTab);
    key(&mut h, Key::Right);
    let commands = h.drain_commands().await;
    assert_eq!(sent(&commands, "list_sessions")[0]["scope"], "global");
    key(&mut h, Key::Tab);
    type_text(&mut h, "i");
    let frame = h.full_frame();
    assert!(
        frame.contains("Sessions · Loading…")
            && !frame.contains("LISTED")
            && !frame.contains("Filtering the listed sessions here"),
        "the old scope's rows are never settled under the new scope: {frame}"
    );
    assert!(h.app_mut().ac().sessions.pending_list_id.is_some());
    answer_listing(
        &mut h,
        json!([{"key": "cli:g1", "title": "lion elsewhere", "executionPath": "/other"},
               {"key": "cli:g2", "title": "alpha", "executionPath": "/other"}]),
    );
    let frame = h.full_frame();
    assert!(
        frame.contains("lion elsewhere") && !frame.contains("alpha"),
        "{frame}"
    );
    assert!(!frame.contains("Loading…"), "{frame}");
}
