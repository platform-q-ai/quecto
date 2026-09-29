//! #2303: a board op's record is appended synchronously from a plain
//! thread, filed under no turn, and a record the log cannot take is
//! dropped with one warning, never a panic.
use std::sync::atomic::Ordering;

use super::EventLogBoardOps;
use crate::application::swarm::ports::BoardOpLog;
use crate::domain::redaction::Redacted;
use crate::domain::swarm::{BoardOpObservation, BoardOpOutcome, BoardRole};
use crate::infrastructure::persistence::audit_log::AuditLog;

fn observation() -> BoardOpObservation {
    BoardOpObservation {
        op: "_status".into(),
        actor_ref: Redacted::from("parent"),
        role: BoardRole::Host,
        run_id: None,
        task_id: None,
        message_id: None,
        outcome: BoardOpOutcome::Ok,
        duration_us: 10,
        lock_wait_us: None,
        busy_wait_us: None,
        busy: None,
        cursor_moved: false,
        result_bytes: 2,
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
    assert_eq!(text, "", "nothing past the cap");
}
