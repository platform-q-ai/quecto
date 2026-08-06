//! Unit tests for `uds_cancel.rs` — cancel-slot lifecycle and the Writer-sink
//! `collect_notification` arm (#994 review follow-up).

use super::*;
use crate::application::agent_loop::AgentLoopImpl;
use crate::domain::message::{ThinkingBlock, ToolCall};
use crate::infrastructure::line_cap::EVENT_LINE_CAP_BYTES as SHARED_EVENT_LINE_CAP_BYTES;
use crate::infrastructure::tools::subagent_registry::{
    SubagentEntry, SubagentNotification, mark_completion_consumed_by_await, new_registry,
};
use crate::interface::cli::uds_cancel_history::finalize_interrupted_turn;

#[test]
fn cancellation_preserves_interrupted_prompt_for_next_turn_context() {
    let prompt = Message::user("remember ESC_ABORT_123");
    let prompt_id = prompt.id();
    let mut messages = vec![
        Message::user("previous"),
        prompt,
        Message::assistant(
            "partial streamed text",
            vec![tool_call("open-prompt", "grep")],
        ),
    ];

    finalize_interrupted_turn(&mut messages, prompt_id);

    assert_eq!(messages.len(), 4);
    assert_eq!(messages[0].content, "previous");
    assert_eq!(messages[1].content, "remember ESC_ABORT_123");
    assert!(matches!(
        messages[1].role,
        crate::domain::message::Role::User
    ));
    assert_tool_call_aborted(&messages, "open-prompt", "grep");
}

#[test]
fn cancellation_preserves_prompt_at_its_logical_boundary_after_pruning() {
    let prompt = Message::user("cancel me");
    let prompt_id = prompt.id();
    // The prompt is pushed at index 2 — the position a caller would have
    // recorded pre-run.
    let mut messages = vec![
        Message::user("dropped-by-pruning"),
        Message::user("survivor"),
        prompt,
        Message::assistant("partial output", vec![tool_call("shifted-open", "find")]),
    ];
    let recorded_prompt_index = 2;

    // Simulate a mid-run physical drop (#1046 ladder rung 2) shifting the
    // prompt LEFT of its recorded index. A positional rollback that
    // truncates at the recorded index would now RETAIN the prompt
    // (#1073 review: without this shift, positional and id-based rollback
    // were indistinguishable and the test falsified nothing).
    messages.remove(0);
    assert_ne!(
        messages
            .iter()
            .position(|m| m.id() == prompt_id)
            .expect("prompt present"),
        recorded_prompt_index,
        "scenario setup: the drop must move the prompt off its recorded index"
    );

    finalize_interrupted_turn(&mut messages, prompt_id);

    assert_eq!(
        messages.len(),
        4,
        "cancellation must locate the prompt by id even after pruning shifted indices"
    );
    assert_eq!(messages[0].content, "survivor");
    assert_eq!(messages[1].content, "cancel me");
    assert_tool_call_aborted(&messages, "shifted-open", "find");
}

fn tool_call(id: &str, name: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: name.into(),
        arguments: "{}".into(),
    }
}

fn tool_results<'a>(messages: &'a [Message], call_id: &str) -> Vec<&'a Message> {
    messages
        .iter()
        .filter(|message| {
            matches!(message.role, crate::domain::message::Role::Tool)
                && message.tool_call_id.as_deref() == Some(call_id)
        })
        .collect()
}

fn assert_tool_result(messages: &[Message], call_id: &str, content: &str) {
    let results = tool_results(messages, call_id);
    assert_eq!(
        results.len(),
        1,
        "tool call {call_id} must have exactly one result"
    );
    assert_eq!(results[0].content, content);
}

fn assert_tool_call_aborted(messages: &[Message], call_id: &str, tool_name: &str) {
    let results = tool_results(messages, call_id);
    assert_eq!(
        results.len(),
        1,
        "tool call {call_id} must have exactly one aborted result"
    );
    let result = results[0];
    assert_eq!(result.content, "aborted by user");
    assert!(result.is_error);
    assert_eq!(result.tool_name.as_deref(), Some(tool_name));
}

fn assert_no_orphan_tool_pairs(messages: &[Message]) {
    use std::collections::{HashMap, HashSet};

    let mut calls = HashSet::new();
    let mut results: HashMap<&str, usize> = HashMap::new();
    for message in messages {
        if matches!(message.role, crate::domain::message::Role::Assistant) {
            for call in &message.tool_calls {
                calls.insert(call.id.as_str());
            }
        }
        if matches!(message.role, crate::domain::message::Role::Tool) {
            if let Some(id) = message.tool_call_id.as_deref() {
                *results.entry(id).or_insert(0) += 1;
            }
        }
    }

    for call_id in calls {
        assert_eq!(
            results.get(call_id).copied(),
            Some(1),
            "tool call {call_id} must have exactly one result"
        );
    }
}

#[test]
fn soft_cancel_preserves_completed_tool_results() {
    let prompt = Message::user("inspect");
    let prompt_id = prompt.id();
    let mut retained_assistant =
        Message::assistant("partial narration", vec![tool_call("done-1", "read")]);
    retained_assistant.thinking_blocks = vec![ThinkingBlock::Redacted {
        data: "secret-reasoning".into(),
    }];
    let _ = retained_assistant.estimated_tokens();
    let mut messages = vec![
        Message::user("previous"),
        prompt,
        retained_assistant,
        Message::tool("done-1", "completed result 1"),
        Message::assistant("", vec![tool_call("done-2", "ls")]),
        Message::tool("done-2", "completed result 2"),
    ];

    finalize_interrupted_turn(&mut messages, prompt_id);

    assert!(
        messages
            .iter()
            .filter(|message| matches!(message.role, crate::domain::message::Role::Assistant))
            .all(|message| message.content.is_empty())
    );
    assert!(
        messages
            .iter()
            .filter(|message| matches!(message.role, crate::domain::message::Role::Assistant))
            .all(|message| message.thinking_blocks.is_empty())
    );
    assert!(
        messages
            .iter()
            .filter(|message| matches!(message.role, crate::domain::message::Role::Assistant))
            .all(|message| message.estimated_tokens()
                < Message::estimate_tokens("partial narration"))
    );
    assert_tool_result(&messages, "done-1", "completed result 1");
    assert_tool_result(&messages, "done-2", "completed result 2");
    assert_no_orphan_tool_pairs(&messages[1..]);
}

#[test]
fn soft_cancel_records_aborted_results_for_unanswered_tool_calls() {
    let prompt = Message::user("inspect");
    let prompt_id = prompt.id();
    let mut messages = vec![
        Message::user("previous"),
        prompt,
        Message::assistant("", vec![tool_call("done-1", "read")]),
        Message::tool("done-1", "completed result"),
        Message::assistant(
            "",
            vec![tool_call("open-1", "grep"), tool_call("open-2", "find")],
        ),
    ];

    finalize_interrupted_turn(&mut messages, prompt_id);

    assert!(
        messages
            .iter()
            .filter(|message| matches!(message.role, crate::domain::message::Role::Assistant))
            .all(|message| message.content.is_empty())
    );
    assert_tool_result(&messages, "done-1", "completed result");
    assert_tool_call_aborted(&messages, "open-1", "grep");
    assert_tool_call_aborted(&messages, "open-2", "find");
    assert_no_orphan_tool_pairs(&messages[1..]);
}

fn make_notif(seq: u64) -> SequencedSubagentNotification {
    SequencedSubagentNotification::new(
        seq,
        SubagentNotification::Completed {
            agent_id: "worker".to_string(),
        },
    )
}

fn registry_with_worker() -> SubagentRegistry {
    let registry = new_registry();
    registry
        .lock()
        .unwrap()
        .insert("worker".to_string(), SubagentEntry::new("/tmp/x".into(), 0));
    registry
}

// --- collect_notification on a Writer sink (#994 review follow-up) ---
//
// The Broadcast arm is covered in uds_subagent_notify_tests.rs via
// `forward_notification_broadcast`; these pin the Writer arm, which must
// still collect notifications for LLM injection (never silently drop)
// while honouring the passive-note dedupe.

#[test]
fn writer_sink_collects_notification_when_not_awaited() {
    let registry = Some(registry_with_worker());
    let mut buf: Vec<u8> = Vec::new();
    let sink = EventSink::writer(&mut buf);
    let mut notifications = Vec::new();

    collect_notification(make_notif(1), &sink, &registry, &mut notifications);

    assert_eq!(
        notifications.len(),
        1,
        "Writer sink must collect the notification for LLM injection"
    );
    assert!(
        buf.is_empty(),
        "Writer sink must not emit client fan-out events for notifications"
    );
}

#[test]
fn writer_sink_suppresses_notification_when_awaited() {
    let registry = registry_with_worker();
    mark_completion_consumed_by_await(&registry, "worker");
    let registry = Some(registry);
    let mut buf: Vec<u8> = Vec::new();
    let sink = EventSink::writer(&mut buf);
    let mut notifications = Vec::new();

    collect_notification(make_notif(1), &sink, &registry, &mut notifications);

    assert!(
        notifications.is_empty(),
        "a legacy completion consumer already reported this completion — must dedupe"
    );
}

#[test]
fn writer_sink_dedupe_flag_consumed_once() {
    let registry = registry_with_worker();
    mark_completion_consumed_by_await(&registry, "worker");
    let registry = Some(registry);
    let mut buf: Vec<u8> = Vec::new();
    let sink = EventSink::writer(&mut buf);
    let mut notifications = Vec::new();

    // First notification consumes the legacy completion-dedupe flag (suppressed);
    // the second must be collected again.
    collect_notification(make_notif(1), &sink, &registry, &mut notifications);
    collect_notification(make_notif(2), &sink, &registry, &mut notifications);

    assert_eq!(
        notifications.len(),
        1,
        "dedupe flag must suppress exactly one notification"
    );
    assert_eq!(notifications[0].sequence, 2);
}

fn make_handle() -> CancelHandle {
    std::sync::Arc::new(std::sync::Mutex::new(CancelSlot::Idle))
}

#[tokio::test]
async fn cancelled_prompt_keeps_user_message_in_conversation() {
    let mut agent = AgentLoopImpl::new(crate::application::agent_loop::AgentLoopConfig {
        provider: crate::interface::test_support::make_stub_provider(),
        tool_registry: Box::new(crate::infrastructure::tools::registry::ToolRegistryImpl::new()),
        model: "stub".into(),
        max_tokens: 100,
        temperature: 0.0,
        spill_store: None,
        session_key: "cli:test".into(),
        context_collapse_after_tool_calls: u32::MAX,
        max_context_tokens: 190_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
    });
    let mut messages = Vec::new();
    let mut session = AgentSession::new("stub".into(), "cli:test".into());
    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
    drop(cancel_tx);
    let mut notification_rx = None;
    let subagent_registry = None;
    let mut bytes = Vec::new();
    let mut sink = EventSink::writer(&mut bytes);

    let outcome = run_agent_message(PromptRun {
        execution_state: None,
        agent: &mut agent,
        messages: &mut messages,
        conversation_snapshot: None,
        session: &mut session,
        sink: &mut sink,
        message: crate::domain::message::Message::user("keep interrupted prompt"),
        system_prompt: "",
        cancel_rx,
        notification_rx: &mut notification_rx,
        subagent_registry: &subagent_registry,
    })
    .await;

    assert!(matches!(outcome, PromptOutcome::Cancelled));
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].role, crate::domain::message::Role::User);
    assert_eq!(messages[0].content, "keep interrupted prompt");
}

#[test]
fn abort_then_prompt_works() {
    // Simulates the full abort flow: reader fires cancel on the
    // running prompt, then handle_abort is dispatched.
    // After both, the next prompt should NOT be pre-cancelled.
    let handle = make_handle();

    // Arm for the current run.
    let _rx = arm_cancel(&handle).expect("should arm");

    // Reader task fires cancel (kills running prompt).
    fire_cancel(&handle);

    // handle_abort dispatches (should NOT fire again after fix).
    // Before fix: fire_cancel(&handle) was called here too.
    // After fix: handle_abort only emits the ack event.

    // Next prompt arms successfully.
    let result = arm_cancel(&handle);
    assert!(
        result.is_some(),
        "next prompt should arm successfully after abort"
    );
}

#[test]
fn single_fire_allows_next_prompt() {
    // After the fix: only one fire_cancel (reader task).
    // The next arm_cancel should succeed.
    let handle = make_handle();

    // Arm for the current run.
    let _rx = arm_cancel(&handle).expect("should arm");

    // Single fire (reader task only).
    fire_cancel(&handle);
    // Slot is now Idle.

    // Next prompt arms successfully.
    let result = arm_cancel(&handle);
    assert!(result.is_some(), "single fire should allow next arm_cancel");
}

#[test]
fn fire_on_idle_pre_cancels() {
    let handle = make_handle();
    fire_cancel(&handle);
    assert!(
        arm_cancel(&handle).is_none(),
        "Fired slot should pre-cancel"
    );
}

#[test]
fn arm_disarm_cycle() {
    let handle = make_handle();
    let _rx = arm_cancel(&handle).expect("should arm");
    disarm_cancel(&handle);
    // Should be back to Idle.
    let rx2 = arm_cancel(&handle);
    assert!(rx2.is_some(), "should re-arm after disarm");
}

#[test]
fn fire_on_already_fired_is_noop() {
    let handle = make_handle();
    fire_cancel(&handle); // Idle → Fired
    fire_cancel(&handle); // Fired → Fired (noop)
    // Should still pre-cancel next arm.
    assert!(arm_cancel(&handle).is_none());
}

#[test]
fn disarm_on_idle_is_noop() {
    let handle = make_handle();
    disarm_cancel(&handle); // nothing to disarm
    // Should still arm normally.
    let rx = arm_cancel(&handle);
    assert!(rx.is_some());
}

#[test]
fn disarm_on_fired_does_not_clear() {
    let handle = make_handle();
    fire_cancel(&handle); // → Fired
    disarm_cancel(&handle); // Fired is not Armed, so this is a no-op
    // Slot should still be Fired.
    assert!(
        arm_cancel(&handle).is_none(),
        "Fired state should survive disarm"
    );
}

#[test]
fn arm_cancel_returns_receiver() {
    let handle = make_handle();
    let rx = arm_cancel(&handle).expect("should arm");
    // Fire should signal the receiver.
    fire_cancel(&handle);
    // rx should be signalled (sender dropped or sent).
    // In a real scenario we'd poll the receiver.
    drop(rx); // just verify it exists
}

#[test]
fn multiple_arm_disarm_cycles() {
    let handle = make_handle();
    for _ in 0..10 {
        let _rx = arm_cancel(&handle).expect("should arm");
        disarm_cancel(&handle);
    }
    // Final arm should still work.
    assert!(arm_cancel(&handle).is_some());
}

#[test]
fn fire_then_arm_resets_to_idle() {
    let handle = make_handle();
    fire_cancel(&handle); // → Fired
    let result = arm_cancel(&handle); // Fired → Idle, returns None
    assert!(result.is_none());
    // Now slot should be Idle, so next arm succeeds.
    let result2 = arm_cancel(&handle);
    assert!(result2.is_some(), "should arm after Fired was consumed");
}

// --- #1062: the outbound frame cap is a should-never-fire invariant ---
//
// Legitimate events are bounded by construction (#1060/#1061). An oversized
// event therefore signals a defect: reject it whole rather than silently
// tailing the user's content, while keeping the sink usable for later events.

// Bound under test comes directly from the shared line-I/O protocol constant,
// not a local test-only value.
use crate::interface::cli::protocol::{EVENT_LINE_CAP_BYTES, EVENT_LINE_JSON_BUDGET};

#[test]
fn event_cap_is_the_shared_protocol_bound() {
    assert_eq!(EVENT_LINE_CAP_BYTES, SHARED_EVENT_LINE_CAP_BYTES);
    assert_eq!(
        EVENT_LINE_CAP_BYTES,
        quecto_line_io::PROTOCOL_LINE_CAP_BYTES
    );
}

/// Emit `event` through a broadcast sink and return the emitted line
/// (including its trailing newline).
async fn emit_line(event: &AgentEvent) -> String {
    let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(8);
    let mut sink = EventSink::Broadcast(tx);
    sink.emit(event).await;
    rx.recv().await.expect("event line should be emitted")
}

/// Parse an emitted line as JSON and assert its event `type`.
fn parse_event_line(line: &str, expected_type: &str) -> serde_json::Value {
    let v: serde_json::Value =
        serde_json::from_str(line.trim_end()).expect("capped line must be valid JSON");
    assert_eq!(v["type"], expected_type, "capped line must keep its type");
    v
}

fn big_turn_end(content: String) -> AgentEvent {
    AgentEvent::TurnEnd {
        message: TurnMessage {
            role: "assistant".to_string(),
            content,
            message_refs: vec![],
            usage: None,
            stop_reason: None,
            context_tokens: Some(1),
            max_context_tokens: Some(2),
            content_length: None,
        },
        tool_results: vec![],
    }
}

#[tokio::test]
async fn oversized_event_is_rejected_whole_and_a_later_event_is_emitted() {
    let oversized = big_turn_end("y".repeat(EVENT_LINE_CAP_BYTES + 1024));
    let later = big_turn_end("later event survives".to_string());
    let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(8);
    let mut sink = EventSink::Broadcast(tx);

    sink.emit(&oversized).await;
    sink.emit(&later).await;

    let line = rx
        .recv()
        .await
        .expect("later in-bound event should be emitted");
    assert_eq!(
        line,
        format!("{}\n", later.to_json_line()),
        "an over-cap event must be rejected whole, without reshaping its payload"
    );
    parse_event_line(&line, "turn_end");
    assert!(
        matches!(
            rx.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
        ),
        "the rejected oversized event must not be delivered before or after the later event"
    );
}

#[tokio::test]
async fn turn_end_event_line_under_the_cap_is_emitted_unmodified() {
    // A payload that serializes just under the cap must pass through
    // byte-for-byte — capping must never touch in-budget events.
    let event = big_turn_end("z".repeat(1_000_000));
    let uncapped = event.to_json_line();
    assert!(
        uncapped.len() < EVENT_LINE_CAP_BYTES - 1,
        "precondition: serialized event is under the cap"
    );

    let line = emit_line(&event).await;
    assert_eq!(
        line,
        format!("{uncapped}\n"),
        "an under-cap turn_end must be emitted unmodified"
    );
}

#[tokio::test]
async fn agent_end_event_line_under_the_cap_is_emitted_unmodified() {
    let event = AgentEvent::AgentEnd {
        messages: vec![serde_json::json!({
            "role": "assistant",
            "content": "w".repeat(1_000_000),
        })],
        message_refs: vec![],
    };
    let uncapped = event.to_json_line();
    assert!(
        uncapped.len() < EVENT_LINE_CAP_BYTES - 1,
        "precondition: serialized event is under the cap"
    );

    let line = emit_line(&event).await;
    assert_eq!(
        line,
        format!("{uncapped}\n"),
        "an under-cap agent_end must be emitted unmodified"
    );
}

#[tokio::test]
async fn event_at_the_json_budget_is_emitted_unmodified() {
    let mut event = big_turn_end(String::new());
    let base_len = event.to_json_line().len();
    if let AgentEvent::TurnEnd { message, .. } = &mut event {
        message
            .content
            .push_str(&"x".repeat(EVENT_LINE_JSON_BUDGET - base_len));
    }
    assert_eq!(event.to_json_line().len(), EVENT_LINE_JSON_BUDGET);

    let line = emit_line(&event).await;
    assert_eq!(
        line,
        format!("{}\n", event.to_json_line()),
        "the exact JSON budget remains valid after adding the wire newline"
    );
}

#[tokio::test]
async fn event_just_over_the_cap_is_rejected_without_partial_delivery() {
    let mut event = big_turn_end("z".repeat(1_000_000));
    let base_len = event.to_json_line().len();
    let over_by = (EVENT_LINE_CAP_BYTES - 1) - base_len + 1;
    if let AgentEvent::TurnEnd { message, .. } = &mut event {
        message.content.push_str(&"z".repeat(over_by));
    }
    assert!(event.to_json_line().len() > EVENT_LINE_CAP_BYTES - 1);

    let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(8);
    let mut sink = EventSink::Broadcast(tx);
    sink.emit(&event).await;
    assert!(
        matches!(
            rx.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
        ),
        "a boundary-crossing event must be rejected rather than tailed into a partial event"
    );
}

// ── Mid-turn ledger advance hints (child-progress freeze fix) ────────────────
//
// `publish_turn_progress` advances the conversation snapshot on each inner
// `TurnCompleted`, and must EMIT the resulting `ledger_advanced` hints: the
// TUI feed re-syncs only on that event, so a silent advance freezes a running
// agent's transcript until the whole prompt finishes (#1283 regression).

fn mid_turn_snapshot() -> crate::interface::cli::uds_multi::ConversationSnapshot {
    std::sync::Arc::new(tokio::sync::RwLock::new(
        crate::interface::cli::uds_snapshots::ConversationSnapshotData::default(),
    ))
}

fn turn_completed(messages: Vec<Message>) -> AgentProgressEvent {
    AgentProgressEvent::TurnCompleted {
        messages: messages.into(),
    }
}

fn ledger_advanced_lines(raw: &[u8]) -> Vec<serde_json::Value> {
    String::from_utf8_lossy(raw)
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| v["type"] == "ledger_advanced")
        .collect()
}

#[tokio::test]
async fn mid_turn_completed_turn_emits_ledger_advanced_hint() {
    let snapshot = mid_turn_snapshot();
    let mut buf: Vec<u8> = Vec::new();
    let mut sink = EventSink::writer(&mut buf);

    let event = turn_completed(vec![
        Message::user("question"),
        Message::assistant("inner-turn answer", vec![]),
    ]);
    publish_turn_progress(&event, Some(&snapshot), &mut sink).await;

    let hints = ledger_advanced_lines(&buf);
    assert!(
        !hints.is_empty(),
        "a changed mid-turn publish must emit ledger_advanced, got: {}",
        String::from_utf8_lossy(&buf)
    );
    assert!(hints.iter().all(|h| h["rev"].as_u64().is_some()));
}

#[tokio::test]
async fn unchanged_mid_turn_publish_emits_no_hint() {
    let snapshot = mid_turn_snapshot();
    let messages = vec![
        Message::user("question"),
        Message::assistant("answer", vec![]),
    ];
    let event = turn_completed(messages);

    let mut first: Vec<u8> = Vec::new();
    let mut sink = EventSink::writer(&mut first);
    publish_turn_progress(&event, Some(&snapshot), &mut sink).await;
    assert!(!ledger_advanced_lines(&first).is_empty());

    // Re-publishing the identical turn advances nothing — and must not spam.
    let mut second: Vec<u8> = Vec::new();
    let mut sink = EventSink::writer(&mut second);
    publish_turn_progress(&event, Some(&snapshot), &mut sink).await;
    assert!(
        ledger_advanced_lines(&second).is_empty(),
        "an unchanged publish must not emit ledger_advanced, got: {}",
        String::from_utf8_lossy(&second)
    );
}

#[tokio::test]
async fn non_turn_events_do_not_touch_the_ledger() {
    let snapshot = mid_turn_snapshot();
    let mut buf: Vec<u8> = Vec::new();
    let mut sink = EventSink::writer(&mut buf);

    publish_turn_progress(
        &AgentProgressEvent::Token("tok".into()),
        Some(&snapshot),
        &mut sink,
    )
    .await;

    assert!(buf.is_empty(), "tokens must not publish or emit");
    assert_eq!(snapshot.read().await.rev, 0);
}
