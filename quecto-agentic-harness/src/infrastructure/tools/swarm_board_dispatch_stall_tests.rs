//! #2303 swarm review: a board call never waits on a stuck event log. Its
//! answer is computed before its `swarm_op` is recorded, and the record
//! waits for the log's write gate only a small bound: past it, the record
//! is dropped and counted, and the next record written notes the drop.
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::call;
use crate::application::swarm::dto::BoardLocation;
use crate::application::swarm::ports::{Clock, IdSource};
use crate::composition::swarm::{board_op_log, with_event_log};
use crate::infrastructure::persistence::audit_log::AuditLog;
use crate::infrastructure::persistence::swarm_board::repository::SqliteBoardRepository;

struct Fixed;
impl Clock for Fixed {
    fn now_seconds(&self) -> f64 {
        1_000.0
    }
}

#[derive(Default)]
struct Counter(std::sync::Mutex<u64>);
impl IdSource for Counter {
    fn hex32(&self) -> String {
        let mut next = self.0.lock().unwrap();
        *next += 1;
        format!("{:032x}", *next)
    }
}

fn lines(base: &tempfile::TempDir) -> Vec<Value> {
    std::fs::read_to_string(AuditLog::file_path(base.path(), "cli:stall"))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// Another writer holds the event log's gate for 2 s: the board call still
/// answers within about 200 ms, its record dropped and counted; once the
/// gate is free, the next call's record lands after a note of the drop.
#[test]
fn a_board_call_answers_while_another_writer_holds_the_log() {
    let (base, dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let log = AuditLog::open_sync(base.path(), "cli:stall").unwrap();
    let handles = with_event_log(
        SqliteBoardRepository::new(&BoardLocation {
            database: dir.path().join("swarm.sqlite"),
            checkout: dir.path().to_path_buf(),
        }),
        std::sync::Arc::new(Fixed),
        std::sync::Arc::new(Counter::default()),
        board_op_log(true, &log).expect("the event log is on"),
    );
    call(&handles, "parent", "bootstrap_run", json!([1, "s", null])).unwrap();
    let holder = log.hold_gate_for(Duration::from_secs(2));
    let started = Instant::now();
    let answer = call(&handles, "parent", "_status", json!([]));
    let waited = started.elapsed();
    assert!(answer.is_ok(), "{answer:?}");
    assert!(waited < Duration::from_millis(200), "{waited:?}");
    holder.join().unwrap();
    assert_eq!(lines(&base).len(), 1, "only the bootstrap's record");
    call(&handles, "parent", "_status", json!([])).unwrap();
    let lines = lines(&base);
    let events: Vec<&str> = lines
        .iter()
        .map(|line| line["event"].as_str().unwrap())
        .collect();
    assert_eq!(events, ["swarm_op", "swarm_ops_dropped", "swarm_op"]);
    assert_eq!(lines[1]["dropped"], 1);
}
