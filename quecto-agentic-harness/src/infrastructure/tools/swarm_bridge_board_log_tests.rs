//! #2313: where a board's calls are recorded. The records follow a session
//! switch into the new session's log; the admission's calls are held from
//! admission, when the event log was decided on before it, and written
//! first once the log opens; and the coordinator's harness writes the run's
//! summary once, its counts the run's `swarm_op` records.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::log::PENDING_RECORDS;
use crate::application::swarm::ports::{BoardOpLog, CoordinationPort};
use crate::domain::swarm::{BoardOpObservation, BoardOpOutcome, RefusalKind, SwarmRunSummary};
use crate::infrastructure::persistence::audit_log::AuditLog;
use crate::infrastructure::persistence::crash_record::Armed;
use crate::infrastructure::tools::swarm_bridge::SwarmContext;

/// The event log, in memory: the records and summaries written, in order.
#[derive(Default)]
struct Recorded {
    ops: Mutex<Vec<BoardOpObservation>>,
    summaries: Mutex<Vec<SwarmRunSummary>>,
}

impl BoardOpLog for Recorded {
    fn record(&self, observation: BoardOpObservation) {
        self.ops.lock().unwrap().push(observation);
    }

    fn summarize(&self, summary: SwarmRunSummary) {
        self.summaries.lock().unwrap().push(summary);
    }
}

impl Recorded {
    fn records(&self) -> Vec<BoardOpObservation> {
        self.ops.lock().unwrap().clone()
    }

    fn ops(&self) -> Vec<String> {
        self.records().into_iter().map(|record| record.op).collect()
    }

    fn summaries(&self) -> Vec<SwarmRunSummary> {
        self.summaries.lock().unwrap().clone()
    }
}

fn context(checkout: &std::path::Path, member: &str, board: &super::SwarmBoard) -> SwarmContext {
    SwarmContext {
        lifecycle: Arc::new(crate::application::swarm::LifecycleService),
        checkout: checkout.to_path_buf(),
        member: member.into(),
        board: board.clone(),
    }
}

/// A run created by `parent` in its checkout's board, for three members.
fn create(parent: &SwarmContext) {
    std::fs::create_dir_all(parent.checkout.join(".quecto")).unwrap();
    let deadline = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 300;
    parent
        .call(
            "create",
            json!(["ship", [], [{"id":"tests","kind":"command","description":"pass"}], 3, deadline]),
        )
        .unwrap();
}

/// Each session's `swarm_op` records in the audit logs under `base`, by
/// the session whose log holds them.
fn ops_by_session(base: &std::path::Path) -> BTreeMap<String, Vec<String>> {
    let mut sessions: BTreeMap<String, Vec<String>> = BTreeMap::new();
    // Only the logs: the crash records keep a directory beside them.
    let logs = std::fs::read_dir(base.join("audit"))
        .unwrap()
        .map(|file| file.unwrap().path())
        .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("jsonl"));
    for path in logs {
        let text = std::fs::read_to_string(path).unwrap();
        for line in text.lines() {
            let line: Value = serde_json::from_str(line).unwrap();
            let session = line["session"].as_str().unwrap().to_owned();
            let ops = sessions.entry(session).or_default();
            if line["event"] == "swarm_op" {
                ops.push(line["op"].as_str().unwrap().to_owned());
            }
        }
    }
    sessions
}

/// The session-pinned log (#2313, #2311 round-2 review L4): once the
/// session switches, the event log the crash target follows to is the one
/// the board's records go to; nothing more is written to the departed
/// session's log.
#[test]
fn a_switch_moves_the_boards_records_to_the_new_sessions_file() {
    let base = tempfile::tempdir().unwrap();
    let checkout = tempfile::tempdir().unwrap();
    let first = AuditLog::open_sync(base.path(), "cli:a").unwrap();
    let target = Armed::new(base.path(), Some("cli:a"), first.crash_line());
    let board = crate::composition::swarm::swarm_board();
    assert!(board.record_in_session(true, &first));
    let parent = context(checkout.path(), "parent", &board);
    create(&parent);
    let arriving = target.follow(Some("cli:b")).expect("the log follows");
    assert!(board.follow_session(&arriving));
    parent
        .call("task_create", json!(["r1", "t", ["tests pass"]]))
        .unwrap();
    let sessions = ops_by_session(base.path());
    assert_eq!(sessions["cli:a"], ["create"], "{sessions:?}");
    assert_eq!(sessions["cli:b"], ["task_create"], "{sessions:?}");
}

/// A board that records nothing (the event log is off) does not start
/// recording on a switch.
#[test]
fn a_board_that_records_nothing_does_not_start_on_a_switch() {
    let base = tempfile::tempdir().unwrap();
    let checkout = tempfile::tempdir().unwrap();
    let board = crate::composition::swarm::swarm_board();
    let arriving = AuditLog::open_sync(base.path(), "cli:b").unwrap();
    assert!(!board.follow_session(&arriving));
    create(&context(checkout.path(), "parent", &board));
    assert_eq!(ops_by_session(base.path()).get("cli:b"), None);
}

/// A later session's log replaces an earlier one's (#2313: it was the
/// first log only).
#[test]
fn a_board_records_in_the_latest_sessions_log() {
    let checkout = tempfile::tempdir().unwrap();
    let board = crate::composition::swarm::swarm_board();
    let (first, second) = (Arc::new(Recorded::default()), Arc::new(Recorded::default()));
    assert!(board.record_in(first.clone()));
    assert!(board.record_in(second.clone()));
    create(&context(checkout.path(), "parent", &board));
    assert!(first.ops().is_empty());
    assert_eq!(second.ops(), ["create"]);
}

/// The admission's calls (#2313, S13 review L1): with the event log
/// decided on before admission, the calls made before the session's log is
/// open are measured, held, and written there first, in order.
#[test]
fn the_admissions_calls_are_written_first_once_the_log_opens() {
    let checkout = tempfile::tempdir().unwrap();
    let board = crate::composition::swarm::swarm_board();
    board.record_from_admission(true);
    let parent = context(checkout.path(), "parent", &board);
    create(&parent);
    parent.summary().unwrap();
    let recorded = Arc::new(Recorded::default());
    assert!(board.record_in(recorded.clone()));
    assert_eq!(recorded.ops(), ["create", "summary"]);
    assert!(
        recorded.records()[0].lock_wait_us.is_some(),
        "measured while held"
    );
    parent
        .call("task_create", json!(["r1", "t", ["tests pass"]]))
        .unwrap();
    assert_eq!(recorded.ops(), ["create", "summary", "task_create"]);
}

/// With the event log off before admission (owner decision T1), the
/// admission's calls are neither measured nor held.
#[test]
fn with_the_event_log_off_before_admission_nothing_is_held() {
    let checkout = tempfile::tempdir().unwrap();
    let board = crate::composition::swarm::swarm_board();
    board.record_from_admission(false);
    create(&context(checkout.path(), "parent", &board));
    let recorded = Arc::new(Recorded::default());
    assert!(board.record_in(recorded.clone()));
    assert!(recorded.ops().is_empty());
}

/// A session that keeps no event log after all (off, or none opened)
/// drops what the admission held, and the board records nothing more.
#[test]
fn a_session_without_an_event_log_drops_what_the_admission_held() {
    let base = tempfile::tempdir().unwrap();
    let checkout = tempfile::tempdir().unwrap();
    let log = AuditLog::open_sync(base.path(), "cli:a").unwrap();
    let board = crate::composition::swarm::swarm_board();
    board.record_from_admission(true);
    let parent = context(checkout.path(), "parent", &board);
    create(&parent);
    assert!(!board.record_in_session(false, &log));
    parent
        .call("task_create", json!(["r1", "t", ["tests pass"]]))
        .unwrap();
    assert_eq!(ops_by_session(base.path()).get("cli:a"), None);
    let held = crate::composition::swarm::swarm_board();
    held.record_from_admission(true);
    create(&context(
        tempfile::tempdir().unwrap().path(),
        "parent",
        &held,
    ));
    held.stop_recording();
    let recorded = Arc::new(Recorded::default());
    assert!(held.record_in(recorded.clone()));
    assert!(recorded.ops().is_empty(), "{:?}", recorded.ops());
}

/// The records held before the log opens are bounded: past
/// [`PENDING_RECORDS`] they are dropped.
#[test]
fn the_records_held_before_the_log_opens_are_bounded() {
    let checkout = tempfile::tempdir().unwrap();
    let board = crate::composition::swarm::swarm_board();
    board.record_from_admission(true);
    let parent = context(checkout.path(), "parent", &board);
    create(&parent);
    for _ in 0..PENDING_RECORDS {
        parent.summary().unwrap();
    }
    let recorded = Arc::new(Recorded::default());
    assert!(board.record_in(recorded.clone()));
    assert_eq!(recorded.ops().len(), PENDING_RECORDS);
    assert_eq!(recorded.ops()[0], "create", "the first are kept");
}

/// How many of `records` are of `run`, for `op`, answered with `decision`.
fn answered(records: &[BoardOpObservation], run: &str, op: &str, decision: &str) -> u64 {
    let count = records
        .iter()
        .filter(|record| record.run_id.as_deref() == Some(run) && record.op == op)
        .filter(|record| record.outcome == BoardOpOutcome::Ok)
        .filter(|record| record.decision.as_deref() == Some(decision))
        .count();
    u64::try_from(count).unwrap()
}

/// #2313: the coordinator's harness writes one `swarm_run_summary` once
/// its run settles, and its counts are the run's `swarm_op` records: per
/// op and refusal kind, tasks, messages and each member's requests. A
/// member id shaped like a credential is never written.
#[test]
fn the_run_summary_counts_equal_the_runs_swarm_op_records() {
    let checkout = tempfile::tempdir().unwrap();
    let board = crate::composition::swarm::swarm_board();
    let recorded = Arc::new(Recorded::default());
    assert!(board.record_in(recorded.clone()));
    let parent = context(checkout.path(), "parent", &board);
    let worker = context(checkout.path(), "worker", &board);
    let secret = "sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    let stranger = context(checkout.path(), secret, &board);
    create(&parent);
    parent.call("_admit", json!(["worker", "r1"])).unwrap();
    parent
        .call("_activate", json!(["worker", "r1", 7, "s", "/w.sock"]))
        .unwrap();
    for request in ["t1", "t2"] {
        parent
            .call("task_create", json!([request, "t", ["tests pass"]]))
            .unwrap();
    }
    let token = worker.call("claim", json!([1])).unwrap()["token"].clone();
    worker.call("claim", json!([999])).unwrap_err();
    worker
        .call(
            "submit",
            json!([1, token, [{"artifact": "report", "revision": "R1"}]]),
        )
        .unwrap();
    parent.call("verify_task", json!([1, token, "R1"])).unwrap();
    let sent = parent
        .call("send", json!(["m1", "worker", "hello"]))
        .unwrap();
    worker.call("ack", json!([sent["id"]])).unwrap();
    let record = json!([{"request_id": "q", "instrumented_attempts": 0, "outcome": "rejected"}]);
    parent.call("_record_request", record.clone()).unwrap();
    stranger.call("_record_request", record).unwrap_err();
    stranger
        .call("task_create", json!(["t3", "t", ["x"]]))
        .unwrap_err();
    parent.cancel_run().unwrap();
    let snapshot = parent.snapshot().unwrap();
    assert!(!worker.summarize_settled(&snapshot), "only the coordinator");
    assert!(parent.summarize_settled(&snapshot));
    assert!(!parent.summarize_settled(&snapshot), "once per run");

    let summaries = recorded.summaries();
    assert_eq!(summaries.len(), 1, "{summaries:?}");
    let summary = &summaries[0];
    let run = summary.run_id.as_str();
    let records: Vec<_> = recorded
        .records()
        .into_iter()
        .filter(|record| record.run_id.as_deref() == Some(run))
        .collect();
    assert_eq!(summary.records, u64::try_from(records.len()).unwrap());
    let mut ok: BTreeMap<String, u64> = BTreeMap::new();
    let mut refused: BTreeMap<(String, RefusalKind), u64> = BTreeMap::new();
    for record in &records {
        match record.outcome {
            BoardOpOutcome::Ok => *ok.entry(record.op.clone()).or_default() += 1,
            BoardOpOutcome::Refused { kind, .. } => {
                *refused.entry((record.op.clone(), kind)).or_default() += 1;
            }
        }
    }
    for (op, counted) in &summary.ops {
        assert_eq!(counted.ok, ok.get(op).copied().unwrap_or(0), "{op}");
        for (kind, count) in &counted.refused {
            assert_eq!(
                Some(count),
                refused.get(&(op.clone(), *kind)),
                "{op} {kind:?}"
            );
        }
    }
    let summed: u64 = summary
        .ops
        .values()
        .map(|op| op.ok + op.refused.values().sum::<u64>())
        .sum();
    assert_eq!(summed, summary.records);
    assert!(
        !summary.ops["claim"].refused.is_empty(),
        "the unknown task's claim is a refusal: {refused:?}"
    );
    assert_eq!(
        summary.tasks.created,
        answered(&records, run, "task_create", "created")
    );
    assert_eq!(summary.tasks.created, 2);
    assert_eq!(summary.tasks.claimed, 1);
    assert_eq!(summary.tasks.submitted, 1);
    assert_eq!(summary.tasks.accepted, 1);
    assert_eq!(summary.messages.sent, 1);
    assert_eq!(summary.messages.acked, 1);
    let usage: Vec<_> = summary
        .request_usage
        .iter()
        .map(|usage| (usage.actor_ref.as_str(), usage.recorded, usage.refused))
        .collect();
    assert_eq!(usage[0], ("parent", 1, 0), "{usage:?}");
    let text = serde_json::to_string(&summaries).unwrap();
    assert!(!text.contains("sk-ant"), "no secret-shaped id: {text}");
    assert!(
        !text.contains("hello") && !text.contains("report"),
        "{text}"
    );
}

/// A writer into a shared buffer, for the `tracing` records.
struct Captured(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Captured {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The `tracing` records of one settled run's summary, written under a
/// subscriber of this thread's own.
fn traced_summary() -> String {
    let checkout = tempfile::tempdir().unwrap();
    let board = crate::composition::swarm::swarm_board();
    assert!(board.record_in(Arc::new(Recorded::default())));
    let parent = context(checkout.path(), "parent", &board);
    create(&parent);
    parent.cancel_run().unwrap();
    let snapshot = parent.snapshot().unwrap();
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let sink = buffer.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || Captured(sink.clone()))
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        tracing::callsite::rebuild_interest_cache();
        assert!(parent.summarize_settled(&snapshot));
    });
    String::from_utf8(buffer.lock().unwrap().clone()).unwrap()
}

/// #2313: the summary also leaves one `tracing` record on the board's
/// target, with the run's id and its counts only. A test running at the
/// same time can leave a callsite's cached interest stale for a moment, so
/// a record missed is looked for again on a fresh run, as the dispatcher's
/// capture does.
#[test]
fn a_run_summary_is_traced_on_the_boards_target() {
    let mut text = String::new();
    for _ in 0..5 {
        text = traced_summary();
        if text.contains("swarm run summary") {
            break;
        }
    }
    let traced: Vec<_> = text
        .lines()
        .filter(|line| line.contains("swarm run summary"))
        .collect();
    assert_eq!(traced.len(), 1, "{text}");
    assert!(
        traced[0].contains(crate::infrastructure::tools::swarm_board_dispatch::TELEMETRY_TARGET)
    );
    assert!(traced[0].contains("records="), "{text}");
}
