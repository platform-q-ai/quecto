//! #2303: a board op's record is appended synchronously from a plain
//! thread, filed under no turn, and a record the log cannot take is
//! dropped with one warning, never a panic.
use std::sync::atomic::Ordering;

use super::EventLogBoardOps;
use crate::application::swarm::ports::BoardOpLog;
use crate::domain::redaction::Redacted;
use crate::domain::swarm::{BoardOpDetail, BoardOpObservation, BoardOpOutcome, BoardRole};
use crate::infrastructure::persistence::audit_log::AuditLog;

fn observation() -> BoardOpObservation {
    BoardOpObservation {
        op: "_status".into(),
        actor_ref: Redacted::from("parent"),
        role: Some(BoardRole::Host),
        run_id: None,
        task_id: None,
        message_id: None,
        outcome: BoardOpOutcome::Ok,
        duration_us: 10,
        lock_wait_us: None,
        busy_wait_us: None,
        busy: None,
        cursor_moved: None,
        result_bytes: 2,
        decision: None,
        detail: BoardOpDetail::NONE,
    }
}

#[test]
fn a_record_is_one_line_under_no_turn_from_a_plain_thread() {
    let base = tempfile::tempdir().unwrap();
    let log = AuditLog::open_sync(base.path(), "cli:ops").unwrap();
    let ops = EventLogBoardOps::new(log.crash_line().unwrap());
    std::thread::spawn(move || ops.record(observation()))
        .join()
        .expect("no runtime is needed and nothing panics");
    let text = std::fs::read_to_string(AuditLog::file_path(base.path(), "cli:ops")).unwrap();
    let lines: Vec<serde_json::Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 1, "{text}");
    assert_eq!(lines[0]["event"], "swarm_op");
    assert_eq!(lines[0]["turn"], serde_json::Value::Null, "{text}");
}

#[test]
fn a_record_past_the_cap_is_dropped_with_one_warning() {
    let base = tempfile::tempdir().unwrap();
    let log = AuditLog::open_sync(base.path(), "cli:full")
        .unwrap()
        .with_cap(64);
    let ops = EventLogBoardOps::new(log.crash_line().unwrap());
    ops.record(observation());
    assert!(ops.warned.load(Ordering::Acquire), "the drop was reported");
    ops.record(observation());
    let text = std::fs::read_to_string(AuditLog::file_path(base.path(), "cli:full")).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 1, "only the log's own cap record: {text}");
    assert!(lines[0].contains(r#""event":"log_capped""#), "{text}");
}

/// Every line of the log at `base`/`key`, parsed.
fn parsed(base: &std::path::Path, key: &str) -> Vec<serde_json::Value> {
    std::fs::read_to_string(AuditLog::file_path(base, key))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// #2303 swarm review: a record waits for the log's write gate at most its
/// bound. Held by another writer for 2 s, the record is dropped and
/// counted, not waited on; the next record written notes the drop first,
/// in the same write, and the count starts again.
#[test]
fn a_record_gives_up_on_a_held_gate_and_the_next_notes_the_drop() {
    let base = tempfile::tempdir().unwrap();
    let log = AuditLog::open_sync(base.path(), "cli:held").unwrap();
    let ops = EventLogBoardOps::new(log.crash_line().unwrap())
        .with_gate_wait(std::time::Duration::from_millis(50));
    let holder = log.hold_gate_for(std::time::Duration::from_secs(2));
    let started = std::time::Instant::now();
    ops.record(observation());
    let waited = started.elapsed();
    assert!(waited < std::time::Duration::from_millis(200), "{waited:?}");
    assert_eq!(
        ops.dropped.load(Ordering::Acquire),
        1,
        "the drop is counted"
    );
    assert!(ops.warned.load(Ordering::Acquire), "the drop was reported");
    holder.join().unwrap();
    assert!(parsed(base.path(), "cli:held").is_empty());
    ops.record(observation());
    let lines = parsed(base.path(), "cli:held");
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert_eq!(lines[0]["event"], "swarm_ops_dropped");
    assert_eq!(lines[0]["dropped"], 1);
    assert_eq!(lines[0]["turn"], serde_json::Value::Null);
    assert_eq!(lines[1]["event"], "swarm_op");
    assert_eq!(ops.dropped.load(Ordering::Acquire), 0, "the count restarts");
    ops.record(observation());
    assert_eq!(parsed(base.path(), "cli:held").len(), 3, "no second note");
}

/// Drops accumulate while the gate stays held: one note carries them all.
#[test]
fn drops_accumulate_into_one_note() {
    let base = tempfile::tempdir().unwrap();
    let log = AuditLog::open_sync(base.path(), "cli:many").unwrap();
    let ops = EventLogBoardOps::new(log.crash_line().unwrap())
        .with_gate_wait(std::time::Duration::from_millis(5));
    let holder = log.hold_gate_for(std::time::Duration::from_millis(500));
    for _ in 0..3 {
        ops.record(observation());
    }
    holder.join().unwrap();
    ops.record(observation());
    let lines = parsed(base.path(), "cli:many");
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert_eq!(lines[0]["event"], "swarm_ops_dropped");
    assert_eq!(lines[0]["dropped"], 3);
    assert_eq!(lines[1]["event"], "swarm_op");
}
