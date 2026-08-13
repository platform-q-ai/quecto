//! Multi-client correlation characterization for solicited get_messages (#1237).
//!
//! `get_messages` responses are broadcast to every UDS client. Fixed literal
//! request ids let Client A's resume/rewind/attach clobber Client B. Dual
//! harnesses simulate two logical TUI clients without two real sockets.

use super::app_paged_history_tests::{chat_text, harness, page, respond, widen_active_viewport};
use super::tui_harness::TuiHarness;
use super::*;

fn raw_respond(app: &mut App, id: Option<&str>, data: serde_json::Value) {
    app.handle_response(
        id.map(String::from),
        "get_messages".into(),
        true,
        Some(data),
        None,
    );
}

fn seed_client_b_transcript(app: &mut App) {
    app.conn.master_session.chat.add_entry(ChatEntry::User {
        text: "client-b private turn".into(),
    });
    app.conn.master_session.history.before_cursor = Some("b-cursor".into());
    app.conn.master_session.history.has_more_before = true;
    app.conn.master_session.history.partial_prefix_len = Some(1);
}

fn b_frame(app: &mut App) -> String {
    widen_active_viewport(app);
    chat_text(app)
}

fn legacy_messages(user: &str, assistant: &str) -> serde_json::Value {
    serde_json::json!({
        "messages": [
            {"role": "user", "content": user},
            {"role": "assistant", "content": assistant},
        ]
    })
}

async fn mint_resume_id(h: &mut TuiHarness) -> String {
    h.app_mut().handle_response(
        Some("resume".into()),
        "resume_session".into(),
        true,
        Some(serde_json::json!({"session": "s-a"})),
        None,
    );
    let id = h
        .app_mut()
        .test_pending_resume_messages_id()
        .expect("resume mints pending id")
        .to_string();
    let _ = h.drain_commands().await;
    id
}

async fn mint_rewind_refresh_id(h: &mut TuiHarness) -> String {
    h.app_mut().conn.rewind.pending_apply_id = Some("rw-a".into());
    h.app_mut()
        .handle_response(Some("rw-a".into()), "rewind_to".into(), true, None, None);
    let id = h
        .app_mut()
        .test_pending_rewind_refresh_id()
        .expect("rewind_to mints refresh id")
        .to_string();
    let _ = h.drain_commands().await;
    id
}

async fn mint_attach_id(h: &mut TuiHarness) -> String {
    h.app_mut().request_master_attach_backfill();
    let id = h
        .app_mut()
        .test_pending_attach_backfill_id()
        .expect("attach mints pending id")
        .to_string();
    let _ = h.drain_commands().await;
    id
}

#[tokio::test]
async fn foreign_resume_does_not_replace_other_client_transcript() {
    let (mut a, mut b) = (harness().await, harness().await);
    seed_client_b_transcript(b.app_mut());
    let before_cursor = b
        .app_mut()
        .conn
        .master_session
        .history
        .before_cursor
        .clone();
    let before_frame = b_frame(b.app_mut());

    let resume_id = mint_resume_id(&mut a).await;
    // Broadcast of A's solicited resume response reaches B.
    respond(
        b.app_mut(),
        Some(&resume_id),
        "get_messages",
        true,
        legacy_messages("a resumed user", "a resumed assistant"),
    );

    let frame = b_frame(b.app_mut());
    assert_eq!(
        frame, before_frame,
        "foreign resume must not mutate B's transcript:\nbefore:\n{before_frame}\nafter:\n{frame}"
    );
    assert!(
        !frame.contains("Session resumed"),
        "B must not show Session resumed for A's resume:\n{frame}"
    );
    assert_eq!(
        b.app_mut().conn.master_session.history.before_cursor,
        before_cursor,
        "foreign resume must not clobber B paging cursors"
    );
}

#[tokio::test]
async fn foreign_rewind_refresh_does_not_replace_other_client_transcript() {
    let (mut a, mut b) = (harness().await, harness().await);
    seed_client_b_transcript(b.app_mut());
    let before_frame = b_frame(b.app_mut());

    let refresh_id = mint_rewind_refresh_id(&mut a).await;
    respond(
        b.app_mut(),
        Some(&refresh_id),
        "get_messages",
        true,
        legacy_messages("a rewound user", "a rewound assistant"),
    );

    let frame = b_frame(b.app_mut());
    assert_eq!(
        frame, before_frame,
        "foreign rewind-refresh must not mutate B"
    );
    assert!(
        !frame.contains("Conversation rewound"),
        "B must not show Conversation rewound for A's refresh:\n{frame}"
    );
}

#[tokio::test]
async fn foreign_rewind_open_does_not_replace_or_open_selector() {
    let mut b = harness().await;
    seed_client_b_transcript(b.app_mut());
    b.app_mut().conn.rewind.pending_open_id = Some("rewind-open-local".into());
    let before_frame = b_frame(b.app_mut());

    respond(
        b.app_mut(),
        Some("rewind-open-foreign-1"),
        "get_messages",
        true,
        legacy_messages("foreign rewind open", "foreign assistant"),
    );

    let frame = b_frame(b.app_mut());
    assert_eq!(
        frame, before_frame,
        "foreign rewind-open must not mutate chat"
    );
    assert!(b.app_mut().conn.rewind.selector.is_none());
    assert_eq!(
        b.app_mut().conn.rewind.pending_open_id.as_deref(),
        Some("rewind-open-local"),
        "foreign rewind-open must not clear local pending open"
    );
}

#[tokio::test]
async fn foreign_rewind_load_and_apply_are_ignored_without_pending_mutation() {
    let mut b = harness().await;
    b.app_mut().editor.set_text("local draft");
    b.app_mut().conn.rewind.pending_load_id = Some("rewind-load-local".into());
    b.app_mut().conn.rewind.pending_apply_message_id = Some("local-message".into());
    b.app_mut().conn.rewind.pending_apply_id = Some("rewind-to-local".into());
    b.app_mut().conn.rewind.pending_apply_editor_baseline = Some("local draft".into());
    b.app_mut().conn.rewind.pending_apply_text = Some("local original".into());
    let notifications_before = b.app_mut().notifications.messages().len();

    b.app_mut().handle_response(
        Some("rewind-load-foreign-1".into()),
        "get_message".into(),
        true,
        Some(serde_json::json!({"role":"user","content":"foreign loaded","id":"foreign"})),
        None,
    );
    b.app_mut().handle_response(
        Some("rewind-to-foreign-1".into()),
        "rewind_to".into(),
        true,
        None,
        None,
    );

    assert_eq!(
        b.app_mut().conn.rewind.pending_load_id.as_deref(),
        Some("rewind-load-local")
    );
    assert_eq!(
        b.app_mut().conn.rewind.pending_apply_message_id.as_deref(),
        Some("local-message")
    );
    assert_eq!(
        b.app_mut().conn.rewind.pending_apply_id.as_deref(),
        Some("rewind-to-local")
    );
    assert_eq!(b.app_mut().editor.text(), "local draft");
    assert_eq!(
        b.app_mut().notifications.messages().len(),
        notifications_before
    );
}

#[tokio::test]
async fn own_resume_still_replaces_and_shows_status() {
    let mut a = harness().await;
    a.app_mut().handle_event(Event::Token {
        token: "SHOULD_BE_CLEARED".into(),
    });
    let resume_id = mint_resume_id(&mut a).await;
    respond(
        a.app_mut(),
        Some(&resume_id),
        "get_messages",
        true,
        legacy_messages("own resumed user", "own resumed assistant"),
    );
    let frame = b_frame(a.app_mut());
    assert!(frame.contains("Session resumed"), "{frame}");
    assert!(frame.contains("own resumed user"), "{frame}");
    assert!(!frame.contains("SHOULD_BE_CLEARED"), "{frame}");
}

#[tokio::test]
async fn foreign_same_family_unknown_id_does_not_hit_legacy_replace() {
    let mut b = harness().await;
    seed_client_b_transcript(b.app_mut());
    let before_frame = b_frame(b.app_mut());

    // No pending on B; foreign minted family id must DROP, not fall through.
    respond(
        b.app_mut(),
        Some("resume-messages-foreign-1"),
        "get_messages",
        true,
        legacy_messages("foreign user", "foreign assistant"),
    );
    respond(
        b.app_mut(),
        Some("rewind-refresh-foreign-2"),
        "get_messages",
        true,
        legacy_messages("foreign rw user", "foreign rw assistant"),
    );
    respond(
        b.app_mut(),
        Some("attach-backfill-foreign-3"),
        "get_messages",
        true,
        legacy_messages("foreign attach user", "foreign attach assistant"),
    );
    // Bare legacy literals from old peers are also non-pending → drop.
    // Use raw_respond so the test helper does not arm pending for the bare id.
    raw_respond(
        b.app_mut(),
        Some("resume-messages"),
        legacy_messages("bare resume user", "bare resume assistant"),
    );

    let frame = b_frame(b.app_mut());
    assert_eq!(
        frame, before_frame,
        "foreign same-family ids must not hit legacy replace:\nbefore:\n{before_frame}\nafter:\n{frame}"
    );
    assert!(!frame.contains("Session resumed"), "{frame}");
    assert!(!frame.contains("foreign user"), "{frame}");
    assert!(!frame.contains("bare resume user"), "{frame}");
}

#[tokio::test]
async fn id_less_busy_connect_snapshot_still_reconciles_on_unrelated_client() {
    let mut b = harness().await;
    b.app_mut().handle_event(Event::Token {
        token: "LIVE_B".into(),
    });
    let mut snapshot = legacy_messages("snapshot question", "snapshot answer");
    if let Some(obj) = snapshot.as_object_mut() {
        obj.insert("snapshot".into(), serde_json::json!(true));
    }
    respond(b.app_mut(), None, "get_messages", true, snapshot);
    let frame = b_frame(b.app_mut());
    assert!(
        frame.contains("snapshot answer") && frame.contains("LIVE_B"),
        "id-less snapshot must still reconcile:\n{frame}"
    );
    assert!(!frame.contains("Session resumed"), "{frame}");
}

#[tokio::test]
async fn foreign_attach_backfill_does_not_reconcile_other_client() {
    let (mut a, mut b) = (harness().await, harness().await);
    seed_client_b_transcript(b.app_mut());
    let before_frame = b_frame(b.app_mut());
    let attach_id = mint_attach_id(&mut a).await;

    respond(
        b.app_mut(),
        Some(&attach_id),
        "get_messages",
        true,
        legacy_messages("a attach user", "a attach assistant"),
    );
    let frame = b_frame(b.app_mut());
    assert_eq!(frame, before_frame, "foreign attach must not mutate B");
    assert!(!frame.contains("a attach user"), "{frame}");
}

#[tokio::test]
async fn own_attach_backfill_still_reconciles_and_preserves_live_tokens() {
    let mut a = harness().await;
    a.app_mut().handle_event(Event::Token {
        token: "LIVE_A".into(),
    });
    let attach_id = mint_attach_id(&mut a).await;
    respond(
        a.app_mut(),
        Some(&attach_id),
        "get_messages",
        true,
        legacy_messages("own attach user", "own attach assistant"),
    );
    let frame = b_frame(a.app_mut());
    assert!(frame.contains("own attach user"), "{frame}");
    assert!(frame.contains("LIVE_A"), "{frame}");
    assert!(!frame.contains("Session resumed"), "{frame}");
}

#[tokio::test]
async fn own_rewind_refresh_still_replaces_with_kind_status() {
    let mut a = harness().await;
    a.app_mut().handle_event(Event::Token {
        token: "PRE_REWIND".into(),
    });
    let refresh_id = mint_rewind_refresh_id(&mut a).await;
    respond(
        a.app_mut(),
        Some(&refresh_id),
        "get_messages",
        true,
        page(&[("m1", "post rewind turn")], None, false),
    );
    let frame = b_frame(a.app_mut());
    assert!(frame.contains("Conversation rewound"), "{frame}");
    assert!(frame.contains("post rewind turn"), "{frame}");
    assert!(!frame.contains("PRE_REWIND"), "{frame}");
    assert!(!frame.contains("Session resumed"), "{frame}");
}

#[tokio::test]
async fn rewind_request_ids_use_fresh_production_tokens_per_request() {
    let mut h = harness().await;

    for kind in ["open", "load", "to"] {
        let first_id = h.app_mut().next_rewind_request_id(kind);
        let second_id = h.app_mut().next_rewind_request_id(kind);
        let prefix = format!("tab0:rewind-{kind}-");
        let first_token = first_id
            .strip_prefix(&prefix)
            .and_then(|rest| rest.rsplit_once('-'))
            .map(|(token, _seq)| token)
            .expect("rewind id includes token and sequence");
        let second_token = second_id
            .strip_prefix(&prefix)
            .and_then(|rest| rest.rsplit_once('-'))
            .map(|(token, _seq)| token)
            .expect("rewind id includes token and sequence");
        assert_ne!(
            first_token, second_token,
            "rewind {kind} ids must include fresh production uniqueness tokens"
        );
        assert!(first_id.starts_with(&prefix), "{first_id}");
        assert!(second_id.starts_with(&prefix), "{second_id}");
    }
}

// --- #1463: minted correlation ids carry a connection namespace -------------
//
// Phase 2 of the multi-session TUI (epic #1467): every correlation id this
// client mints is scoped to its connection, so a broadcast response can never
// match a pending latch on another tab. The master tab is `TabId(0)`; its
// namespace prefix is `tab0:`.

/// Namespace prefix every master-tab minted correlation id must carry (#1463).
const MASTER_NAMESPACE: &str = "tab0:";

#[track_caller]
fn assert_namespaced(id: &str, what: &str) {
    assert!(
        id.starts_with(MASTER_NAMESPACE),
        "{what} must be namespaced to its connection (#1463): \
         expected prefix {MASTER_NAMESPACE:?}, got {id:?}"
    );
}

#[tokio::test]
async fn minted_resume_id_carries_connection_namespace() {
    let mut h = harness().await;
    let id = mint_resume_id(&mut h).await;
    assert_namespaced(&id, "solicited resume get_messages id");
}

#[tokio::test]
async fn minted_rewind_refresh_id_carries_connection_namespace() {
    let mut h = harness().await;
    let id = mint_rewind_refresh_id(&mut h).await;
    assert_namespaced(&id, "post-rewind refresh get_messages id");
}

#[tokio::test]
async fn minted_attach_backfill_id_carries_connection_namespace() {
    let mut h = harness().await;
    let id = mint_attach_id(&mut h).await;
    assert_namespaced(&id, "attach backfill get_messages id");
}

#[tokio::test]
async fn rewind_flow_request_ids_carry_connection_namespace() {
    let mut h = harness().await;
    for kind in ["open", "load", "to"] {
        let id = h.app_mut().next_rewind_request_id(kind);
        assert_namespaced(&id, "rewind flow request id");
    }
}

#[tokio::test]
async fn message_recovery_ids_carry_connection_namespace() {
    let mut h = harness().await;
    // A finished run whose refs delivered no assistant content forces the
    // #1060 fetch-on-miss recovery, minting one request id per ref plus a
    // batch id.
    h.app_mut().handle_event(Event::AgentStart);
    h.app_mut().handle_event(Event::AgentEnd {
        messages: vec![],
        message_refs: vec!["m-1463".into()],
    });
    let _ = h.drain_commands().await;
    let app = h.app_mut();
    assert!(
        !app.conn.pending_message_recovery.is_empty(),
        "precondition: content-less refs mint recovery requests"
    );
    for id in app.conn.pending_message_recovery.keys() {
        assert_namespaced(id, "message-recovery request id");
    }
    assert!(
        !app.conn.message_recovery_batches.is_empty(),
        "precondition: recovery mints a batch id"
    );
    for id in app.conn.message_recovery_batches.keys() {
        assert_namespaced(id, "message-recovery batch id");
    }
}

#[tokio::test]
async fn minted_ids_derive_namespace_from_tab_id() {
    // Guard against a hard-coded "tab0:" literal satisfying every assertion
    // above (#1463 review): a connection re-keyed to tab 1 must mint tab1: ids.
    let mut h = harness().await;
    h.app_mut().test_set_master_tab(1);
    let id = mint_resume_id(&mut h).await;
    assert!(
        id.starts_with("tab1:"),
        "minted id namespaces must derive from the connection's tab id, \
         not a constant prefix (#1463): got {id:?}"
    );
}

#[tokio::test]
async fn startup_request_ids_carry_connection_namespace() {
    // The connect-time literals ("init", "init-subagents") and the attach
    // backfill are minted ids too (#1463 scope).
    let mut h = harness().await;
    h.app_mut().send_startup_requests();
    let commands = h.drain_commands().await;
    let ids: Vec<String> = commands
        .iter()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|cmd| cmd.get("id").and_then(|v| v.as_str()).map(str::to_owned))
        .collect();
    assert_eq!(
        ids.len(),
        3,
        "startup sends get_state + get_subagents + attach backfill: {commands:?}"
    );
    for id in &ids {
        assert_namespaced(id, "startup request id");
    }
    assert!(
        ids.iter().any(|id| id.ends_with(":init")),
        "get_state keeps its init suffix under the namespace: {ids:?}"
    );
    assert!(
        ids.iter().any(|id| id.ends_with(":init-subagents")),
        "get_subagents keeps its init-subagents suffix under the namespace: {ids:?}"
    );
}

#[tokio::test]
async fn stub_recall_ids_carry_connection_namespace() {
    use super::app_paged_history_tests::prime_active_viewport;
    let mut h = harness().await;
    respond(
        h.app_mut(),
        Some("attach-backfill"),
        "get_messages",
        true,
        serde_json::json!({
            "messages": [{
                "id": "stub-1463",
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
    let _ = h.drain_commands().await;
    let app = h.app_mut();
    assert!(
        !app.conn.pending_stub_recall.is_empty(),
        "precondition: a visible stub mints a recall request"
    );
    for id in app.conn.pending_stub_recall.keys() {
        assert_namespaced(id, "stub-recall request id");
    }
}

#[tokio::test]
async fn routed_subagent_feed_ids_carry_connection_namespace() {
    // The inspection-feed "initial" literals ride routed ids on the MASTER
    // connection; broadcast responses to them must be tab-scoped too (#1463).
    let mut h = harness().await;
    h.app_mut()
        .update_subagent_bar(vec![crate::protocol::client::SubagentInfoEvent {
            agent_uuid: Some("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into()),
            display_name: Some("worker-ns".into()),
            agent_id: "worker-ns".into(),
            status: "running".into(),
            last_tool: None,
            last_error: None,
            pid: 1,
            socket_path: None,
            parent_id: None,
            workflow: None,
            read_only: false,
            execution_backend: None,
            environment: None,
        }]);
    h.app_mut()
        .ensure_synced_subagent_feed("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
    let commands = h.drain_commands().await;
    let ids: Vec<String> = commands
        .iter()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|cmd| cmd.get("id").and_then(|v| v.as_str()).map(str::to_owned))
        .filter(|id| id.contains("subagent-"))
        .collect();
    assert!(
        !ids.is_empty(),
        "precondition: an inspection-only feed sends routed requests: {commands:?}"
    );
    for id in &ids {
        assert_namespaced(id, "routed sub-agent feed request id");
    }
}

#[tokio::test]
async fn foreign_namespace_response_does_not_resolve_pending_resume() {
    // The other side of the namespace boundary (#1463 review): prefixing on
    // mint is worthless if matching strips or ignores the prefix.
    let mut h = harness().await;
    let id = mint_resume_id(&mut h).await;
    let foreign = format!("tab1:{}", id.strip_prefix(MASTER_NAMESPACE).unwrap_or(&id));
    respond(
        h.app_mut(),
        Some(&foreign),
        "get_messages",
        true,
        legacy_messages("foreign resumed user", "foreign resumed assistant"),
    );
    assert_eq!(
        h.app_mut().test_pending_resume_messages_id(),
        Some(id.as_str()),
        "a response bearing another tab's namespace must leave this tab's \
         pending resume fetch unresolved (#1463)"
    );
    let frame = b_frame(h.app_mut());
    assert!(
        !frame.contains("foreign resumed user"),
        "a foreign-namespace transcript must not land in this tab:\n{frame}"
    );
}

#[tokio::test]
async fn disconnect_diag_completion_for_another_tab_leaves_this_latch_pending() {
    // The disconnect-diagnosis pending latch is keyed per tab (#1463,
    // accepted phase-2 debt from PR #1470): a completion attributed to some
    // other tab must not clear (or emit through) this tab's latch.
    let mut h = harness().await;
    h.app_mut().conn.disconnect_diag_pending = true;
    h.app_mut().finish_agent_stream_closed(
        crate::shell::connection::TabId(1),
        Some("other tab's exit detail".into()),
    );
    assert!(
        h.app_mut().conn.disconnect_diag_pending,
        "a diagnosis completion keyed to another tab must leave this tab's \
         pending latch set (#1463)"
    );
}
