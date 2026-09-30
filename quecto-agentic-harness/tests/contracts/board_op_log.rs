//! `BoardOpLog` on the event-log adapter (#2303): a record is appended
//! synchronously from a plain thread (no async runtime), as one `swarm_op`
//! line filed under no turn; a record the log cannot take is dropped
//! without a panic, the first such record capping the log.
use quecto::application::swarm::ports::BoardOpLog;
use quecto::domain::redaction::Redacted;
use quecto::domain::swarm::{
    ArgumentFaults, BoardOpDetail, BoardOpObservation, BoardOpOutcome, BoardRole, RefusalKind,
};
use quecto::infrastructure::persistence::audit_log::AuditLog;
use quecto::infrastructure::persistence::board_op_log::EventLogBoardOps;

fn observation() -> BoardOpObservation {
    BoardOpObservation {
        op: "_snapshot".into(),
        actor_ref: Redacted::from("worker-1"),
        role: Some(BoardRole::Host),
        run_id: None,
        task_id: None,
        message_id: None,
        outcome: BoardOpOutcome::Refused {
            kind: RefusalKind::NotMember,
            committed: false,
        },
        duration_us: 5,
        lock_wait_us: Some(1),
        busy_wait_us: Some(0),
        busy: Some(false),
        commit_us: Some(1),
        cursor_moved: None,
        result_bytes: 0,
        decision: None,
        detail: BoardOpDetail::NONE,
        arguments: Box::new(ArgumentFaults::NONE),
    }
}

#[test]
fn a_record_is_appended_from_a_plain_thread() {
    let base = tempfile::tempdir().unwrap();
    let log = AuditLog::open_sync(base.path(), "cli:contract").unwrap();
    let ops = EventLogBoardOps::new(log.crash_line().unwrap());
    std::thread::spawn(move || {
        ops.record(observation());
        ops.record(observation());
    })
    .join()
    .unwrap();
    let text = std::fs::read_to_string(AuditLog::file_path(base.path(), "cli:contract")).unwrap();
    let lines: Vec<serde_json::Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 2, "{text}");
    for line in lines {
        assert_eq!(line["event"], "swarm_op");
        assert_eq!(line["kind"], "not_member");
        assert_eq!(line["turn"], serde_json::Value::Null);
        assert_eq!(line["session"], "cli:contract");
    }
}

#[test]
fn a_record_the_log_cannot_take_is_dropped_quietly() {
    let base = tempfile::tempdir().unwrap();
    let log = AuditLog::open_sync(base.path(), "cli:full")
        .unwrap()
        .with_cap(16);
    let ops = EventLogBoardOps::new(log.crash_line().unwrap());
    ops.record(observation());
    ops.record(observation());
    let text = std::fs::read_to_string(AuditLog::file_path(base.path(), "cli:full")).unwrap();
    // The first record that does not fit caps the log; nothing follows.
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 1, "{text}");
    assert!(lines[0].contains(r#""event":"log_capped""#), "{text}");
}
