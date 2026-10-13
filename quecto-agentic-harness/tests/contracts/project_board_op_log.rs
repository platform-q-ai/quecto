//! `ProjectBoardOpLog` on the event-log adapter, and composition's
//! recording builders (epic #2481 T2): a record is appended synchronously
//! from a plain thread as one `board_op` line filed under no turn; one held
//! off the log's write gate past 50 ms is dropped and counted, and the next
//! record written is preceded by a `board_ops_dropped` note; with the event
//! log off, neither builder gives a recording, and nothing is written to
//! the event log, to `tracing` or to the CLI's daily log.
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use quecto::application::project_board::ports::ProjectBoardOpLog;
use quecto::composition::bootstrap::project_board::{cli_board_recording, session_board_recording};
use quecto::domain::project_board::value_objects::telemetry::{
    Access, BoardOpRecord, Client, FrontEnd, OpKind, OpOutcome,
};
use quecto::infrastructure::config::telemetry::{EventLogConfig, TelemetryConfig};
use quecto::infrastructure::persistence::audit_log::AuditLog;
use quecto::infrastructure::persistence::project_board::op_log::EventLogProjectBoardOps;
use serde_json::Value;

fn record() -> BoardOpRecord {
    BoardOpRecord {
        op: OpKind::Show,
        front_end: FrontEnd::Cli,
        client: Client::Cli,
        access: Access::Read,
        outcome: OpOutcome::Ok,
        fault: None,
        task: None,
        pr: Some(7),
        head: None,
        listing: None,
        cancelled: false,
        build_us: None,
        result_bytes: 0,
        duration_us: 12,
    }
}

fn telemetry(enabled: bool) -> TelemetryConfig {
    TelemetryConfig {
        event_log: EventLogConfig { enabled },
    }
}

/// The owner's global file at `base`, switching the event log `enabled`.
fn global(base: &Path, enabled: bool) {
    let config = serde_json::json!({"telemetry": {"event_log": {"enabled": enabled}}});
    std::fs::write(base.join("config.json"), config.to_string()).unwrap();
}

/// Every line of the log at `base`/`key`, parsed.
fn parsed(base: &Path, key: &str) -> Vec<Value> {
    std::fs::read_to_string(AuditLog::file_path(base, key))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// The names of the files in `base`'s audit directory.
fn audit_files(base: &Path) -> Vec<String> {
    match std::fs::read_dir(AuditLog::directory(base)) {
        Ok(entries) => entries
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect(),
        Err(_) => Vec::new(),
    }
}

#[derive(Clone, Default)]
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

/// What `run` writes to `tracing`, at every level, under a subscriber of
/// this thread's own.
fn traced(run: impl FnOnce()) -> String {
    let captured = Captured::default();
    let sink = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || sink.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();
    tracing::subscriber::with_default(subscriber, run);
    String::from_utf8(captured.0.lock().unwrap().clone()).unwrap()
}

/// Owner condition (2026-10-12): with the event log off, in the session's
/// configuration and the owner's global file, neither builder gives a
/// recording, and nothing reaches the event log, `tracing` or the disk.
#[test]
fn off_gives_no_recording_from_either_builder_and_writes_nothing() {
    for global_file in [None, Some(false)] {
        let base = tempfile::tempdir().unwrap();
        if let Some(enabled) = global_file {
            global(base.path(), enabled);
        }
        let log = AuditLog::open_sync(base.path(), "cli:off").unwrap();
        // Any recording a builder gives records, so the checks below would
        // see what it writes or traces before the `None` checks fail.
        let mut built = (None, None);
        let output = traced(|| {
            built = (
                session_board_recording(&telemetry(false), base.path(), &log),
                cli_board_recording(base.path()),
            );
            for recording in [&built.0, &built.1].into_iter().flatten() {
                recording.record(record());
            }
        });
        assert_eq!(output, "", "nothing is traced while off");
        assert!(parsed(base.path(), "cli:off").is_empty());
        assert_eq!(
            audit_files(base.path()),
            ["cli:off.jsonl".replace(':', "_")]
        );
        assert!(built.0.is_none(), "the session builder gives none");
        assert!(built.1.is_none(), "the CLI builder gives none");
    }
}

/// The sessions' rule (#2150): the session's configuration or the owner's
/// global file switches recording on; a configuration never switches the
/// global file off.
#[test]
fn the_session_builder_records_when_its_config_or_the_global_file_is_on() {
    for (config_on, global_on) in [(true, false), (false, true), (true, true)] {
        let base = tempfile::tempdir().unwrap();
        global(base.path(), global_on);
        let log = AuditLog::open_sync(base.path(), "cli:on").unwrap();
        let recording = session_board_recording(&telemetry(config_on), base.path(), &log)
            .expect("a recording while on");
        let output = traced(|| recording.record(record()));
        assert!(
            output.contains("board op") && output.contains("quecto::project_board"),
            "the recording traces while on: {output}"
        );
        let lines = parsed(base.path(), "cli:on");
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!(lines[0]["event"], "board_op");
        assert_eq!(lines[0]["session"], "cli:on");
        assert_eq!(lines[0]["turn"], Value::Null);
    }
}

/// The `quecto board` CLI loads no layered configuration: its daily
/// `board-cli:<UTC date>` log follows the owner's global file, and is
/// opened (dated) by the first record, so a CLI run that records nothing
/// leaves no empty file.
#[test]
fn the_cli_builder_records_in_its_daily_log_when_the_global_file_is_on() {
    let base = tempfile::tempdir().unwrap();
    global(base.path(), true);
    let recording = cli_board_recording(base.path()).expect("a recording while on");
    assert_eq!(
        audit_files(base.path()),
        Vec::<String>::new(),
        "the daily log is opened by its first record, not before"
    );
    let output = std::thread::spawn(move || traced(|| recording.record(record())))
        .join()
        .unwrap();
    assert!(
        output.contains("board op") && output.contains("quecto::project_board"),
        "the recording traces while on: {output}"
    );
    let files = audit_files(base.path());
    assert_eq!(files.len(), 1, "{files:?}");
    let day = files[0]
        .strip_prefix("board-cli_")
        .and_then(|name| name.strip_suffix(".jsonl"))
        .expect("board-cli_<date>.jsonl");
    let shaped = day.len() == 10
        && day.chars().enumerate().all(|(at, c)| match at {
            4 | 7 => c == '-',
            _ => c.is_ascii_digit(),
        });
    assert!(shaped, "{day} is a date");
    let lines = parsed(base.path(), &format!("board-cli:{day}"));
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0]["event"], "board_op");
    assert_eq!(lines[0]["session"], format!("board-cli:{day}"));
    assert_eq!(lines[0]["turn"], Value::Null);
}

/// A daily log that cannot be opened records nothing, and traces no
/// record either, as a session with no log line gives no recording.
#[test]
fn the_cli_builder_traces_nothing_when_its_log_cannot_be_opened() {
    let base = tempfile::tempdir().unwrap();
    global(base.path(), true);
    std::fs::write(AuditLog::directory(base.path()), "not a directory").unwrap();
    let recording = cli_board_recording(base.path()).expect("a recording while on");
    let output = traced(|| recording.record(record()));
    assert!(!output.contains("board op"), "no record traced: {output}");
    assert!(
        output.contains("not being written"),
        "one warning: {output}"
    );
}

#[test]
fn a_record_is_appended_from_a_plain_thread_under_no_turn() {
    let base = tempfile::tempdir().unwrap();
    let log = AuditLog::open_sync(base.path(), "cli:contract").unwrap();
    let ops = EventLogProjectBoardOps::new(log.crash_line().unwrap());
    std::thread::spawn(move || {
        ops.record(record());
        ops.record(record());
    })
    .join()
    .expect("no runtime is needed and nothing panics");
    let lines = parsed(base.path(), "cli:contract");
    assert_eq!(lines.len(), 2, "{lines:?}");
    for line in lines {
        assert_eq!(line["event"], "board_op");
        assert_eq!(line["op"], "show");
        assert_eq!(line["outcome"], "ok");
        assert_eq!(line["pr"], 7);
        assert_eq!(line["turn"], Value::Null);
        assert_eq!(line["session"], "cli:contract");
    }
}

/// The record never holds up the board: held off the log's write gate, it
/// gives up after 50 ms and is counted; the next record written is
/// preceded by the note of the drop, in the same write, and the count
/// starts again.
#[test]
fn a_record_held_off_past_50_ms_is_dropped_and_noted_before_the_next() {
    let base = tempfile::tempdir().unwrap();
    let log = AuditLog::open_sync(base.path(), "cli:held").unwrap();
    let ops = EventLogProjectBoardOps::new(log.crash_line().unwrap());
    let holder = log.hold_gate_for(Duration::from_secs(2));
    let started = Instant::now();
    ops.record(record());
    ops.record(record());
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_millis(100),
        "each waits 50 ms: {waited:?}"
    );
    assert!(
        waited < Duration::from_millis(1500),
        "neither waits it out: {waited:?}"
    );
    holder.join().unwrap();
    assert!(parsed(base.path(), "cli:held").is_empty());
    ops.record(record());
    let lines = parsed(base.path(), "cli:held");
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert_eq!(lines[0]["event"], "board_ops_dropped");
    assert_eq!(lines[0]["dropped"], 2);
    assert_eq!(lines[0]["turn"], Value::Null);
    assert_eq!(lines[1]["event"], "board_op");
    ops.record(record());
    let lines = parsed(base.path(), "cli:held");
    assert_eq!(lines.len(), 3, "no second note: {lines:?}");
    assert_eq!(lines[2]["event"], "board_op");
}

#[test]
fn a_record_the_log_cannot_take_is_dropped_quietly() {
    let base = tempfile::tempdir().unwrap();
    let log = AuditLog::open_sync(base.path(), "cli:full")
        .unwrap()
        .with_cap(16);
    let ops = EventLogProjectBoardOps::new(log.crash_line().unwrap());
    ops.record(record());
    ops.record(record());
    let lines = parsed(base.path(), "cli:full");
    // The first record that does not fit caps the log; nothing follows.
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0]["event"], "log_capped");
}
