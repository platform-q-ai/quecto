//! Tests for the connect-time conversation snapshot path (#828).
//!
//! A busy sub-agent must serve its prior conversation immediately on connect.
//! The snapshot is a SEPARATE `Arc<RwLock<Vec<Message>>>` from the dispatch
//! loop's `&mut messages`, so a newly-connected client can be served the
//! pre-turn history by the accept loop even while the dispatch loop holds
//! `messages` mutably for the whole turn (`agent.process(messages)`).

use crate::domain::message::Message;
use crate::interface::cli::protocol::SessionState;
use crate::interface::cli::uds_multi::{
    BusyFlag, BusyGuard, ConversationSnapshot, build_get_messages_line, build_get_state_line,
};

/// The connect-time line is a `get_messages`-shaped success Response carrying the
/// prior conversation, byte-for-byte consumable by the TUI's existing
/// `route_subagent_event` get_messages handler.
#[test]
fn build_get_messages_line_serializes_prior_history() {
    let messages = vec![
        Message::user("prior question"),
        Message::assistant("prior answer", vec![]),
    ];
    let line = build_get_messages_line(&messages);
    assert!(
        line.ends_with('\n'),
        "line must be newline-terminated: {line}"
    );

    let v: serde_json::Value = serde_json::from_str(line.trim()).expect("valid JSON line");
    assert_eq!(v["type"], "response");
    assert_eq!(v["command"], "get_messages");
    assert_eq!(v["success"], true);
    let msgs = v["data"]["messages"]
        .as_array()
        .expect("data.messages array");
    assert_eq!(msgs.len(), 2, "both prior messages present: {line}");
    assert_eq!(msgs[0]["role"], "user");
    assert!(line.contains("prior question"), "got: {line}");
    assert!(line.contains("prior answer"), "got: {line}");
}

/// The snapshot is independent of the dispatch loop's exclusive `&mut messages`
/// borrow: while a simulated turn holds `messages` mutably for its whole
/// duration, a concurrent reader (the accept loop) can still read the snapshot
/// and obtain the pre-turn history — i.e. a BUSY child serves prior history at
/// once rather than mid-sentence-only.
#[tokio::test]
async fn snapshot_readable_while_turn_holds_messages_mut() {
    let snapshot: ConversationSnapshot = std::sync::Arc::new(tokio::sync::RwLock::new(vec![
        Message::user("q1"),
        Message::assistant("a1", vec![]),
    ]));

    // Own a separate `messages` buffer mutably for the whole "turn", mirroring
    // `agent.process(messages)` holding `&mut messages` across the turn.
    let mut messages = vec![Message::user("q1"), Message::assistant("a1", vec![])];
    let (started_tx, started_rx) = tokio::sync::oneshot::channel::<()>();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
    let turn = tokio::spawn(async move {
        let _busy: &mut Vec<Message> = &mut messages;
        started_tx.send(()).unwrap();
        // Hold the mutable borrow until released — the turn is mid-flight.
        release_rx.await.unwrap();
        messages.push(Message::user("q2"));
    });

    started_rx.await.unwrap();

    // Mid-turn: the accept-loop read path still serves prior history.
    let line = {
        let snap = snapshot.read().await;
        build_get_messages_line(&snap)
    };
    assert!(line.contains("q1"), "prior history served mid-turn: {line}");
    assert!(line.contains("a1"), "prior history served mid-turn: {line}");

    release_tx.send(()).unwrap();
    turn.await.unwrap();
}

/// `BusyGuard` marks the agent mid-turn for the accept loop's connect-time
/// gating: it sets the shared busy flag on construction and clears it on drop
/// (covering normal completion, early return, and panic via RAII) so the
/// unsolicited connect-time snapshot is pushed only while a turn is in flight.
#[test]
fn busy_guard_sets_flag_for_its_scope_and_clears_on_drop() {
    use std::sync::atomic::Ordering;
    let flag: BusyFlag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    assert!(!flag.load(Ordering::SeqCst), "starts idle");
    {
        let _guard = BusyGuard::new(&flag);
        assert!(flag.load(Ordering::SeqCst), "busy for the turn's scope");
    }
    assert!(!flag.load(Ordering::SeqCst), "cleared on drop (turn over)");
}

#[test]
fn build_get_state_line_serializes_status_snapshot() {
    let state = SessionState {
        model: "mock-model".into(),
        is_streaming: true,
        session_key: "cli:test".into(),
        message_count: 2,
        pending_message_count: 1,
        max_context_tokens: 1234,
        workflow: None,
    };

    let line = build_get_state_line(&state);
    assert!(
        line.ends_with('\n'),
        "line must be newline-terminated: {line}"
    );

    let v: serde_json::Value = serde_json::from_str(line.trim()).expect("valid JSON line");
    assert_eq!(v["type"], "response");
    assert_eq!(v["command"], "get_state");
    assert_eq!(v["success"], true);
    assert_eq!(v["data"]["isStreaming"], true);
    assert_eq!(v["data"]["messageCount"], 2);
    assert_eq!(v["data"]["pendingMessageCount"], 1);
}
