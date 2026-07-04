//! Unit tests for `uds_cancel.rs` — cancel-slot lifecycle and the Writer-sink
//! `collect_notification` arm (#994 review follow-up).

use super::*;
use crate::infrastructure::tools::subagent_registry::{
    SubagentEntry, SubagentNotification, mark_completion_consumed_by_await, new_registry,
};

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
    let sink = EventSink::Writer(&mut buf);
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
    let sink = EventSink::Writer(&mut buf);
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
    let sink = EventSink::Writer(&mut buf);
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
