//! Review round 1 of the picker's search in the shell (#2010): Enter acts
//! only on the settled answer (R1-T1), what a search is doing is on screen
//! (R1-T2, R1-T3), a lost answer or a lost connection never wedges the box
//! (R1-T4), an old harness costs one warning and still narrows (R1-T5), and
//! every way the picker closes abandons the flight (R1-T7, R1-T13).
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

fn no_rows(h: &mut TuiHarness, request: &serde_json::Value, extra: serde_json::Value) {
    let mut fields = json!({"sessions": [], "totalMatches": 0});
    for (field, value) in extra.as_object().into_iter().flatten() {
        fields[field] = value.clone();
    }
    answer(h, request, "unused", fields);
}

#[tokio::test]
async fn enter_typed_ahead_of_the_answer_resumes_the_answered_row_never_a_listed_one() {
    let mut h = harness().await;
    open_picker(&mut h).await;
    type_text(&mut h, "zebra");
    key(&mut h, Key::Enter);
    key(&mut h, Key::Enter);
    let commands = h.drain_commands().await;
    assert!(sent(&commands, "resume_session").is_empty(), "{commands:?}");
    assert!(h.app_mut().ac().sessions.resume_selector.is_some());
    let first = sent(&commands, "search_session_metadata")[0].clone();
    // The overtaken answer (for `z`) is shown as progress and resumes nothing
    // — and its top row is not the one the owed Enter will open.
    let progress = json!([
        {"key": "cli:progress", "title": "PROGRESS", "homeVersion": "h1-00000000000000cc"},
        {"key": "cli:found", "title": "ZEBRA PLAN", "homeVersion": "h1-00000000000000bb"}]);
    answer(&mut h, &first, "unused", json!({ "sessions": progress }));
    assert!(h.full_frame().contains("→ PROGRESS"), "{}", h.full_frame());
    let commands = h.drain_commands().await;
    assert!(sent(&commands, "resume_session").is_empty(), "{commands:?}");
    let latest = sent(&commands, "search_session_metadata")[0].clone();
    assert_eq!(latest["query"], "zebra");
    answer(&mut h, &latest, "ZEBRA PLAN", json!({}));
    let resumed = sent(&h.drain_commands().await, "resume_session");
    assert_eq!(resumed.len(), 1, "{resumed:?}");
    assert_eq!(
        (&resumed[0]["session"], &resumed[0]["expectedHomeVersion"]),
        (&json!("cli:found"), &json!("h1-00000000000000bb"))
    );
    assert!(h.app_mut().ac().sessions.resume_selector.is_none());
}

#[tokio::test]
async fn a_deferred_enter_resumes_nothing_when_the_answer_has_no_rows() {
    let mut h = harness().await;
    open_picker(&mut h).await;
    type_text(&mut h, "q");
    key(&mut h, Key::Enter);
    key(&mut h, Key::Enter);
    let request = searches(&mut h).await[0].clone();
    no_rows(&mut h, &request, json!({}));
    assert!(sent(&h.drain_commands().await, "resume_session").is_empty());
    let frame = h.full_frame();
    // Wrapped over at most two rows, never cut.
    let said = frame.replace('│', " ");
    let said = said.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        said.contains(
            "No sessions match \"q\" in Local Folder — Tab to Scope to search All Folders"
        ),
        "{frame}"
    );
    assert!(
        !frame.contains("No items") && !frame.contains("Searching…"),
        "{frame}"
    );
    // Settled rows are acted on at once again.
    type_text_in_search(&mut h, "r").await;
}

/// Back to the box, one more character, answered fresh, Enter resumes it.
async fn type_text_in_search(h: &mut TuiHarness, text: &str) {
    key(h, Key::BackTab);
    type_text(h, text);
    let request = searches(h).await[0].clone();
    answer(h, &request, "SETTLED", json!({}));
    key(h, Key::Enter);
    key(h, Key::Enter);
    assert_eq!(sent(&h.drain_commands().await, "resume_session").len(), 1);
}

#[tokio::test]
async fn searching_progress_truncation_and_refusal_are_on_screen() {
    let mut h = harness().await;
    open_picker(&mut h).await;
    type_text(&mut h, "me");
    let first = searches(&mut h).await[0].clone();
    let frame = h.full_frame();
    assert!(
        frame.contains("Sessions · Searching…") && frame.contains("LISTED"),
        "{frame}"
    );
    // Overtaken but newer than the rows on screen: shown, still searching.
    answer(&mut h, &first, "PROGRESS", json!({}));
    let frame = h.full_frame();
    assert!(
        frame.contains("PROGRESS") && !frame.contains("LISTED"),
        "{frame}"
    );
    assert!(frame.contains("Sessions · Searching…"), "{frame}");
    let latest = searches(&mut h).await[0].clone();
    answer(
        &mut h,
        &latest,
        "FRESH",
        json!({"totalMatches": 5200, "truncated": true}),
    );
    let frame = h.full_frame();
    assert!(
        frame.contains("Showing 1 of 5,200 — keep typing to narrow"),
        "{frame}"
    );
    assert!(!frame.contains("Searching…"), "{frame}");
    // All Folders words its own empty answer; a refusal is said in the picker.
    key(&mut h, Key::BackTab);
    key(&mut h, Key::Char(' '));
    let global = searches(&mut h).await[0].clone();
    assert_eq!(global["scope"], "global");
    no_rows(&mut h, &global, json!({}));
    let frame = h.full_frame();
    assert!(
        frame.contains("No sessions match \"me\" in All Folders"),
        "{frame}"
    );
    assert!(!frame.contains("Tab to Scope"), "{frame}");
    key(&mut h, Key::Tab);
    type_text(&mut h, "x");
    let request = searches(&mut h).await[0].clone();
    no_rows(
        &mut h,
        &request,
        json!({"refused": "query too long: 300 characters"}),
    );
    assert!(
        h.full_frame()
            .contains("Search refused: query too long: 300 characters")
    );
}

#[tokio::test]
async fn an_overtaken_answer_of_another_scope_is_never_shown() {
    let mut h = harness().await;
    open_picker(&mut h).await;
    type_text(&mut h, "w");
    let local = searches(&mut h).await[0].clone();
    key(&mut h, Key::BackTab);
    key(&mut h, Key::Char(' '));
    answer(&mut h, &local, "LOCAL-ROW", json!({}));
    assert!(!h.full_frame().contains("LOCAL-ROW"));
    let global = searches(&mut h).await[0].clone();
    assert_eq!(global["scope"], "global");
    // The right generation echoing the wrong scope is not this scope's answer.
    answer(&mut h, &global, "MISLABELLED", json!({"scope": "local"}));
    assert!(!h.full_frame().contains("MISLABELLED"));
}

#[tokio::test]
async fn a_lost_answer_is_retried_once_then_given_up_and_every_edit_recovers() {
    let mut h = harness().await;
    open_picker(&mut h).await;
    type_text(&mut h, "z");
    let lost = searches(&mut h).await[0].clone();
    type_text(&mut h, "eb");
    let start = tokio::time::Instant::now();
    assert!(
        !h.app_mut()
            .service_search_timeout(start + ANSWER_TIMEOUT / 2)
    );
    assert!(
        h.app_mut()
            .service_search_timeout(start + ANSWER_TIMEOUT + ANSWER_TIMEOUT / 2)
    );
    let retry = searches(&mut h).await;
    assert_eq!((retry.len(), &retry[0]["query"]), (1, &json!("zeb")));
    // The lost answer turning up late is nobody's.
    answer(&mut h, &lost, "LATE", json!({}));
    assert!(!h.full_frame().contains("LATE"));
    assert!(
        h.app_mut()
            .service_search_timeout(start + ANSWER_TIMEOUT * 3)
    );
    assert!(searches(&mut h).await.is_empty(), "one retry only");
    let notes = h.notification_messages().join("\n");
    assert_eq!(notes.matches("Search did not answer").count(), 1, "{notes}");
    let frame = h.full_frame();
    assert!(
        frame.contains("Sessions · No answer") && frame.contains("Search did not answer — edit"),
        "{frame}"
    );
    // Stalled rows are not acted on.
    key(&mut h, Key::Enter);
    key(&mut h, Key::Enter);
    assert!(sent(&h.drain_commands().await, "resume_session").is_empty());
    // Typing, toggling the scope and emptying the box each ask again.
    key(&mut h, Key::BackTab);
    type_text(&mut h, "r");
    let again = searches(&mut h).await;
    assert_eq!((again.len(), &again[0]["query"]), (1, &json!("zebr")));
    assert!(
        h.app_mut()
            .service_search_timeout(start + ANSWER_TIMEOUT * 9)
    );
    assert!(
        h.app_mut()
            .service_search_timeout(start + ANSWER_TIMEOUT * 19)
    );
    let _ = h.drain_commands().await;
    key(&mut h, Key::BackTab);
    key(&mut h, Key::Char(' '));
    assert_eq!(searches(&mut h).await.len(), 1, "a scope toggle recovers");
    assert!(
        h.app_mut()
            .service_search_timeout(start + ANSWER_TIMEOUT * 29)
    );
    assert!(
        h.app_mut()
            .service_search_timeout(start + ANSWER_TIMEOUT * 39)
    );
    let _ = h.drain_commands().await;
    key(&mut h, Key::Tab);
    for _ in 0..4 {
        key(&mut h, Key::Backspace);
    }
    let commands = h.drain_commands().await;
    assert_eq!(
        sent(&commands, "list_sessions").len(),
        1,
        "an emptied box lists"
    );
}

#[tokio::test]
async fn a_lost_connection_frees_the_flight_and_the_next_edit_searches_again() {
    let mut h = harness().await;
    open_picker(&mut h).await;
    type_text(&mut h, "z");
    let dead = searches(&mut h).await[0].clone();
    h.app_mut().mark_agent_disconnected_for_test();
    assert!(!h.app_mut().ac().sessions.search.is_in_flight());
    assert!(h.full_frame().contains("Sessions · Disconnected"));
    h.app_mut().ac_mut().agent_connected = true;
    answer(&mut h, &dead, "DEAD", json!({}));
    assert!(!h.full_frame().contains("DEAD"));
    type_text(&mut h, "e");
    let next = searches(&mut h).await;
    assert_eq!((next.len(), &next[0]["query"]), (1, &json!("ze")));
    // A listing that died with the connection does not leave "Loading…" up.
    answer(&mut h, &next[0], "OK", json!({}));
    key(&mut h, Key::Backspace);
    key(&mut h, Key::Backspace);
    assert!(h.full_frame().contains("Sessions · Loading…"));
    h.app_mut().mark_agent_disconnected_for_test();
    let frame = h.full_frame();
    assert!(
        !frame.contains("Loading…") && frame.contains("Sessions · Disconnected"),
        "{frame}"
    );
}

/// The awaited listing answered with three rows.
fn list_three(h: &mut TuiHarness) {
    let id = h.app_mut().ac().sessions.pending_list_id.clone().unwrap();
    let listed = json!({"sessions": [
        {"key": "cli:a", "title": "Fix the RENDERER beta", "executionPath": "/work/app", "updatedUnixSecs": 30},
        {"key": "cli:b", "title": "write docs", "executionPath": "/work/app/docs-site", "updatedUnixSecs": 20},
        {"key": "cli:c", "title": "Straße migration", "executionPath": "/work/app", "updatedUnixSecs": 10},
    ]});
    h.app_mut()
        .handle_response(Some(id), "list_sessions".into(), true, Some(listed), None);
}

/// Three listed rows, the search box focused.
async fn open_three(h: &mut TuiHarness) {
    h.app_mut().ac_mut().agent_connected = true;
    h.app_mut().send_list_sessions();
    list_three(h);
    key(h, Key::Tab);
    key(h, Key::Tab);
    let _ = h.drain_commands().await;
}

#[tokio::test]
async fn an_old_harness_costs_one_warning_and_the_box_still_narrows_the_listed_rows() {
    let mut h = harness().await;
    open_three(&mut h).await;
    type_text(&mut h, "b");
    assert_eq!(searches(&mut h).await.len(), 1);
    let error = "unknown variant `search_session_metadata`, expected one of `prompt`";
    h.app_mut()
        .handle_response(None, "parse_error".into(), false, None, Some(error.into()));
    type_text(&mut h, "eta");
    assert!(
        searches(&mut h).await.is_empty(),
        "asked once per connection"
    );
    let notes = h.notification_messages().join("\n");
    assert_eq!(
        notes
            .matches("Search needs a newer quecto harness — restart the agent")
            .count(),
        1,
        "{notes}"
    );
    let frame = h.full_frame();
    assert!(
        frame.contains("RENDERER beta") && !frame.contains("write docs"),
        "{frame}"
    );
    assert!(
        !frame.contains("Searching…"),
        "filtered here: settled at once: {frame}"
    );
    // The same visible-text rule: every word, any order, title / path / whole key.
    for (query, shown, hidden) in [
        ("BETA  fix", "RENDERER beta", "write docs"),
        ("docs-site", "write docs", "RENDERER"),
        ("strasse", "Straße migration", "write docs"),
        ("cli:b", "write docs", "RENDERER"),
        ("cli:", "No sessions match", "write docs"),
    ] {
        for _ in 0..40 {
            key(&mut h, Key::Backspace);
        }
        // The emptied box lists its scope again; the text filters that listing.
        let _ = h.drain_commands().await;
        list_three(&mut h);
        type_text(&mut h, query);
        let frame = h.full_frame();
        assert!(
            frame.contains(shown) && !frame.contains(hidden),
            "{query}: {frame}"
        );
    }
    assert!(searches(&mut h).await.is_empty());
    // A filtered row is settled: Enter resumes it, with its listed version.
    key(&mut h, Key::Enter);
    // A new connection is asked again.
    h.app_mut().mark_agent_disconnected_for_test();
    h.app_mut().ac_mut().agent_connected = true;
    key(&mut h, Key::BackTab);
    type_text(&mut h, "x");
    assert_eq!(searches(&mut h).await.len(), 1);
}

/// R1-T7, the reviewer's probe: a picker closed by a tab switch.
#[tokio::test]
async fn a_picker_closed_by_a_tab_switch_abandons_the_flight_and_a_late_answer_changes_nothing() {
    let mut h = harness().await;
    open_picker(&mut h).await;
    type_text(&mut h, "z");
    let request = searches(&mut h).await[0].clone();
    h.app_mut().close_session_switch_overlays();
    assert!(!h.app_mut().ac().sessions.search.is_in_flight());
    assert!(h.app_mut().ac().sessions.pending_list_id.is_none());
    answer(
        &mut h,
        &request,
        "LATE",
        json!({"refused": "late refusal", "diagnostics": ["cli_x.json: late"]}),
    );
    let sessions = &h.app_mut().ac().sessions;
    assert!(!sessions.home_versions.contains_key("cli:found"));
    assert!(!sessions.listed_titles.contains_key("cli:found"));
    assert!(
        h.notification_messages().is_empty(),
        "{:?}",
        h.notification_messages()
    );
    let later = tokio::time::Instant::now() + ANSWER_TIMEOUT * 2;
    assert!(
        !h.app_mut().service_search_timeout(later),
        "nothing to give up"
    );
}

/// R1-T13: the four guards the round-1 mutants survived.
#[tokio::test]
async fn reopening_a_failed_listing_and_an_answer_each_leave_no_stale_flight_or_version() {
    let mut h = harness().await;
    open_picker(&mut h).await;
    type_text(&mut h, "z");
    let _ = h.drain_commands().await;
    // Reopened with a search still in the air: the next edit goes straight out.
    h.app_mut().send_list_sessions();
    assert!(
        !h.app_mut().ac().sessions.search.is_in_flight(),
        "abandoned on open"
    );
    key(&mut h, Key::Escape);
    // A failed listing closes the picker and leaves nothing in flight.
    open_picker(&mut h).await;
    type_text(&mut h, "zz");
    assert!(h.app_mut().ac().sessions.search.is_in_flight());
    key(&mut h, Key::Backspace);
    key(&mut h, Key::Backspace);
    let id = h.app_mut().ac().sessions.pending_list_id.clone();
    assert!(id.is_some() && h.app_mut().ac().sessions.search.is_in_flight());
    h.app_mut()
        .handle_response(id, "list_sessions".into(), false, None, Some("boom".into()));
    assert!(h.app_mut().ac().sessions.resume_selector.is_none());
    assert!(
        !h.app_mut().ac().sessions.search.is_in_flight(),
        "abandoned on failure"
    );
    // A selection closes the picker through the same path: a search the
    // emptied box left in the air is abandoned with it.
    open_picker(&mut h).await;
    type_text(&mut h, "z");
    key(&mut h, Key::Backspace);
    let id = h.app_mut().ac().sessions.pending_list_id.clone();
    let listed = json!({"sessions": [{"key": "cli:listed", "title": "LISTED"}]});
    h.app_mut()
        .handle_response(id, "list_sessions".into(), true, Some(listed), None);
    assert!(
        h.app_mut().ac().sessions.search.is_in_flight(),
        "still in the air"
    );
    key(&mut h, Key::Enter);
    key(&mut h, Key::Enter);
    assert!(h.app_mut().ac().sessions.resume_selector.is_none());
    assert!(
        !h.app_mut().ac().sessions.search.is_in_flight(),
        "abandoned on selection"
    );
    let _ = h.drain_commands().await;
    // An answer replaces the rows, so a version kept from older rows goes.
    open_picker(&mut h).await;
    h.app_mut().ac_mut().sessions.selected_home_version =
        Some(("cli:found".into(), "h1-old".into()));
    type_text(&mut h, "f");
    let request = searches(&mut h).await[0].clone();
    answer(&mut h, &request, "FOUND", json!({}));
    assert_eq!(h.app_mut().ac().sessions.selected_home_version, None);
}
