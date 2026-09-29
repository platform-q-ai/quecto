//! #2303 round-2 review L1, L2: every writer of a log (the async writer,
//! the board's `swarm_op` appender, the panic hook's crash line) writes
//! under one gate, so no line interleaves with another however long, no
//! line follows `log_capped`, and whichever writer first finds the budget
//! full writes the one `log_capped` record.
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{AuditLog, CRASH_GATE_WAIT};
use crate::domain::audit::AuditEvent;
use crate::domain::redaction::Redacted;
use crate::domain::swarm::{BoardOpObservation, BoardOpOutcome, BoardRole};

fn swarm_op() -> AuditEvent {
    AuditEvent::SwarmOp(BoardOpObservation {
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
    })
}

/// The gate wait of these tests' appends: long enough never to give up
/// on a writer that is writing.
const WAIT: Duration = Duration::from_secs(60);

fn error(message: String) -> AuditEvent {
    AuditEvent::Error {
        source: "agent".into(),
        tool: None,
        message,
        location: None,
    }
}

/// Every line of the log, each parsed: a line two writes interleaved
/// into does not parse.
fn lines(base: &Path, key: &str) -> Vec<serde_json::Value> {
    let text = std::fs::read_to_string(AuditLog::file_path(base, key)).unwrap_or_default();
    assert!(text.is_empty() || text.ends_with('\n'), "a torn last line");
    text.lines()
        .map(|line| {
            serde_json::from_str(line).unwrap_or_else(|e| {
                panic!(
                    "an interleaved line ({e}): {}",
                    &line[..line.len().min(200)]
                )
            })
        })
        .collect()
}

fn capped_count(lines: &[serde_json::Value]) -> usize {
    lines
        .iter()
        .filter(|line| line["event"] == "log_capped")
        .count()
}

/// The async writer and the `swarm_op` appender race to fill a small log:
/// in every round exactly one `log_capped` record is written, and it is
/// the last line (L1: nothing follows it).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_line_follows_log_capped_under_two_writers() {
    for round in 0..20 {
        let base = tempfile::tempdir().unwrap();
        let key = format!("cli:race-{round}");
        let log = Arc::new(
            AuditLog::open_sync(base.path(), &key)
                .unwrap()
                .with_cap(8 * 1024),
        );
        let line = log.crash_line().unwrap();
        let appender = std::thread::spawn(move || {
            for _ in 0..200 {
                let _full = line.append(None, vec![swarm_op()], WAIT);
            }
        });
        for index in 0..200 {
            log.emit(1, error(format!("async {index}"))).await.unwrap();
        }
        appender.join().unwrap();
        let lines = lines(base.path(), &key);
        assert_eq!(capped_count(&lines), 1, "round {round}");
        assert_eq!(
            lines.last().unwrap()["event"],
            "log_capped",
            "round {round}: a line followed log_capped"
        );
    }
}

/// `swarm_op` appends alone fill the budget (L2): the first that does not
/// fit writes the `log_capped` record in its place, and nothing, from
/// either writer, follows it.
#[tokio::test]
async fn swarm_op_appends_that_fill_the_log_cap_it() {
    let base = tempfile::tempdir().unwrap();
    let log = AuditLog::open_sync(base.path(), "cli:ops")
        .unwrap()
        .with_cap(4 * 1024);
    let line = log.crash_line().unwrap();
    let mut written = 0;
    while line.append(None, vec![swarm_op()], WAIT).is_ok() {
        written += 1;
        assert!(written < 1_000, "the cap is reached");
    }
    let refused = line.append(None, vec![swarm_op()], WAIT).unwrap_err();
    assert_eq!(refused.kind(), std::io::ErrorKind::StorageFull);
    log.emit(1, error("after".into())).await.unwrap();
    let lines = lines(base.path(), "cli:ops");
    assert_eq!(lines.len(), written + 1);
    assert_eq!(capped_count(&lines), 1);
    let last = lines.last().unwrap();
    assert_eq!(last["event"], "log_capped");
    assert_eq!(last["turn"], serde_json::Value::Null, "filed as the op was");
}

/// Lines far longer than one pipe buffer, and than tokio's 2 MiB write
/// chunk, from the async writer and the `swarm_op` appender at once, each
/// land whole (L2).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn long_lines_from_two_writers_never_interleave() {
    let base = tempfile::tempdir().unwrap();
    let log = Arc::new(AuditLog::open_sync(base.path(), "cli:long").unwrap());
    let line = log.crash_line().unwrap();
    let appender = std::thread::spawn(move || {
        for _ in 0..400 {
            line.append(None, vec![swarm_op()], WAIT).unwrap();
        }
    });
    let long = "x".repeat(3 * 1024 * 1024);
    for _ in 0..4 {
        log.emit(1, error(long.clone())).await.unwrap();
    }
    appender.join().unwrap();
    let lines = lines(base.path(), "cli:long");
    assert_eq!(lines.len(), 404);
    let long_lines = lines
        .iter()
        .filter(|line| line["message"].as_str() == Some(long.as_str()))
        .count();
    assert_eq!(long_lines, 4);
}

/// A panic hook's line waits for the gate at most [`CRASH_GATE_WAIT`]:
/// held by a writer that never lets go, the line is given up, not hung on.
#[test]
fn the_crash_line_gives_up_on_a_gate_that_stays_held() {
    let base = tempfile::tempdir().unwrap();
    let log = AuditLog::open_sync(base.path(), "cli:held").unwrap();
    let line = log.crash_line().unwrap();
    let held = log.gate.held.lock().unwrap();
    let started = Instant::now();
    let refused = line.write(1, error("dying".into())).unwrap_err();
    let waited = started.elapsed();
    drop(held);
    assert_eq!(refused.kind(), std::io::ErrorKind::WouldBlock);
    assert!(waited >= CRASH_GATE_WAIT, "{waited:?}");
    assert!(
        waited < CRASH_GATE_WAIT + Duration::from_secs(5),
        "{waited:?}"
    );
    assert!(lines(base.path(), "cli:held").is_empty());
    line.write(1, error("dying".into())).unwrap();
}

/// An append of no events writes nothing and is an error, not a panic.
#[test]
fn an_append_of_nothing_is_refused() {
    let base = tempfile::tempdir().unwrap();
    let log = AuditLog::open_sync(base.path(), "cli:none").unwrap();
    let refused = log.crash_line().unwrap().append(None, vec![], WAIT);
    assert_eq!(
        refused.unwrap_err().kind(),
        std::io::ErrorKind::InvalidInput
    );
    assert!(lines(base.path(), "cli:none").is_empty());
}
