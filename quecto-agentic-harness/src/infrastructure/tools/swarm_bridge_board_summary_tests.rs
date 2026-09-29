//! #2313 review M1: each member is a process of its own, with a board of
//! its own over the run's one file. The coordinator's summary counts its
//! own process's `swarm_op` records (`scope: process`), and reads the
//! run-wide totals, every member's work included, from the board file at
//! settle.
use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::json;

use super::{Recorded, context, create};
use crate::domain::swarm::{
    BoardOpObservation, BoardOpOutcome, MessageTotals, RefusalKind, SummaryScope, TaskStates,
};
use crate::infrastructure::tools::swarm_bridge::SwarmContext;

/// A board of its own for `member`, recording in a log of its own: the
/// member's process.
fn process(checkout: &std::path::Path, member: &str) -> (SwarmContext, Arc<Recorded>) {
    let board = crate::composition::swarm::swarm_board();
    let log = Arc::new(Recorded::default());
    assert!(board.record_in(log.clone()));
    (context(checkout, member, &board), log)
}

/// How many of `records` are of `run`, for `op`, answered with `decision`.
fn answered(records: &[BoardOpObservation], op: &str, decision: &str) -> u64 {
    let count = records
        .iter()
        .filter(|record| record.op == op && record.outcome == BoardOpOutcome::Ok)
        .filter(|record| record.decision.as_deref() == Some(decision))
        .count();
    u64::try_from(count).unwrap()
}

/// `count(*)` of the board file's `sql`, read directly.
fn board_count(parent: &SwarmContext, sql: &str) -> u64 {
    let connection = rusqlite::Connection::open(parent.database()).unwrap();
    let count: i64 = connection.query_row(sql, [], |row| row.get(0)).unwrap();
    u64::try_from(count).unwrap()
}

/// The worker claims, submits, sends, withdraws and records a request in
/// its own board; the coordinator's summary counts only the coordinator's
/// own records in its process scope, and the board's state for the run.
#[test]
fn the_summary_counts_its_process_and_reads_the_run_from_the_board() {
    let checkout = tempfile::tempdir().unwrap();
    let (parent, parent_log) = process(checkout.path(), "parent");
    let (worker, worker_log) = process(checkout.path(), "worker");
    let secret = "sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    let (stranger, _stranger_log) = process(checkout.path(), secret);
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
    let first = worker
        .call("send", json!(["m1", "parent", "hello"]))
        .unwrap();
    parent.call("ack", json!([first["id"]])).unwrap();
    let second = worker
        .call("send", json!(["m2", "parent", "hello again"]))
        .unwrap();
    worker.call("withdraw", json!([second["id"]])).unwrap();
    worker
        .call(
            "_record_request",
            json!([{"request_id": "w1", "context_input_tokens": 70, "input_tokens": 50, "output_tokens": 10,
                    "cache_read_tokens": 20, "cache_write_tokens": 0,
                    "instrumented_attempts": 2, "outcome": "succeeded"}]),
        )
        .unwrap();
    let rejected = json!([{"request_id": "p1", "instrumented_attempts": 0, "outcome": "rejected"}]);
    parent.call("_record_request", rejected.clone()).unwrap();
    stranger.call("_record_request", rejected).unwrap_err();
    parent.cancel_run().unwrap();
    let snapshot = parent.snapshot().unwrap();
    assert!(!worker.summarize_settled(&snapshot), "only the coordinator");
    assert!(parent.summarize_settled(&snapshot));
    assert!(!parent.summarize_settled(&snapshot), "once per run");
    assert!(worker_log.summaries().is_empty());

    let summaries = parent_log.summaries();
    assert_eq!(summaries.len(), 1, "{summaries:?}");
    let summary = &summaries[0];
    assert_eq!(summary.scope, SummaryScope::Process);
    // The process scope: exactly the coordinator's own records of the run.
    let records: Vec<_> = parent_log
        .records()
        .into_iter()
        .filter(|record| record.run_id.as_deref() == Some(summary.run_id.as_str()))
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
            assert_eq!(Some(count), refused.get(&(op.clone(), *kind)), "{op}");
        }
    }
    assert!(!summary.ops.contains_key("claim"), "the worker's, not here");
    assert_eq!(
        summary.tasks.created,
        answered(&records, "task_create", "created")
    );
    assert_eq!(summary.tasks.created, 2);
    assert_eq!(summary.tasks.claimed, 0, "claimed in the worker's process");
    assert_eq!(summary.tasks.submitted, 0);
    assert_eq!(summary.messages.acked, 1);
    assert_eq!(summary.messages.sent, 0, "sent in the worker's process");
    assert!(
        worker_log
            .records()
            .iter()
            .any(|record| record.op == "claim")
    );

    // The run-wide section: the board's state, every member's work.
    let run = summary.run.as_ref().expect("the board was read at settle");
    let tasks = |status: &str| {
        board_count(
            &parent,
            &format!("SELECT count(*) FROM tasks WHERE status='{status}'"),
        )
    };
    assert_eq!(
        run.tasks,
        TaskStates {
            total: board_count(&parent, "SELECT count(*) FROM tasks"),
            ready: tasks("ready"),
            claimed: tasks("claimed"),
            blocked: tasks("blocked"),
            submitted: tasks("submitted"),
            completed: tasks("completed"),
        }
    );
    assert_eq!((run.tasks.total, run.tasks.submitted), (2, 1));
    assert_eq!(
        run.messages,
        MessageTotals {
            sent: 2,
            acked: 1,
            withdrawn: 1,
        }
    );
    let usage: BTreeMap<_, _> = run
        .usage
        .iter()
        .map(|usage| (usage.actor_ref.as_str().to_owned(), usage.clone()))
        .collect();
    assert_eq!(usage.len(), 2, "{usage:?}");
    let worker_usage = &usage["worker"];
    assert_eq!(
        (
            worker_usage.requests,
            worker_usage.input_tokens,
            worker_usage.output_tokens,
            worker_usage.cache_read_tokens,
            worker_usage.attempts
        ),
        (1, 50, 10, 20, 2),
        "the worker's real token usage"
    );
    assert_eq!(usage["parent"].requests, 1);
    assert!(run.wall_time_us.is_some(), "from the run's creation");
    let text = serde_json::to_string(&summaries).unwrap();
    assert!(!text.contains("sk-ant"), "no secret-shaped id: {text}");
    assert!(
        !text.contains("hello") && !text.contains("report"),
        "{text}"
    );
}
