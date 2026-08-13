//! Review-follow-up regression tests for #1061 paged history, split from
//! `app_paged_history_tests.rs` to keep that file within the source
//! line-count gate.

use super::app_paged_history_tests::{
    chat_text, drained_get_messages_commands, harness, page, prime_active_viewport, respond,
    widen_active_viewport,
};
use super::app_response::ATTACH_BACKFILL_ID;
use super::tui_harness::TuiHarness;
use super::*;
use crate::conversation::history_paging::{PENDING_HISTORY_PAGE_RETRY, PendingHistoryPage};

#[tokio::test]
async fn independent_clients_do_not_reuse_history_page_correlation_ids() {
    let (mut first, mut second) = (harness().await, harness().await);
    for h in [&mut first, &mut second] {
        let session = &mut h.app_mut().master_session;
        session.history.has_more_before = true;
        session.history.before_cursor = Some("cursor".into());
        session.chat.set_viewport_height(1);
    }
    let id = |h: &mut TuiHarness| h.app_mut().next_history_page_request().unwrap().0;
    assert_ne!(id(&mut first), id(&mut second));
}

#[tokio::test]
async fn stale_in_flight_page_is_retried_after_age_window() {
    // A response that is never delivered has no failure event to clear the
    // pending entry; once it ages past the retry window the same cursor must be
    // requestable again instead of wedging paging until a lifecycle reset
    // (#1061 review follow-up).
    let mut h = harness().await;
    {
        let session = &mut h.app_mut().master_session;
        session.history.has_more_before = true;
        session.history.before_cursor = Some("cursor".into());
        session.history.pending_page = Some(PendingHistoryPage {
            request_id: "history-page-lost".into(),
            before: "cursor".into(),
            requested_at: std::time::Instant::now()
                .checked_sub(PENDING_HISTORY_PAGE_RETRY)
                .expect("clock supports backdating by the retry window"),
        });
        session.chat.set_viewport_height(1);
    }
    let retry = h.app_mut().next_history_page_request();
    assert!(
        retry.is_some(),
        "a stale in-flight page must be retryable for the same cursor"
    );
    // The fresh request replaces the lost one, so a fresh pending now dedupes.
    assert!(h.app_mut().next_history_page_request().is_none());
}

#[tokio::test]
async fn late_twin_of_stale_retried_page_is_dropped() {
    // The load-bearing claim behind PENDING_HISTORY_PAGE_RETRY: if the
    // presumed-lost reply eventually arrives AFTER a retry replaced it, exact
    // request-id correlation must drop the late twin, and only the fresh
    // request's reply may apply (#1061 review follow-up).
    let mut h = harness().await;
    respond(
        h.app_mut(),
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        page(&[("m3", "newest-page")], Some("m3"), true),
    );
    let _ = h.drain_commands().await;
    prime_active_viewport(h.app_mut());
    h.app_mut().handle_key(Key::PageUp);
    let first = drained_get_messages_commands(&mut h).await;
    let stale_id = first[0]["id"]
        .as_str()
        .expect("first older-page request id")
        .to_string();

    // Age the in-flight request past the retry window, then retry the cursor.
    h.app_mut()
        .master_session
        .history
        .pending_page
        .as_mut()
        .expect("request is in flight")
        .requested_at = std::time::Instant::now()
        .checked_sub(PENDING_HISTORY_PAGE_RETRY)
        .expect("clock supports backdating by the retry window");
    prime_active_viewport(h.app_mut());
    h.app_mut().handle_key(Key::PageUp);
    let retry = drained_get_messages_commands(&mut h).await;
    let fresh_id = retry[0]["id"]
        .as_str()
        .expect("retried older-page request id")
        .to_string();
    assert_ne!(stale_id, fresh_id, "retry must mint a fresh correlation id");

    // The presumed-lost twin finally arrives: it must be dropped.
    respond(
        h.app_mut(),
        Some(&stale_id),
        "get_messages",
        true,
        page(&[("m1", "stale twin page")], None, false),
    );
    widen_active_viewport(h.app_mut());
    let frame = chat_text(h.app_mut());
    assert!(
        !frame.contains("stale twin page"),
        "the late twin of a retried page must not apply:\n{frame}"
    );

    // The fresh request's reply still applies normally.
    respond(
        h.app_mut(),
        Some(&fresh_id),
        "get_messages",
        true,
        page(&[("m1", "fresh older page")], None, false),
    );
    widen_active_viewport(h.app_mut());
    let frame = chat_text(h.app_mut());
    assert!(
        frame.contains("fresh older page"),
        "the fresh retried page must still apply:\n{frame}"
    );
}

#[tokio::test]
async fn idless_snapshot_after_paging_replaces_whole_prefix_without_duplication() {
    // #1061 review follow-up: `partial_backfill_len` tracks the TOTAL loaded
    // backfill prefix. A busy-connect snapshot (id-less) landing after the user
    // has paged back must replace the entire loaded prefix — replacing only the
    // most recent page would duplicate the newest slice and open a gap.
    let mut h = harness().await;
    respond(
        h.app_mut(),
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        page(
            &[("m3", "third message"), ("m4", "fourth message")],
            Some("m3"),
            true,
        ),
    );
    let _ = h.drain_commands().await;
    prime_active_viewport(h.app_mut());
    h.app_mut().handle_key(Key::PageUp);
    let commands = drained_get_messages_commands(&mut h).await;
    let request_id = commands[0]["id"]
        .as_str()
        .expect("older-page request id")
        .to_string();
    respond(
        h.app_mut(),
        Some(&request_id),
        "get_messages",
        true,
        page(
            &[("m1", "first message"), ("m2", "second message")],
            Some("m1"),
            true,
        ),
    );

    // Id-less busy-connect snapshot of the newest page arrives late.
    respond(
        h.app_mut(),
        None,
        "get_messages",
        true,
        page(
            &[("m3", "third message"), ("m4", "fourth message")],
            Some("m3"),
            true,
        ),
    );

    widen_active_viewport(h.app_mut());
    let frame = chat_text(h.app_mut());
    assert_eq!(
        frame.matches("third message").count(),
        1,
        "the snapshot must not duplicate the newest slice:\n{frame}"
    );
    assert_eq!(
        frame.matches("fourth message").count(),
        1,
        "the snapshot must not duplicate the newest slice:\n{frame}"
    );
    assert!(
        !frame.contains("first message"),
        "the whole loaded prefix is replaced (older pages re-fetchable via the cursor):\n{frame}"
    );

    // The snapshot's cursor restarts paging, so the replaced older history
    // remains reachable.
    let _ = h.drain_commands().await;
    prime_active_viewport(h.app_mut());
    h.app_mut().handle_key(Key::PageUp);
    let commands = drained_get_messages_commands(&mut h).await;
    assert!(
        commands
            .iter()
            .any(|cmd| cmd.get("before").and_then(|v| v.as_str()) == Some("m3")),
        "paging must restart from the snapshot's cursor; commands={commands:?}"
    );
}

#[tokio::test]
async fn oversized_stub_recall_requests_each_page_and_reassembles_body() {
    let mut h = harness().await;
    respond(
        h.app_mut(),
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        serde_json::json!({
            "messages": [{
                "id": "oversized-stub",
                "role": "assistant",
                "content": "[assistant stub — recall available]",
                "collapsed": true,
                "contentLength": 26,
            }],
            "hasMoreBefore": false,
            "before": null,
        }),
    );
    let _ = h.drain_commands().await;

    prime_active_viewport(h.app_mut());
    h.app_mut().handle_key(Key::PageUp);
    let commands = h.drain_commands().await;
    let first = commands
        .iter()
        .find_map(|line| {
            let cmd = serde_json::from_str::<serde_json::Value>(line).ok()?;
            (cmd.get("type").and_then(|v| v.as_str()) == Some("get_message")).then_some(cmd)
        })
        .expect("visible oversized stub must issue first ranged recall");
    assert_eq!(
        first.get("messageId").and_then(|v| v.as_str()),
        Some("oversized-stub")
    );
    assert_eq!(first.get("offset").and_then(|v| v.as_u64()), Some(0));
    assert!(
        first.get("limit").and_then(|v| v.as_u64()).is_some(),
        "oversized recall must request a bounded page: {first}"
    );
    let first_req = first
        .get("id")
        .and_then(|v| v.as_str())
        .expect("first recall request carries a correlation id")
        .to_string();

    respond(
        h.app_mut(),
        Some(&first_req),
        "get_message",
        true,
        serde_json::json!({
            "id": "oversized-stub",
            "role": "assistant",
            "content": "abcdefghijkl",
            "offset": 0,
            "nextOffset": 12,
            "contentLength": 26,
            "hasMoreContent": true,
        }),
    );
    let commands = h.drain_commands().await;
    let second = commands
        .iter()
        .find_map(|line| {
            let cmd = serde_json::from_str::<serde_json::Value>(line).ok()?;
            (cmd.get("type").and_then(|v| v.as_str()) == Some("get_message")).then_some(cmd)
        })
        .expect("partial recall response must request the next range");
    assert_eq!(
        second.get("messageId").and_then(|v| v.as_str()),
        Some("oversized-stub")
    );
    assert_eq!(second.get("offset").and_then(|v| v.as_u64()), Some(12));
    let second_req = second
        .get("id")
        .and_then(|v| v.as_str())
        .expect("second recall request carries a correlation id")
        .to_string();

    respond(
        h.app_mut(),
        Some(&second_req),
        "get_message",
        true,
        serde_json::json!({
            "id": "oversized-stub",
            "role": "assistant",
            "content": "mnopqrstuvwxyz",
            "offset": 12,
            "nextOffset": 26,
            "contentLength": 26,
            "hasMoreContent": false,
        }),
    );

    h.app_mut().active_chat_mut().scroll_down(1000);
    let frame = chat_text(h.app_mut());
    assert!(
        frame.contains("abcdefghijklmnopqrstuvwxyz"),
        "all recalled pages must replace the stub as one complete body: {frame}"
    );
    assert!(
        !frame.contains("recall available"),
        "stub body must be gone after paged recall completes: {frame}"
    );
}

#[tokio::test]
async fn stub_recall_rejects_first_page_content_length_that_differs_from_stub_metadata() {
    let mut h = harness().await;
    respond(
        h.app_mut(),
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        serde_json::json!({
            "messages": [{
                "id": "length-pinned-stub",
                "role": "assistant",
                "content": "[assistant stub — recall available]",
                "collapsed": true,
                "contentLength": 26,
            }],
            "hasMoreBefore": false,
            "before": null,
        }),
    );
    let _ = h.drain_commands().await;

    prime_active_viewport(h.app_mut());
    h.app_mut().handle_key(Key::PageUp);
    let first_req = h
        .drain_commands()
        .await
        .iter()
        .find_map(|line| {
            let cmd = serde_json::from_str::<serde_json::Value>(line).ok()?;
            (cmd.get("type").and_then(|v| v.as_str()) == Some("get_message"))
                .then(|| cmd.get("id").and_then(|v| v.as_str()).unwrap().to_string())
        })
        .expect("visible stub must issue recall");

    respond(
        h.app_mut(),
        Some(&first_req),
        "get_message",
        true,
        serde_json::json!({
            "id": "length-pinned-stub",
            "role": "assistant",
            "content": "abcdefghijkl",
            "offset": 0,
            "nextOffset": 12,
            "contentLength": 27,
            "hasMoreContent": true,
        }),
    );

    h.app_mut().active_chat_mut().scroll_down(1000);
    let frame = chat_text(h.app_mut());
    assert!(
        frame.contains("recall available"),
        "mismatched contentLength must preserve the stub: {frame}"
    );
    assert!(
        h.app_mut()
            .conn
            .failed_stub_recalls
            .contains(&(None, "length-pinned-stub".to_string())),
        "mismatched first page must mark the stub recall failed"
    );
}

#[tokio::test]
async fn wholesale_replacement_with_stale_prefix_keeps_later_live_entries() {
    let mut h = harness().await;
    respond(
        h.app_mut(),
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        page(&[("o3", "old newest")], Some("o3"), true),
    );
    let _ = h.drain_commands().await;
    prime_active_viewport(h.app_mut());
    h.app_mut().handle_key(Key::PageUp);
    let commands = drained_get_messages_commands(&mut h).await;
    let older_id = commands[0]["id"]
        .as_str()
        .expect("older-page id")
        .to_string();
    respond(
        h.app_mut(),
        Some(&older_id),
        "get_messages",
        true,
        page(
            &[("o1", "old first"), ("o2", "old second")],
            Some("o1"),
            true,
        ),
    );

    respond(
        h.app_mut(),
        Some("resume-messages"),
        "get_messages",
        true,
        page(&[("r1", "replacement newest")], Some("r1"), true),
    );
    h.app_mut()
        .master_session
        .chat
        .add_entry(ChatEntry::Assistant {
            text: "post replacement live entry".into(),
            streaming: false,
        });

    respond(
        h.app_mut(),
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        page(&[("r0", "replacement snapshot older")], Some("r0"), true),
    );

    widen_active_viewport(h.app_mut());
    let frame = chat_text(h.app_mut());
    assert!(frame.contains("replacement snapshot older"), "{frame}");
    assert!(
        frame.contains("post replacement live entry"),
        "stale prefix must not delete live replacement entries:\n{frame}"
    );
    assert!(!frame.contains("old first"), "{frame}");
}

#[tokio::test]
async fn short_wholesale_replacement_survives_longer_stale_prefix_on_later_snapshot() {
    let mut h = harness().await;
    respond(
        h.app_mut(),
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        page(
            &[("o1", "old one"), ("o2", "old two"), ("o3", "old three")],
            Some("o1"),
            true,
        ),
    );

    respond(
        h.app_mut(),
        Some("resume-messages"),
        "get_messages",
        true,
        page(&[("r1", "short replacement")], Some("r1"), true),
    );
    respond(
        h.app_mut(),
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        page(&[("r0", "short replacement older")], Some("r0"), true),
    );

    widen_active_viewport(h.app_mut());
    let frame = chat_text(h.app_mut());
    assert!(frame.contains("short replacement"), "{frame}");
}
