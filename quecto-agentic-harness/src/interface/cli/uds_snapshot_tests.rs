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
use crate::interface::cli::uds_snapshots::ConversationSnapshotData;

/// #1060 review 1a: the id-addressable ledger keeps a ref resolvable after the
/// live conversation drops or collapses the referenced message.
#[test]
fn ledger_resolves_refs_after_prune_and_collapse() {
    let a = Message::assistant("full answer A", vec![]);
    let b = Message::assistant("full answer B", vec![]);
    let (a_id, b_id) = (a.id().to_string(), b.id().to_string());

    let mut snap = ConversationSnapshotData::default();
    snap.publish(&[a.clone(), b.clone()]);
    assert!(snap.resolve(&a_id).is_some() && snap.resolve(&b_id).is_some());

    // The ladder DROPS A from the live conversation (publish without it).
    snap.publish(std::slice::from_ref(&b));
    assert!(
        snap.resolve(&a_id).is_some(),
        "a dropped ref must still resolve via the ledger"
    );

    // The ladder COLLAPSES B in place (same id, stub content). publish must not
    // clobber the full copy already in the ledger.
    let mut b_stub = b.clone();
    b_stub.content = "recall(spilled)".to_string();
    snap.publish(&[b_stub.clone()]);
    assert_eq!(
        snap.resolve(&b_id).map(|m| m.content.as_str()),
        Some("full answer B"),
        "the ledger's full copy must win over a collapsed live stub"
    );

    // record_full overwrites with an authoritative full copy (un-demoted).
    let mut snap2 = ConversationSnapshotData::default();
    snap2.publish(std::slice::from_ref(&b_stub));
    snap2.record_full(std::slice::from_ref(&b));
    assert_eq!(
        snap2.resolve(&b_id).map(|m| m.content.as_str()),
        Some("full answer B")
    );
}

/// #1060 review r4 finding 2: the ledger is byte-bounded and evicts oldest-first,
/// so a weeks-long session cannot grow it without limit. The oldest refs stop
/// resolving once the budget is exceeded; the most recent still resolve.
#[test]
fn ledger_is_byte_bounded_and_evicts_oldest() {
    // Each message ~6 MiB of content; the 16 MiB budget holds ~2. Recording 4
    // in order must evict the two oldest.
    let big = || Message::assistant("X".repeat(6 * 1024 * 1024), vec![]);
    let msgs: Vec<Message> = (0..4).map(|_| big()).collect();
    let ids: Vec<String> = msgs.iter().map(|m| m.id().to_string()).collect();

    let mut snap = ConversationSnapshotData::default();
    snap.record_full(&msgs);

    assert!(
        snap.resolve(&ids[0]).is_none() && snap.resolve(&ids[1]).is_none(),
        "the oldest refs must be evicted once the ledger byte budget is exceeded"
    );
    assert!(
        snap.resolve(&ids[3]).is_some(),
        "the most recent ref must still resolve"
    );
}

/// #1060 review r4 finding 2 (follow-up): the ledger must ALSO cap entry count,
/// so a flood of tiny/empty/tool-metadata messages — which add little content
/// but real per-entry cost (id copies + struct clones) — cannot grow it without
/// bound even though the byte budget is far from full.
#[test]
fn ledger_is_entry_bounded_for_tiny_messages() {
    use crate::interface::cli::uds_snapshots::LEDGER_MAX_ENTRIES;
    let mut snap = ConversationSnapshotData::default();
    // Zero-content messages, more than the entry cap.
    let msgs: Vec<Message> = (0..LEDGER_MAX_ENTRIES + 100)
        .map(|_| Message::assistant("", vec![]))
        .collect();
    let ids: Vec<String> = msgs.iter().map(|m| m.id().to_string()).collect();
    snap.record_full(&msgs);

    assert!(
        snap.resolve(&ids[0]).is_none(),
        "the oldest tiny messages must be evicted by the entry-count cap"
    );
    assert!(
        snap.resolve(ids.last().unwrap()).is_some(),
        "the most recent message must still resolve"
    );
}

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

/// The connect-time snapshot is tagged `snapshot: true` so a caller can tell the
/// data may lag the in-flight turn — unlike a live dispatch-loop reply (#842).
#[test]
fn build_get_messages_line_marks_snapshot() {
    let line = build_get_messages_line(&[Message::user("q")]);
    let v: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(
        v["data"]["snapshot"], true,
        "snapshot marker present: {line}"
    );
}

/// A history whose serialized form would exceed the parent's per-line read cap is
/// tailed so the call yields a useful (trimmed) answer instead of erroring (#842).
#[test]
fn build_get_messages_line_trims_oversized_history() {
    // Each message ~10 KiB; 200 of them (~2 MiB) exceeds the 1 MiB line cap.
    let big = "x".repeat(10 * 1024);
    let messages: Vec<Message> = (0..200)
        .map(|i| Message::assistant(format!("{i}-{big}"), vec![]))
        .collect();
    let line = build_get_messages_line(&messages);
    assert!(
        line.len() <= 1024 * 1024,
        "line must fit under the 1 MiB cap, got {} bytes",
        line.len()
    );
    let v: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(v["data"]["trimmed"], true, "trimmed marker present");
    let msgs = v["data"]["messages"].as_array().unwrap();
    assert!(!msgs.is_empty(), "keeps the most recent messages");
    // The newest message (index 199) must be retained (tail, not head).
    let kept_last = msgs.last().unwrap()["content"].as_str().unwrap();
    assert!(
        kept_last.starts_with("199-"),
        "newest message kept: {kept_last}"
    );
}

/// A single message that alone exceeds the budget cannot be returned under the
/// parent's read cap, so it is dropped — the call yields an empty `trimmed`
/// snapshot rather than erroring or panicking (#842).
#[test]
fn build_get_messages_line_drops_single_oversized_message() {
    // One ~2 MiB message exceeds the 1 MiB line cap on its own.
    let huge = "x".repeat(2 * 1024 * 1024);
    let line = build_get_messages_line(&[Message::assistant(huge, vec![])]);
    assert!(
        line.len() <= 1024 * 1024,
        "line must fit under the 1 MiB cap, got {} bytes",
        line.len()
    );
    let v: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(v["data"]["trimmed"], true, "trimmed marker present");
    let msgs = v["data"]["messages"].as_array().unwrap();
    assert!(
        msgs.is_empty(),
        "an oversized lone message is dropped: {msgs:?}"
    );
}

/// The snapshot is independent of the dispatch loop's exclusive `&mut messages`
/// borrow: while a simulated turn holds `messages` mutably for its whole
/// duration, a concurrent reader (the accept loop) can still read the snapshot
/// and obtain the pre-turn history — i.e. a BUSY child serves prior history at
/// once rather than mid-sentence-only.
#[tokio::test]
async fn snapshot_readable_while_turn_holds_messages_mut() {
    let snapshot: ConversationSnapshot = std::sync::Arc::new(tokio::sync::RwLock::new(
        crate::interface::cli::uds_snapshots::ConversationSnapshotData::from_messages(vec![
            Message::user("q1"),
            Message::assistant("a1", vec![]),
        ]),
    ));

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
        build_get_messages_line(&snap.messages)
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
        effort: None,
        effort_levels: Vec::new(),
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

// ─── get_subagents connect-time snapshot (#874) ───────────────────────────────
//
// A busy child must serve its current registry view immediately on connect,
// mirroring the #842 busy-serve path established for get_messages/get_state.
// The `SubagentRegistry` is an `Arc<Mutex<…>>` independent of the dispatch
// loop's exclusive `&mut messages` borrow, so `get_subagents` can be served
// from the registry off the dispatch loop while a turn is in flight.

/// The connect-time `get_subagents` line is a success Response carrying the
/// child's current subagent list, byte-for-byte consumable by the parent's
/// id-correlated reader (which accepts the id-less snapshot for a
/// `get_subagents` request, #874).
#[test]
fn build_get_subagents_line_serializes_registry_view() {
    use crate::infrastructure::tools::subagent_registry::{
        SubagentEntry, SubagentRegistry, SubagentStatus,
    };
    use crate::interface::cli::uds_multi::build_get_subagents_line;

    let mut entry = SubagentEntry::new("/tmp/gc.sock".into(), 4321);
    entry.status = SubagentStatus::Running;
    entry.last_tool = Some("bash".into());
    entry.parent_id = Some("child".into());
    let registry: SubagentRegistry = std::sync::Arc::new(std::sync::Mutex::new(
        std::collections::HashMap::from([("grandchild-worker".to_string(), entry)]),
    ));

    let line = build_get_subagents_line(&Some(registry));
    assert!(
        line.ends_with('\n'),
        "line must be newline-terminated: {line}"
    );

    let v: serde_json::Value = serde_json::from_str(line.trim()).expect("valid JSON line");
    assert_eq!(v["type"], "response");
    assert_eq!(v["command"], "get_subagents");
    assert_eq!(v["success"], true);
    let agents = v["data"]["subagents"]
        .as_array()
        .expect("data.subagents array");
    assert_eq!(agents.len(), 1, "one registered subagent: {line}");
    assert_eq!(agents[0]["agentId"], "grandchild-worker");
    assert_eq!(agents[0]["status"], "running");
    assert_eq!(agents[0]["pid"], 4321);
}

/// The connect-time `get_subagents` snapshot is tagged `snapshot: true` so a
/// caller can tell the data may lag the in-flight turn — consistent with the
/// #842 snapshot markers on get_messages/get_state.
#[test]
fn build_get_subagents_line_marks_snapshot() {
    use crate::interface::cli::uds_multi::build_get_subagents_line;

    let line = build_get_subagents_line(&None);
    let v: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(
        v["data"]["snapshot"], true,
        "snapshot marker present: {line}"
    );
    let agents = v["data"]["subagents"].as_array().unwrap();
    assert!(agents.is_empty(), "no registry => empty subagents list");
}

/// A `get_subagents` snapshot for a `None` registry yields an empty subagents
/// list (matching `build_subagent_info_list`'s contract), not an error.
#[test]
fn build_get_subagents_line_empty_when_no_registry() {
    use crate::interface::cli::uds_multi::build_get_subagents_line;

    let line = build_get_subagents_line(&None);
    let v: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(v["success"], true);
    assert_eq!(
        v["data"]["subagents"]
            .as_array()
            .map(std::vec::Vec::len)
            .unwrap_or(999),
        0
    );
}

// ─── #914: busy get_state reflects LIVE workflow progress mid-turn ─────────────
#[test]
fn busy_get_state_reflects_live_workflow_progress_mid_turn() {
    use crate::domain::workflow::{
        WorkflowConfig, WorkflowEngine, WorkflowTemplate, WorkflowTemplateStep,
    };
    use crate::interface::cli::uds_snapshots::{
        build_get_state_line_live, build_get_state_line_with_streaming,
    };
    use std::sync::{Arc, Mutex};

    let step = |k: &str| WorkflowTemplateStep {
        key: k.into(),
        label: k.to_uppercase(),
        phase: "p".into(),
        guidance: None,
    };
    let config = WorkflowConfig {
        auto_continue: true,
        completion_nudge: false,
        selector_prompt: None,
        templates: vec![WorkflowTemplate {
            id: "t".into(),
            label: "T".into(),
            description: "d".into(),
            when_to_use: None,
            steps: vec![step("a"), step("b"), step("c")],
            guards: vec![],
        }],
    };
    let mut engine = WorkflowEngine::new(config, false).unwrap();
    engine.select_template("t", None).unwrap();

    // Frozen snapshot captured at turn boundary (0/3), with automation flags.
    let mut frozen_wf = serde_json::to_value(engine.snapshot(true)).unwrap();
    frozen_wf["automation"] = serde_json::json!({"autoContinue": true, "completionNudge": false});
    let state = SessionState {
        model: "m".into(),
        is_streaming: true,
        session_key: "k".into(),
        message_count: 1,
        pending_message_count: 0,
        max_context_tokens: 1,
        effort: None,
        effort_levels: Vec::new(),
        workflow: Some(frozen_wf),
    };

    // Steps get checked off MID-TURN via the workflow tool — engine now at 2/3.
    engine.check(1).unwrap();
    engine.check(2).unwrap();
    let handle = Arc::new(Mutex::new(engine));

    // The frozen-snapshot builder is stale (still 0/3) — this is the bug.
    let frozen_v: serde_json::Value =
        serde_json::from_str(build_get_state_line_with_streaming(&state, true).trim()).unwrap();
    assert_eq!(
        frozen_v["data"]["workflow"]["progress"]["done"], 0,
        "frozen snapshot reports pre-turn 0/3 (the stale path #914 fixes)"
    );

    // #914 fix: the live builder reports the engine's current 2/3.
    let live_v: serde_json::Value =
        serde_json::from_str(build_get_state_line_live(&state, &Some(handle), true).trim())
            .unwrap();
    assert_eq!(
        live_v["data"]["workflow"]["progress"]["done"], 2,
        "live get_state must reflect mid-turn progress (2/3), not the frozen snapshot"
    );
    assert_eq!(live_v["data"]["workflow"]["progress"]["total"], 3);
    // Automation flags are preserved from the frozen snapshot.
    assert_eq!(
        live_v["data"]["workflow"]["automation"]["autoContinue"],
        true
    );
}
