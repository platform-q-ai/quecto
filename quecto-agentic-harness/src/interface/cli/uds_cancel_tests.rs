//! Unit tests for `uds_cancel.rs` — cancel-slot lifecycle and the Writer-sink
//! `collect_notification` arm (#994 review follow-up).

use super::*;
use crate::infrastructure::tools::subagent_registry::{
    SubagentEntry, SubagentNotification, mark_completion_consumed_by_await, new_registry,
};

#[test]
fn cancellation_removes_prompt_at_its_logical_boundary_after_pruning() {
    let prompt = Message::user("cancel me");
    let prompt_id = prompt.id();
    let mut messages = vec![
        Message::user("survivor"),
        prompt,
        Message::assistant("partial output", vec![]),
    ];

    rollback_prompt(&mut messages, prompt_id);

    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].content, "survivor");
}

fn make_notif(seq: u64) -> SequencedSubagentNotification {
    SequencedSubagentNotification::new(
        seq,
        SubagentNotification::Completed {
            agent_id: "worker".to_string(),
            summary: "done".to_string(),
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
// while honouring the auto-await dedupe.

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
        "a manual await already consumed this completion — must dedupe"
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

    // First notification consumes the pending await flag (suppressed);
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

// --- #1047: outbound event lines must never exceed the 1 MiB protocol cap ---
//
// The TUI client drops any event line above 1 MiB (`MAX_FRAME_PAYLOAD_BYTES` in
// quecto-tui). Near a full context window a turn's messages can exceed that,
// so `EventSink::emit` must tail/cap the payload instead of emitting an
// un-receivable line — otherwise the TUI silently loses `turn_end`/`agent_end`
// and the session appears frozen/disconnected.

// Bound under test: `protocol::EVENT_LINE_CAP_BYTES`, whose value is pinned to
// the TUI client's `quecto-tui::infrastructure::client::MAX_FRAME_PAYLOAD_BYTES` (see
// the constant's doc comment in protocol.rs). If the client cap ever changes,
// change the protocol constant — these tests follow it automatically.
use crate::interface::cli::protocol::EVENT_LINE_CAP_BYTES;

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
    let v: serde_json::Value = serde_json::from_str(line.trim_end())
        .unwrap_or_else(|e| panic!("capped line must be valid JSON ({expected_type}): {e}"));
    assert_eq!(v["type"], expected_type, "capped line must keep its type");
    v
}

fn big_turn_end(content: String) -> AgentEvent {
    AgentEvent::TurnEnd {
        message: TurnMessage {
            role: "assistant".to_string(),
            content,
            usage: None,
            stop_reason: None,
            context_tokens: Some(1),
            max_context_tokens: Some(2),
        },
        tool_results: vec![],
    }
}

#[tokio::test]
async fn agent_end_event_line_stays_within_protocol_cap_keeping_recent_messages() {
    // ~2 MiB of run messages — well beyond the cap.
    let big = "x".repeat(256 * 1024);
    let messages: Vec<serde_json::Value> = (0..8)
        .map(|i| serde_json::json!({"role": "assistant", "content": format!("{i}:{big}")}))
        .collect();
    let line = emit_line(&AgentEvent::AgentEnd { messages }).await;

    assert!(
        line.len() <= EVENT_LINE_CAP_BYTES,
        "agent_end event line must be tailed to the protocol cap so the \
         TUI can receive it (#1047); got {} bytes",
        line.len()
    );
    let v = parse_event_line(&line, "agent_end");
    // Tailing must keep the MOST RECENT message, not delete content wholesale:
    // the final answer (message "7:…") is what the user is waiting on.
    let kept = v["messages"].as_array().expect("messages array");
    assert!(!kept.is_empty(), "capped agent_end must retain messages");
    let last = kept.last().unwrap()["content"].as_str().expect("content");
    assert!(
        last.ends_with(&"x".repeat(1024)),
        "capped agent_end must preserve the tail of the most recent message"
    );
    assert!(
        last.starts_with("7:"),
        "capped agent_end must keep the newest message (got prefix {:?})",
        &last[..8.min(last.len())]
    );
}

#[tokio::test]
async fn turn_end_event_line_stays_within_protocol_cap_keeping_content_tail() {
    let original = format!("{}FINAL-ANSWER-TAIL", "y".repeat(2 * 1024 * 1024));
    let line = emit_line(&big_turn_end(original.clone())).await;

    assert!(
        line.len() <= EVENT_LINE_CAP_BYTES,
        "turn_end event line must be capped to the protocol cap so the \
         TUI can receive it (#1047); got {} bytes",
        line.len()
    );
    let v = parse_event_line(&line, "turn_end");
    let content = v["message"]["content"].as_str().expect("content string");
    // Tailing keeps the END of the message — the conclusion of the answer.
    assert!(
        content.ends_with(&original[original.len() - 1024..]),
        "capped turn_end must preserve the tail of the original content"
    );
    assert!(
        content.len() > EVENT_LINE_CAP_BYTES / 2,
        "capped turn_end should keep most of the budget's worth of content, \
         not delete it wholesale; kept {} bytes",
        content.len()
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
async fn turn_end_event_line_just_over_the_cap_is_tailed() {
    // The other side of the boundary: barely over the cap must trigger
    // tailing (a blanket truncate-everything implementation would also pass
    // the far-over tests; this pins the boundary itself).
    let mut event = big_turn_end("z".repeat(1_000_000));
    let base_len = event.to_json_line().len();
    let over_by = (EVENT_LINE_CAP_BYTES - 1) - base_len + 1;
    if let AgentEvent::TurnEnd { message, .. } = &mut event {
        message.content.push_str(&"z".repeat(over_by));
    }
    assert!(event.to_json_line().len() > EVENT_LINE_CAP_BYTES - 1);

    let line = emit_line(&event).await;
    assert!(
        line.len() <= EVENT_LINE_CAP_BYTES,
        "just-over-cap turn_end must be tailed under the cap; got {} bytes",
        line.len()
    );
    parse_event_line(&line, "turn_end");
}
