//! The picker's metadata search in the shell (#2010): what an edit sends, the
//! single flight, which answer is shown, and what failure and refusal say.
use super::super::super::app_paged_history_tests::harness;
use super::super::super::tui_harness::TuiHarness;
use super::super::super::*;
use serde_json::json;

fn sent(commands: &[String], kind: &str) -> Vec<serde_json::Value> {
    commands
        .iter()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter(|command| command["type"] == kind)
        .collect()
}

/// `/resume` answered with one listed row, the search box focused.
async fn open_picker(h: &mut TuiHarness) {
    h.app_mut().ac_mut().agent_connected = true;
    h.app_mut().send_list_sessions();
    let id = h.app_mut().ac().sessions.pending_list_id.clone().unwrap();
    let listed = json!({"sessions": [{"key": "cli:listed", "title": "LISTED", "homeVersion": "h1-00000000000000aa"}]});
    h.app_mut()
        .handle_response(Some(id), "list_sessions".into(), true, Some(listed), None);
    for key in [Key::Tab, Key::Tab] {
        h.app_mut().handle_resume_selector_key(&key);
    }
    let _ = h.drain_commands().await;
}

fn type_text(h: &mut TuiHarness, text: &str) {
    for ch in text.chars() {
        h.app_mut().handle_resume_selector_key(&Key::Char(ch));
    }
}

fn answer(h: &mut TuiHarness, request: &serde_json::Value, title: &str, extra: serde_json::Value) {
    let mut data = json!({
        "generation": request["generation"], "scope": request["scope"], "totalMatches": 1,
        "sessions": [{"key": "cli:found", "title": title, "homeVersion": "h1-00000000000000bb",
                      "homeState": "legacy_unscoped"}],
    });
    for (field, value) in extra.as_object().into_iter().flatten() {
        data[field] = value.clone();
    }
    let id = request["id"].as_str().map(str::to_string);
    h.app_mut()
        .handle_response(id, "search_session_metadata".into(), true, Some(data), None);
}

#[tokio::test]
async fn an_edit_sends_one_search_and_its_answer_replaces_rows_and_versions() {
    let mut h = harness().await;
    open_picker(&mut h).await;
    type_text(&mut h, "zeb");
    let searches = sent(&h.drain_commands().await, "search_session_metadata");
    assert_eq!(searches.len(), 1, "single flight: {searches:?}");
    assert_eq!(
        (&searches[0]["query"], &searches[0]["scope"]),
        (&json!("z"), &json!("local"))
    );
    assert!(h.app_mut().ac().sessions.search.is_in_flight());
    assert!(
        h.full_frame().contains("LISTED"),
        "rows stay until an answer replaces them"
    );
    // Overtaken: dropped, and the latest text goes out.
    answer(&mut h, &searches[0], "STALE", json!({}));
    assert!(!h.full_frame().contains("STALE"));
    let next = sent(&h.drain_commands().await, "search_session_metadata");
    assert_eq!((next.len(), &next[0]["query"]), (1, &json!("zeb")));
    assert!(next[0]["generation"].as_u64() > searches[0]["generation"].as_u64());
    answer(&mut h, &next[0], "FRESH", json!({}));
    let frame = h.full_frame();
    assert!(
        frame.contains("FRESH") && !frame.contains("LISTED"),
        "{frame}"
    );
    assert!(
        frame.contains("Unscoped · no folder on record") && frame.contains("key cli:found"),
        "{frame}"
    );
    let sessions = &h.app_mut().ac().sessions;
    assert_eq!(
        sessions.home_versions.get("cli:found").map(String::as_str),
        Some("h1-00000000000000bb")
    );
    assert!(!sessions.home_versions.contains_key("cli:listed"));
    assert!(!sessions.search.is_in_flight());
}

#[tokio::test]
async fn an_answer_is_shown_only_under_the_sent_id_with_the_latest_generation() {
    let mut h = harness().await;
    open_picker(&mut h).await;
    type_text(&mut h, "q");
    let request = sent(&h.drain_commands().await, "search_session_metadata")[0].clone();
    let mut foreign = request.clone();
    foreign["id"] = json!("other-tab:resume-search-1");
    answer(&mut h, &foreign, "FOREIGN", json!({}));
    assert!(!h.full_frame().contains("FOREIGN"));
    assert!(
        h.app_mut().ac().sessions.search.is_in_flight(),
        "a foreign answer settles nothing"
    );
    answer(
        &mut h,
        &request,
        "WRONG-GENERATION",
        json!({"generation": 0}),
    );
    assert!(!h.full_frame().contains("WRONG-GENERATION"));
    assert!(sent(&h.drain_commands().await, "search_session_metadata").is_empty());
}

#[tokio::test]
async fn a_cleared_box_lists_the_scope_and_a_listing_in_flight_yields_to_a_search() {
    let mut h = harness().await;
    open_picker(&mut h).await;
    type_text(&mut h, "w");
    let search = sent(&h.drain_commands().await, "search_session_metadata")[0].clone();
    h.app_mut().handle_resume_selector_key(&Key::Backspace);
    let commands = h.drain_commands().await;
    assert_eq!(sent(&commands, "list_sessions").len(), 1);
    assert!(sent(&commands, "search_session_metadata").is_empty());
    answer(&mut h, &search, "LATE", json!({}));
    assert!(!h.full_frame().contains("LATE"));
    // Typing again before the listing is answered: the listing is unwanted.
    let list_id = h.app_mut().ac().sessions.pending_list_id.clone();
    type_text(&mut h, "x");
    assert!(h.app_mut().ac().sessions.pending_list_id.is_none());
    let late = json!({"sessions": [{"key": "cli:l", "title": "LATE-LISTING"}]});
    h.app_mut()
        .handle_response(list_id, "list_sessions".into(), true, Some(late), None);
    assert!(!h.full_frame().contains("LATE-LISTING"));
}

#[tokio::test]
async fn a_failed_or_refused_search_says_so_and_leaves_the_picker_usable() {
    let mut h = harness().await;
    open_picker(&mut h).await;
    type_text(&mut h, "a");
    let request = sent(&h.drain_commands().await, "search_session_metadata")[0].clone();
    let id = request["id"].as_str().map(str::to_string);
    h.app_mut().handle_response(
        id,
        "search_session_metadata".into(),
        false,
        None,
        Some("sessions dir unreadable".into()),
    );
    assert!(
        h.notification_messages()
            .iter()
            .any(|n| n.contains("Could not search sessions"))
    );
    assert!(h.full_frame().contains("LISTED") && !h.app_mut().ac().sessions.search.is_in_flight());
    type_text(&mut h, "b");
    let request = sent(&h.drain_commands().await, "search_session_metadata")[0].clone();
    answer(
        &mut h,
        &request,
        "x",
        json!({"sessions": [], "refused": "query too long: 300\u{1b}[2J characters", "diagnostics": ["cli_bad.json: session record unavailable"]}),
    );
    let notes = h.notification_messages().join("\n");
    assert!(
        notes.contains("Search refused: query too long: 300") && !notes.contains('\u{1b}'),
        "{notes}"
    );
    assert!(notes.contains("cli_bad.json"), "{notes}");
    assert!(h.full_frame().contains("No items"));
}

#[tokio::test]
async fn a_failed_send_frees_the_flight_and_a_closed_picker_sends_nothing() {
    let mut h = harness().await;
    open_picker(&mut h).await;
    h.app_mut().ac_mut().agent_connected = false;
    type_text(&mut h, "a");
    assert!(!h.app_mut().ac().sessions.search.is_in_flight());
    h.app_mut().ac_mut().agent_connected = true;
    type_text(&mut h, "b");
    let request = sent(&h.drain_commands().await, "search_session_metadata")[0].clone();
    assert_eq!(request["query"], "ab");
    h.app_mut().handle_resume_selector_key(&Key::Escape);
    answer(&mut h, &request, "AFTER-ESCAPE", json!({}));
    assert!(h.app_mut().ac().sessions.resume_selector.is_none());
    // The closed picker's search is nobody's: its answer settles and records nothing.
    assert!(!h.app_mut().ac().sessions.search.is_in_flight());
    assert!(!(h.app_mut().ac().sessions.home_versions).contains_key("cli:found"));
    assert!(h.drain_commands().await.is_empty());
    // No picker: a discovery request lists, and a queued search has nothing to send.
    let scope = crate::protocol::session_payloads::SessionListScope::Global;
    h.app_mut().request_session_discovery(scope);
    assert_eq!(sent(&h.drain_commands().await, "list_sessions").len(), 1);
}

#[tokio::test]
async fn a_harness_without_the_command_frees_the_flight_and_says_so() {
    let mut h = harness().await;
    open_picker(&mut h).await;
    // Not about a search, or no search in flight: not this tab's.
    h.app_mut()
        .handle_search_parse_error(Some("unknown variant `search_session_metadata`"));
    assert!(h.notification_messages().is_empty());
    type_text(&mut h, "a");
    let _ = h.drain_commands().await;
    let error = "unknown variant `search_session_metadata`, expected one of `prompt`";
    h.app_mut().handle_response(
        None,
        "parse_error".into(),
        false,
        None,
        Some("missing field `x`".into()),
    );
    assert!(h.app_mut().ac().sessions.search.is_in_flight());
    h.app_mut()
        .handle_response(None, "parse_error".into(), false, None, Some(error.into()));
    assert!(!h.app_mut().ac().sessions.search.is_in_flight());
    assert!(
        h.notification_messages()
            .iter()
            .any(|n| n.contains("newer quecto harness"))
    );
    assert!(h.full_frame().contains("LISTED"), "the rows stay");
    type_text(&mut h, "b");
    assert_eq!(
        sent(&h.drain_commands().await, "search_session_metadata").len(),
        1
    );
}
