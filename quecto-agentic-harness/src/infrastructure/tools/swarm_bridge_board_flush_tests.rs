//! #2313 review L4 and nit: binding the session's log writes what was held
//! outside the binding's lock, so a board call made meanwhile is never
//! held up by the flush, yet lands after the held records; the records
//! dropped while held are noted in the log; and the drops a replaced log
//! had not noted yet are carried to the log that replaces it.
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::super::log::SessionLog;
use crate::application::swarm::ports::BoardOpLog;
use crate::domain::redaction::Redacted;
use crate::domain::swarm::{
    BoardOpDetail, BoardOpObservation, BoardOpOutcome, BoardRole, SwarmRunSummary,
};
use crate::infrastructure::persistence::audit_log::AuditLog;
use crate::infrastructure::persistence::board_op_log::EventLogBoardOps;

fn named(op: &str) -> BoardOpObservation {
    BoardOpObservation {
        op: op.into(),
        actor_ref: Redacted::from("parent"),
        role: Some(BoardRole::Host),
        run_id: None,
        task_id: None,
        message_id: None,
        outcome: BoardOpOutcome::Ok,
        duration_us: 1,
        lock_wait_us: None,
        busy_wait_us: None,
        busy: None,
        cursor_moved: None,
        result_bytes: 0,
        decision: None,
        detail: BoardOpDetail::NONE,
    }
}

/// A log each record of which takes `each` to write, telling `started`
/// when its first write begins.
struct Slow {
    each: Duration,
    written: Mutex<Vec<String>>,
    started: Mutex<Option<mpsc::Sender<()>>>,
}

impl BoardOpLog for Slow {
    fn record(&self, observation: BoardOpObservation) {
        if let Some(started) = self.started.lock().unwrap().take() {
            let _ = started.send(());
        }
        std::thread::sleep(self.each);
        self.written.lock().unwrap().push(observation.op);
    }

    fn summarize(&self, _summary: SwarmRunSummary) {}
}

impl crate::application::swarm::ports::SessionOpLog for Slow {
    fn dropped(&self, _drops: crate::application::swarm::dto::DroppedRecords) {}

    fn take_unnoted(&self) -> crate::application::swarm::dto::DroppedRecords {
        crate::application::swarm::dto::DroppedRecords::default()
    }
}

/// The flush of the held records runs outside the binding's lock: a
/// record written while it runs returns at once, and is written after the
/// held records, never before.
#[test]
fn a_record_made_during_the_flush_is_not_blocked_and_lands_after_it() {
    let session = Arc::new(SessionLog::pending());
    for op in ["h1", "h2", "h3"] {
        session.record(named(op));
    }
    let (started, flushing) = mpsc::channel();
    let slow = Arc::new(Slow {
        each: Duration::from_millis(150),
        written: Mutex::new(Vec::new()),
        started: Mutex::new(Some(started)),
    });
    let binding = {
        let (session, slow) = (session.clone(), slow.clone());
        std::thread::spawn(move || session.bind(slow))
    };
    flushing.recv().unwrap();
    let began = Instant::now();
    session.record(named("late"));
    let waited = began.elapsed();
    assert!(waited < Duration::from_millis(100), "blocked {waited:?}");
    binding.join().unwrap();
    assert_eq!(
        *slow.written.lock().unwrap(),
        ["h1", "h2", "h3", "late"],
        "the held records first"
    );
    session.record(named("after"));
    assert_eq!(slow.written.lock().unwrap().last().unwrap(), "after");
}

/// Every line of the log at `base`/`key`, parsed.
fn parsed(base: &std::path::Path, key: &str) -> Vec<serde_json::Value> {
    std::fs::read_to_string(AuditLog::file_path(base, key))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// The records dropped while held are written into the session's event
/// log as `swarm_ops_dropped`, after the held records.
#[test]
fn the_records_dropped_while_held_are_noted_in_the_event_log() {
    let base = tempfile::tempdir().unwrap();
    let log = AuditLog::open_sync(base.path(), "cli:held").unwrap();
    let session = SessionLog::pending();
    let past = super::super::log::PENDING_RECORDS + 2;
    for n in 0..past {
        session.record(named(&format!("op{n}")));
    }
    session.bind(Arc::new(EventLogBoardOps::new(log.crash_line().unwrap())));
    let lines = parsed(base.path(), "cli:held");
    assert_eq!(lines.len(), super::super::log::PENDING_RECORDS + 1);
    assert_eq!(lines[0]["op"], "op0");
    let note = lines.last().unwrap();
    assert_eq!(note["event"], "swarm_ops_dropped", "{note}");
    assert_eq!(note["dropped"], 2);
}

/// A session switch rebinds the log: the drops the departing log counted
/// but had not noted yet are noted in the arriving one, not lost.
#[test]
fn a_rebind_carries_the_unnoted_drops_to_the_new_log() {
    let base = tempfile::tempdir().unwrap();
    let first = AuditLog::open_sync(base.path(), "cli:a").unwrap();
    let second = AuditLog::open_sync(base.path(), "cli:b").unwrap();
    let session = SessionLog::pending();
    session.bind(Arc::new(
        EventLogBoardOps::new(first.crash_line().unwrap()).with_gate_wait(Duration::from_millis(5)),
    ));
    let holder = first.hold_gate_for(Duration::from_millis(300));
    session.record(named("lost"));
    holder.join().unwrap();
    assert!(parsed(base.path(), "cli:a").is_empty(), "held off");
    session.bind(Arc::new(EventLogBoardOps::new(
        second.crash_line().unwrap(),
    )));
    let lines = parsed(base.path(), "cli:b");
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0]["event"], "swarm_ops_dropped");
    assert_eq!(lines[0]["dropped"], 1);
    session.record(named("next"));
    assert_eq!(parsed(base.path(), "cli:a").len(), 0, "the first is left");
    assert_eq!(parsed(base.path(), "cli:b").len(), 2);
}
