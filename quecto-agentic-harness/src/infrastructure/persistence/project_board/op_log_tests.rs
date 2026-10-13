//! Epic #2481 T2: the tracing adapter traces each record at its access's
//! level, holding the `board_op` line's fields only, then hands it on; a
//! dropped record raises one warning.
use std::sync::{Arc, Mutex};

use super::*;
use crate::domain::project_board::value_objects::telemetry::{
    Client, FrontEnd, OpKind, OpOutcome, StaticKind,
};
use crate::infrastructure::persistence::audit_log::AuditLog;

#[derive(Default)]
struct Kept(Mutex<Vec<BoardOpRecord>>);

impl ProjectBoardOpLog for Arc<Kept> {
    fn record(&self, record: BoardOpRecord) {
        self.0.lock().unwrap().push(record);
    }
}

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
fn a_read_is_traced_at_debug_and_a_write_at_info_then_handed_on() {
    let kept = Arc::new(Kept::default());
    let ops = TracingProjectBoardOps::over(kept.clone());
    let read = traced(tracing::Level::TRACE, || ops.record(record(Access::Read)));
    assert!(
        read.contains("DEBUG") && read.contains("quecto::project_board"),
        "{read}"
    );
    assert!(read.contains("board op"), "{read}");
    let json = serde_json::to_string(&record(Access::Read)).unwrap();
    assert!(read.contains(&json), "the line's fields: {read}");
    let info_only = traced(tracing::Level::INFO, || ops.record(record(Access::Read)));
    assert_eq!(info_only, "", "a read is DEBUG");
    let write = traced(tracing::Level::INFO, || ops.record(record(Access::Write)));
    assert!(
        write.contains("INFO") && write.contains(r#""rule":"not_holder""#),
        "{write}"
    );
    let kept = kept.0.lock().unwrap();
    let handed: Vec<Access> = kept.iter().map(|record| record.access).collect();
    assert_eq!(handed, [Access::Read, Access::Read, Access::Write]);
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
