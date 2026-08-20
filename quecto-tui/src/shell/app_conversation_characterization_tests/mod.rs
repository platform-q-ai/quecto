//! Characterization pins for the conversation history/recovery parity contract
//! (#1221). These cover parity-contract boundary rows that the pre-existing
//! #1060/#1061 suites do not assert explicitly, so the extraction of the
//! conversation module can be proven behaviour-preserving.
//!
//! Every test drives the production entry points (key input, event handling,
//! response handling) and asserts on rendered frames or emitted commands.

use super::app_paged_history_tests::{
    chat_text, drained_get_messages_commands, harness, page, prime_active_viewport, respond,
    widen_active_viewport,
};
use super::app_response::ATTACH_BACKFILL_ID;
use super::tui_harness::TuiHarness;
use super::*;

fn get_message_commands(lines: &[String]) -> Vec<serde_json::Value> {
    lines
        .iter()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|cmd| cmd.get("type").and_then(|v| v.as_str()) == Some("get_message"))
        .collect()
}

/// Contract row "Older-page request emission": `hasMoreBefore=false` advertises
/// no older history, so scrolling to the top must emit no page request.
#[tokio::test]
async fn scroll_back_emits_no_page_request_when_no_older_history_is_advertised() {
    let mut h = harness().await;
    respond(
        h.app_mut(),
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        page(&[("m1", "only page")], Some("m1"), false),
    );
    let _ = h.drain_commands().await;
    prime_active_viewport(h.app_mut());

    h.app_mut().handle_key(Key::PageUp);

    assert!(
        drained_get_messages_commands(&mut h).await.is_empty(),
        "no older history advertised must emit no older-page request"
    );
}

/// Contract row "Older-page request emission": a payload advertising more
/// history but carrying no `before` cursor cannot be paged.
#[tokio::test]
async fn scroll_back_emits_no_page_request_without_a_cursor() {
    let mut h = harness().await;
    respond(
        h.app_mut(),
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        page(&[("m1", "cursorless page")], None, true),
    );
    let _ = h.drain_commands().await;
    prime_active_viewport(h.app_mut());

    h.app_mut().handle_key(Key::PageUp);

    assert!(
        drained_get_messages_commands(&mut h).await.is_empty(),
        "a missing before-cursor must suppress the older-page request"
    );
}

/// Contract row "Page correlation": ids sharing our prefix but differing in the
/// suffix must be rejected — only an EXACT match may prepend.
#[tokio::test]
async fn page_response_with_prefix_matching_id_is_rejected() {
    let mut h = harness().await;
    respond(
        h.app_mut(),
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        page(&[("m3", "newest page")], Some("m3"), true),
    );
    let _ = h.drain_commands().await;
    prime_active_viewport(h.app_mut());
    h.app_mut().handle_key(Key::PageUp);
    let commands = drained_get_messages_commands(&mut h).await;
    let request_id = commands[0]["id"].as_str().expect("page id").to_string();

    respond(
        h.app_mut(),
        Some(&format!("{request_id}-extra")),
        "get_messages",
        true,
        page(&[("x1", "near miss page")], None, false),
    );

    widen_active_viewport(h.app_mut());
    let frame = chat_text(h.app_mut());
    assert!(
        !frame.contains("near miss page"),
        "a prefix-only id match must not prepend history:\n{frame}"
    );
}

/// Contract row "Backfill reconcile": an empty page still publishes its cursors
/// (and must not latch the backfill guard), so paging stays reachable.
#[tokio::test]
async fn empty_backfill_page_still_publishes_its_paging_cursor() {
    let mut h = harness().await;
    respond(
        h.app_mut(),
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        page(&[], Some("m5"), true),
    );
    let _ = h.drain_commands().await;
    // Give the viewport a live entry to anchor on, then scroll to the top.
    h.app_mut().handle_event(Event::Token {
        token: "live token".into(),
    });
    prime_active_viewport(h.app_mut());

    h.app_mut().handle_key(Key::PageUp);

    let commands = drained_get_messages_commands(&mut h).await;
    assert!(
        commands
            .iter()
            .any(|cmd| cmd.get("before").and_then(|v| v.as_str()) == Some("m5")),
        "an empty page must still publish its cursor for paging; commands={commands:?}"
    );
}

/// Contract row "Zero-fetch / replacement paths": a legacy resume payload
/// without paging metadata still clears the transcript and shows the status.
#[tokio::test]
async fn legacy_resume_payload_without_paging_metadata_replaces_transcript() {
    let mut h = harness().await;
    h.app_mut().handle_event(Event::Token {
        token: "pre-resume live text".into(),
    });

    respond(
        h.app_mut(),
        Some("resume-messages"),
        "get_messages",
        true,
        serde_json::json!({
            "messages": [{"role": "user", "content": "legacy resumed turn"}],
        }),
    );

    widen_active_viewport(h.app_mut());
    let frame = chat_text(h.app_mut());
    assert!(
        frame.contains("legacy resumed turn"),
        "legacy payload must render its messages:\n{frame}"
    );
    assert!(
        frame.contains("Session resumed"),
        "legacy payload must still announce the resume:\n{frame}"
    );
    assert!(
        !frame.contains("pre-resume live text"),
        "legacy payload must clear the live transcript:\n{frame}"
    );
}

/// Contract row "Stub recall": a response body whose `id` disagrees with the
/// requested stub is rejected and never retried.
#[tokio::test]
async fn stub_recall_rejects_mismatched_body_id_and_does_not_retry() {
    let mut h = harness().await;
    respond(
        h.app_mut(),
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        serde_json::json!({
            "messages": [{
                "id": "stub-mismatch",
                "role": "assistant",
                "content": "[assistant stub — recall available]",
                "collapsed": true,
            }],
            "hasMoreBefore": false,
            "before": null,
        }),
    );
    let _ = h.drain_commands().await;
    prime_active_viewport(h.app_mut());
    h.app_mut().handle_key(Key::PageUp);
    let commands = h.drain_commands().await;
    let req_id = get_message_commands(&commands)
        .first()
        .and_then(|cmd| cmd.get("id").and_then(|v| v.as_str()).map(str::to_owned))
        .expect("visible stub must issue a recall");

    respond(
        h.app_mut(),
        Some(&req_id),
        "get_message",
        true,
        serde_json::json!({
            "id": "some-other-message",
            "role": "assistant",
            "content": "body for a different message",
        }),
    );

    h.app_mut().active_chat_mut().scroll_down(1000);
    let frame = chat_text(h.app_mut());
    assert!(
        frame.contains("recall available"),
        "a mismatched body id must preserve the stub:\n{frame}"
    );
    assert!(
        !frame.contains("body for a different message"),
        "a mismatched body must never be applied:\n{frame}"
    );

    prime_active_viewport(h.app_mut());
    h.app_mut().handle_key(Key::PageUp);
    assert!(
        get_message_commands(&h.drain_commands().await).is_empty(),
        "a rejected recall must not be retried on the next scroll"
    );

    // Positive control: the same scroll in the same state DOES fetch a stub
    // that has not been marked failed, so the assertion above is not vacuous.
    let mut fresh = harness().await;
    respond(
        fresh.app_mut(),
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        serde_json::json!({
            "messages": [{
                "id": "stub-fresh",
                "role": "assistant",
                "content": "[assistant stub — recall available]",
                "collapsed": true,
            }],
            "hasMoreBefore": false,
            "before": null,
        }),
    );
    let _ = fresh.drain_commands().await;
    prime_active_viewport(fresh.app_mut());
    fresh.app_mut().handle_key(Key::PageUp);
    assert!(
        !get_message_commands(&fresh.drain_commands().await).is_empty(),
        "control: an unfailed visible stub must still be recalled on scroll"
    );
}

/// Contract row "Ref-based turn recovery": a shorter-than-advertised assistant
/// body (`contentLength`) triggers recovery even when refs and tool
/// cardinality agree.
#[tokio::test]
async fn truncated_assistant_body_triggers_recovery_via_advertised_content_length() {
    let mut h = TuiHarness::new().await;
    {
        let a = h.app_mut();
        a.handle_event(Event::AgentStart);
        a.handle_event(Event::Token {
            token: "short".into(),
        });
        a.handle_event(Event::TurnEnd {
            message: serde_json::json!({
                "role": "assistant",
                "content": "",
                "messageRefs": ["11111111-1111-1111-1111-111111111111"],
                "contentLength": 9_000,
            }),
        });
    }

    assert!(
        !get_message_commands(&h.drain_commands().await).is_empty(),
        "an assistant body shorter than the advertised length must trigger recovery"
    );

    // Negative control: the SAME turn whose body meets the advertised length
    // must NOT trigger recovery, so the trigger is length-driven and not
    // merely ref-presence-driven.
    let mut satisfied = TuiHarness::new().await;
    {
        let a = satisfied.app_mut();
        a.handle_event(Event::AgentStart);
        a.handle_event(Event::Token {
            token: "a full length assistant body".into(),
        });
        a.handle_event(Event::TurnEnd {
            message: serde_json::json!({
                "role": "assistant",
                "content": "a full length assistant body",
                "messageRefs": ["11111111-1111-1111-1111-111111111111"],
                "contentLength": "a full length assistant body".len(),
            }),
        });
    }
    assert!(
        get_message_commands(&satisfied.drain_commands().await).is_empty(),
        "control: a body meeting the advertised length must not trigger recovery"
    );
}

/// Contract row "Ref-based turn recovery": the atomic range replacement happens
/// only once EVERY ref has responded — a partially answered batch must leave
/// the transcript untouched.
#[tokio::test]
async fn partially_answered_recovery_batch_does_not_mutate_the_transcript() {
    let mut h = TuiHarness::new().await;
    {
        let a = h.app_mut();
        a.handle_event(Event::AgentStart);
        a.handle_event(Event::TurnEnd {
            message: serde_json::json!({
                "role": "assistant",
                "content": "",
                "messageRefs": [
                    "11111111-1111-1111-1111-111111111111",
                    "22222222-2222-2222-2222-222222222222",
                    "33333333-3333-3333-3333-333333333333",
                ],
            }),
        });
    }
    let commands = get_message_commands(&h.drain_commands().await);
    let first_id = commands
        .first()
        .and_then(|cmd| cmd.get("id").and_then(|v| v.as_str()).map(str::to_owned))
        .expect("recovery must fetch the refs");
    let first_message_id = commands[0]["messageId"]
        .as_str()
        .expect("recovery request carries the message id")
        .to_string();

    respond(
        h.app_mut(),
        Some(&first_id),
        "get_message",
        true,
        serde_json::json!({
            "id": first_message_id,
            "role": "assistant",
            "content": "partial batch content",
        }),
    );

    let frame = chat_text(h.app_mut());
    assert!(
        !frame.contains("partial batch content"),
        "an incomplete batch must not be applied to the transcript:\n{frame}"
    );

    // Positive control: once the REMAINING refs answer, the batch is applied —
    // proving the assertion above pins atomicity, not a permanently dead path.
    for cmd in commands.iter().skip(1) {
        let id = cmd["id"].as_str().expect("request id").to_string();
        let message_id = cmd["messageId"].as_str().expect("message id").to_string();
        respond(
            h.app_mut(),
            Some(&id),
            "get_message",
            true,
            serde_json::json!({
                "id": message_id,
                "role": "assistant",
                "content": "partial batch content",
            }),
        );
    }
    widen_active_viewport(h.app_mut());
    let frame = chat_text(h.app_mut());
    assert!(
        frame.contains("partial batch content"),
        "control: a fully answered batch must be applied atomically:\n{frame}"
    );
}

/// Contract row "Command ordering / FIFO": a failed enqueue of an older-page
/// request rolls the pending entry back so the same cursor can be retried.
#[tokio::test]
async fn failed_page_enqueue_rolls_back_the_pending_request() {
    let mut h = harness().await;
    respond(
        h.app_mut(),
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        page(&[("m3", "newest page")], Some("m3"), true),
    );
    let _ = h.drain_commands().await;
    prime_active_viewport(h.app_mut());
    h.app_mut().handle_key(Key::PageUp);
    let commands = drained_get_messages_commands(&mut h).await;
    let request_id = commands[0]["id"].as_str().expect("page id").to_string();

    // Go through the REAL failure entry point, not the rollback helper alone,
    // so the notify + rollback pairing stays pinned.
    h.app_mut().handle_command_send_failure(CommandSendFailure {
        command: Command::GetMessages {
            agent_id: None,
            id: Some(request_id),
            before: Some("m3".into()),
            count: None,
        },
        error: "channel full".into(),
        connection: MASTER_CONNECTION_ID.to_string(),
    });

    prime_active_viewport(h.app_mut());
    h.app_mut().handle_key(Key::PageUp);
    let retry = drained_get_messages_commands(&mut h).await;
    assert!(
        retry
            .iter()
            .any(|cmd| cmd.get("before").and_then(|v| v.as_str()) == Some("m3")),
        "a rolled-back page request must be immediately retryable; commands={retry:?}"
    );
}

/// Contract row "Command ordering / FIFO": rolling back a `GetMessages` id that
/// is not the in-flight page must be a no-op (the real request stays deduped).
#[tokio::test]
async fn rollback_of_an_unrelated_page_id_is_a_no_op() {
    let mut h = harness().await;
    respond(
        h.app_mut(),
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        page(&[("m3", "newest page")], Some("m3"), true),
    );
    let _ = h.drain_commands().await;
    prime_active_viewport(h.app_mut());
    h.app_mut().handle_key(Key::PageUp);
    let _ = drained_get_messages_commands(&mut h).await;

    h.app_mut().handle_command_send_failure(CommandSendFailure {
        command: Command::GetMessages {
            agent_id: None,
            id: Some("history-page-someone-else-1".into()),
            before: Some("m3".into()),
            count: None,
        },
        error: "channel full".into(),
        connection: MASTER_CONNECTION_ID.to_string(),
    });

    prime_active_viewport(h.app_mut());
    h.app_mut().handle_key(Key::PageUp);
    assert!(
        drained_get_messages_commands(&mut h).await.is_empty(),
        "an unrelated rollback must not clear our in-flight page request"
    );
}

/// Contract row "Ref-based turn recovery" + "Range assembly": an oversized
/// recovered body is fetched page by page (`offset`/`limit`) and reassembled
/// exactly before the turn range is replaced. This pins the `Continue` arm in
/// `app_message_recovery`, which duplicates the stub-recall paging logic.
#[tokio::test]
async fn oversized_recovery_ref_pages_and_reassembles_before_replacing_the_turn() {
    let page_bytes = super::app_paged_history::GET_MESSAGE_PAGE_BYTES;
    let pages = [
        "A".repeat(page_bytes),
        "B".repeat(page_bytes),
        "C".repeat(64),
    ];
    let total: usize = pages.iter().map(String::len).sum();

    let mut h = TuiHarness::new().await;
    {
        let a = h.app_mut();
        a.handle_event(Event::AgentStart);
        a.handle_event(Event::TurnEnd {
            message: serde_json::json!({
                "role": "assistant",
                "content": "",
                "messageRefs": ["11111111-1111-1111-1111-111111111111"],
            }),
        });
    }

    let mut offset = 0usize;
    for (i, page) in pages.iter().enumerate() {
        let commands = get_message_commands(&h.drain_commands().await);
        let request = commands
            .last()
            .unwrap_or_else(|| panic!("page {i} must be requested"));
        assert_eq!(
            request.get("offset").and_then(serde_json::Value::as_u64),
            Some(offset as u64),
            "page {i} must be requested at the accumulated offset"
        );
        assert_eq!(
            request.get("limit").and_then(serde_json::Value::as_u64),
            Some(page_bytes as u64),
            "each recovery page must be bounded by GET_MESSAGE_PAGE_BYTES"
        );
        let id = request["id"].as_str().expect("request id").to_string();
        let message_id = request["messageId"]
            .as_str()
            .expect("message id")
            .to_string();
        let last = i == pages.len() - 1;
        offset += page.len();
        respond(
            h.app_mut(),
            Some(&id),
            "get_message",
            true,
            serde_json::json!({
                "id": message_id,
                "role": "assistant",
                "content": page,
                "offset": offset - page.len(),
                "contentLength": total,
                "hasMoreContent": !last,
                "nextOffset": offset,
            }),
        );
    }

    let expected = pages.concat();
    let texts = h.master_assistant_texts();
    assert!(
        texts.iter().any(|text| text == &expected),
        "the oversized recovered body must be reassembled exactly before replacing the turn; \
         got assistant entry lengths {:?} (expected one of length {})",
        texts.iter().map(String::len).collect::<Vec<_>>(),
        expected.len()
    );
}

/// Contract row "Stub recall": a control sequence SPLIT across two recall pages
/// must be sanitized once AFTER reassembly, so it is neutralised exactly as an
/// unsplit sequence would be. Sanitizing per page would let the halves rejoin
/// into a live escape sequence in the transcript.
#[tokio::test]
async fn a_control_sequence_split_across_recall_pages_is_sanitized_after_reassembly() {
    let page_bytes = super::app_paged_history::GET_MESSAGE_PAGE_BYTES;
    // The escape straddles the page boundary: "\u{1b}[" ends page one, "31m" opens page two.
    let head = format!("{}\u{1b}[", "h".repeat(page_bytes - 2));
    let tail = "31mRED".to_string();
    let total = head.len() + tail.len();

    let mut h = harness().await;
    respond(
        h.app_mut(),
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        serde_json::json!({
            "messages": [{
                "id": "stub-split",
                "role": "assistant",
                "content": "[assistant stub — recall available]",
                "collapsed": true,
            }],
            "hasMoreBefore": false,
            "before": null,
        }),
    );
    let _ = h.drain_commands().await;
    prime_active_viewport(h.app_mut());
    h.app_mut().handle_key(Key::PageUp);

    for (i, page) in [head.clone(), tail.clone()].iter().enumerate() {
        let commands = get_message_commands(&h.drain_commands().await);
        let request = commands.last().expect("recall page must be requested");
        let id = request["id"].as_str().expect("request id").to_string();
        let last = i == 1;
        respond(
            h.app_mut(),
            Some(&id),
            "get_message",
            true,
            serde_json::json!({
                "id": "stub-split",
                "role": "assistant",
                "content": page,
                "offset": if last { head.len() } else { 0 },
                "contentLength": total,
                "hasMoreContent": !last,
                "nextOffset": head.len(),
            }),
        );
    }

    let texts = h.master_assistant_texts();
    let recalled = texts
        .iter()
        .find(|text| text.contains("RED"))
        .expect("the recalled body must reach the transcript");
    // Assert the PAYLOAD, not just the absence of an ESC byte: a per-page
    // sanitizer would consume page one's dangling `ESC[` via the
    // unterminated-CSI branch, leaving no ESC but a visible literal `31m`.
    assert!(
        !recalled.contains('\u{1b}'),
        "the rejoined escape must be sanitized after reassembly, got {recalled:?}"
    );
    assert!(
        recalled.ends_with("RED") && !recalled.contains("31m"),
        "sanitizing per page would leave the escape's tail as literal text; \
         the sequence must be neutralised as a whole, got {recalled:?}"
    );
}

/// Contract row "Stub recall": a visible stub is fetched at most once while a
/// recall is IN FLIGHT. Without the pending-recall dedupe every scroll would
/// re-issue the same request, amplifying load on a slow server.
#[tokio::test]
async fn a_visible_stub_is_fetched_at_most_once_while_in_flight() {
    let mut h = harness().await;
    respond(
        h.app_mut(),
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        serde_json::json!({
            "messages": [{
                "id": "stub-inflight",
                "role": "assistant",
                "content": "[assistant stub — recall available]",
                "collapsed": true,
            }],
            "hasMoreBefore": false,
            "before": null,
        }),
    );
    let _ = h.drain_commands().await;

    prime_active_viewport(h.app_mut());
    h.app_mut().handle_key(Key::PageUp);
    assert_eq!(
        get_message_commands(&h.drain_commands().await).len(),
        1,
        "the first scroll must issue exactly one recall"
    );

    // Scroll again with the recall still unanswered.
    prime_active_viewport(h.app_mut());
    h.app_mut().handle_key(Key::PageUp);
    assert!(
        get_message_commands(&h.drain_commands().await).is_empty(),
        "an in-flight recall must not be re-issued on a further scroll"
    );
}

/// Contract row "Backfill reconcile": a page that is BOTH trimmed and
/// advertises more history must NOT latch `history_backfilled`, so a later
/// broadcast snapshot is still reconciled into the transcript. Latching early
/// would freeze the loaded prefix and strand the newer snapshot.
#[tokio::test]
async fn a_trimmed_page_advertising_more_history_stays_open_to_later_snapshots() {
    let mut h = harness().await;
    respond(
        h.app_mut(),
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        serde_json::json!({
            "messages": [{"id": "m8", "role": "user", "content": "trimmed newest"}],
            // `trimmed` ALONE must keep the backfill open. Setting
            // `hasMoreBefore` too would give the latch a second sufficient
            // condition, and the test would survive removal of the `trimmed`
            // guard it is named for.
            "trimmed": true,
            "hasMoreBefore": false,
            "before": "m8",
        }),
    );
    let _ = h.drain_commands().await;

    // A later id-less broadcast snapshot must still be reconciled.
    respond(
        h.app_mut(),
        None,
        "get_messages",
        true,
        serde_json::json!({
            "messages": [
                {"id": "m7", "role": "user", "content": "later snapshot older"},
                {"id": "m8", "role": "user", "content": "trimmed newest"},
            ],
            "hasMoreBefore": false,
            "before": null,
        }),
    );

    widen_active_viewport(h.app_mut());
    let frame = chat_text(h.app_mut());
    assert!(
        frame.contains("later snapshot older"),
        "an unlatched trimmed prefix must accept a later snapshot:\n{frame}"
    );
}

#[cfg(test)]
mod thinking_recovery_tests;
