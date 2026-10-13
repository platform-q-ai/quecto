//! Epic #2481 T2: the tracing adapter traces each record at its access's
//! level, holding the `board_op` line's fields only, then hands it on; a
//! dropped record raises one warning.
use std::sync::{Arc, Mutex};

use super::*;
use crate::domain::project_board::value_objects::telemetry::{
    Client, FrontEnd, OpKind, OpOutcome, StaticKind,
};
use crate::infrastructure::persistence::audit_log::AuditLog;

fn record(access: Access) -> BoardOpRecord {
    BoardOpRecord {
        op: OpKind::Move,
        front_end: FrontEnd::Tool,
        client: Client::Agent,
        access,
        outcome: OpOutcome::Refused {
            code: StaticKind::new("rule"),
            rule: Some(StaticKind::new("not_holder")),
        },
        fault: None,
        task: None,
        pr: None,
        head: None,
        listing: None,
        cancelled: false,
        build_us: None,
        result_bytes: 0,
        duration_us: 9,
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

/// What `run` writes to `tracing` at `level` and above.
fn traced(level: tracing::Level, run: impl FnOnce()) -> String {
    let captured = Captured::default();
    let sink = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || sink.clone())
        .with_ansi(false)
        .with_max_level(level)
        .finish();
    tracing::subscriber::with_default(subscriber, run);
    String::from_utf8(captured.0.lock().unwrap().clone()).unwrap()
}

#[test]
fn a_read_is_traced_at_debug_and_a_write_at_info_then_written() {
    let base = tempfile::tempdir().unwrap();
    let log = AuditLog::open_sync(base.path(), "cli:traced").unwrap();
    let ops = TracingProjectBoardOps::over(EventLogProjectBoardOps::new(log.crash_line().unwrap()));
    let read = traced(tracing::Level::TRACE, || ops.record(record(Access::Read)));
    assert!(
        read.contains("DEBUG") && read.contains("quecto::project_board"),
        "{read}"
    );
    assert!(read.contains("board op"), "{read}");
    let json = serde_json::to_string(&record(Access::Read)).unwrap();
    assert!(read.contains(&json), "the record's own fields: {read}");
    assert!(!read.contains("cli:traced"), "no envelope: {read}");
    let info_only = traced(tracing::Level::INFO, || ops.record(record(Access::Read)));
    assert_eq!(info_only, "", "a read is DEBUG");
    let write = traced(tracing::Level::INFO, || ops.record(record(Access::Write)));
    assert!(
        write.contains("INFO") && write.contains(r#""rule":"not_holder""#),
        "{write}"
    );
    let text = std::fs::read_to_string(AuditLog::file_path(base.path(), "cli:traced")).unwrap();
    let accesses: Vec<String> = text
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap()["access"].to_string())
        .collect();
    assert_eq!(accesses, [r#""read""#, r#""read""#, r#""write""#]);
}

#[test]
fn a_dropped_record_warns_once() {
    let base = tempfile::tempdir().unwrap();
    let log = AuditLog::open_sync(base.path(), "cli:full")
        .unwrap()
        .with_cap(16);
    let ops = EventLogProjectBoardOps::new(log.crash_line().unwrap());
    let warned = traced(tracing::Level::WARN, || {
        ops.record(record(Access::Read));
        ops.record(record(Access::Read));
    });
    assert_eq!(warned.matches("not being written").count(), 1, "{warned}");
    assert!(ops.warned.load(Ordering::Acquire));
}

#[test]
fn drops_are_counted_saturating() {
    let base = tempfile::tempdir().unwrap();
    let log = AuditLog::open_sync(base.path(), "cli:count").unwrap();
    let ops = EventLogProjectBoardOps::new(log.crash_line().unwrap());
    ops.count_dropped(u64::MAX - 1);
    ops.count_dropped(5);
    assert_eq!(ops.dropped.load(Ordering::Acquire), u64::MAX);
}

/// #2536 review: a log opened by its first record is opened once, by that
/// record, and never before it.
#[test]
fn a_log_opened_by_its_first_record_is_opened_once_then() {
    use std::sync::atomic::AtomicUsize;
    let base = tempfile::tempdir().unwrap();
    let opened = Arc::new(AtomicUsize::new(0));
    let (dir, count) = (base.path().to_path_buf(), opened.clone());
    let ops = EventLogProjectBoardOps::opened_by_first_record(move || {
        count.fetch_add(1, Ordering::AcqRel);
        AuditLog::open_sync(&dir, "cli:lazy").ok()?.crash_line()
    });
    assert_eq!(opened.load(Ordering::Acquire), 0, "not before a record");
    assert!(!AuditLog::directory(base.path()).exists());
    ops.record(record(Access::Read));
    ops.record(record(Access::Write));
    assert_eq!(opened.load(Ordering::Acquire), 1, "once");
    let text = std::fs::read_to_string(AuditLog::file_path(base.path(), "cli:lazy")).unwrap();
    assert_eq!(text.lines().count(), 2, "{text}");
}

/// A log that cannot be opened drops every record, with one warning: the
/// opener is tried once, and no record is traced (#2536 review round 2).
#[test]
fn a_log_that_cannot_be_opened_drops_its_records_with_one_warning() {
    use std::sync::atomic::AtomicUsize;
    let tried = Arc::new(AtomicUsize::new(0));
    let count = tried.clone();
    let ops =
        TracingProjectBoardOps::over(EventLogProjectBoardOps::opened_by_first_record(move || {
            count.fetch_add(1, Ordering::AcqRel);
            None
        }));
    let traced = traced(tracing::Level::TRACE, || {
        ops.record(record(Access::Read));
        ops.record(record(Access::Write));
    });
    assert_eq!(tried.load(Ordering::Acquire), 1, "tried once");
    assert_eq!(traced.matches("not being written").count(), 1, "{traced}");
    assert!(!traced.contains("board op"), "no record traced: {traced}");
    assert_eq!(
        ops.inner.dropped.load(Ordering::Acquire),
        0,
        "nothing to note in"
    );
}
