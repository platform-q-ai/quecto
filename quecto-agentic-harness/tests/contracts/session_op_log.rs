//! `SessionOpLog` on the event-log adapter (#2313): the session's event
//! log writes a run's summary (`BoardOpLog::summarize`) as one line under
//! no turn, notes the records and summaries dropped before they reached
//! it, one line per kind, and hands over only the drops it has not noted.
use quecto::application::swarm::dto::DroppedRecords;
use quecto::application::swarm::ports::{BoardOpLog, SessionOpLog};
use quecto::domain::swarm::RunSummaryFold;
use quecto::infrastructure::persistence::audit_log::AuditLog;
use quecto::infrastructure::persistence::board_op_log::EventLogBoardOps;
use serde_json::json;

fn lines(base: &std::path::Path, key: &str) -> Vec<serde_json::Value> {
    std::fs::read_to_string(AuditLog::file_path(base, key))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// The event log writes a run summary as one line under no turn, and its
/// drop notes: one line per kind dropped, none for nothing.
#[test]
fn the_event_log_writes_the_summary_and_the_drop_notes() {
    let base = tempfile::tempdir().unwrap();
    let log = AuditLog::open_sync(base.path(), "cli:summary").unwrap();
    let ops = EventLogBoardOps::new(log.crash_line().unwrap());
    let run = "0123456789abcdef0123456789abcdef";
    ops.summarize(RunSummaryFold::new(run, 0).summary(1, None));
    ops.dropped(DroppedRecords::default());
    ops.dropped(DroppedRecords {
        ops: 2,
        summaries: 1,
    });
    let written = lines(base.path(), "cli:summary");
    assert_eq!(written.len(), 3, "{written:?}");
    assert_eq!(written[0]["event"], "swarm_run_summary");
    assert_eq!(written[0]["turn"], serde_json::Value::Null);
    assert_eq!(
        (&written[1]["event"], &written[1]["dropped"]),
        (&json!("swarm_ops_dropped"), &json!(2))
    );
    assert_eq!(
        (&written[2]["event"], &written[2]["dropped"]),
        (&json!("swarm_run_summary_dropped"), &json!(1))
    );
}

/// A log that noted its drops holds none unnoted: nothing is handed to
/// the log that replaces it (what the gate held off is handed over, as
/// the adapter's own tests pin with the gate held).
#[test]
fn noted_drops_are_not_handed_over() {
    let base = tempfile::tempdir().unwrap();
    let log = AuditLog::open_sync(base.path(), "cli:noted").unwrap();
    let ops = EventLogBoardOps::new(log.crash_line().unwrap());
    assert_eq!(ops.take_unnoted(), DroppedRecords::default(), "fresh");
    ops.dropped(DroppedRecords {
        ops: 1,
        summaries: 1,
    });
    assert_eq!(ops.take_unnoted(), DroppedRecords::default(), "noted");
    assert_eq!(lines(base.path(), "cli:noted").len(), 2);
}
