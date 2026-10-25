//! Failure/lifecycle contracts on borrowed real SQLite connections. The adapter
//! (not these query operations) must own the interrupt watchdog and its lease.
use super::{board_backup::old_snapshot, fixture::*};
use quecto::application::project_board::ports::backup::*;
use quecto::infrastructure::persistence::sqlite::project_board::{
    backup::SqliteBoardBackup, backup_files::SystemBackupFileOps, backup_sqlite::*,
};
use rusqlite::{Connection, InterruptHandle};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Default)]
struct WatchStats {
    armed: AtomicUsize,
    joined: AtomicUsize,
    live: AtomicUsize,
    abandoned: AtomicUsize,
}
struct RecordingDeadline(Arc<WatchStats>);
struct RecordingLease {
    inner: Box<dyn ValidationLease>,
    stats: Arc<WatchStats>,
    active: bool,
}
impl ValidationLease for RecordingLease {
    fn cancel_and_join(&mut self) -> Result<(), BackupError> {
        self.inner.cancel_and_join()?;
        assert!(
            self.active,
            "one explicit cancellation/join per armed lease"
        );
        self.active = false;
        self.stats.joined.fetch_add(1, Ordering::SeqCst);
        self.stats.live.fetch_sub(1, Ordering::SeqCst);
        Ok(())
    }
}
impl Drop for RecordingLease {
    fn drop(&mut self) {
        if self.active {
            self.stats.abandoned.fetch_add(1, Ordering::SeqCst);
            self.stats.live.fetch_sub(1, Ordering::SeqCst);
        }
    }
}
impl BackupValidationDeadline for RecordingDeadline {
    fn arm(
        &self,
        interrupt: InterruptHandle,
        remaining: Duration,
    ) -> Result<Box<dyn ValidationLease>, BackupError> {
        let inner = SystemValidationDeadline.arm(interrupt, remaining)?;
        self.0.armed.fetch_add(1, Ordering::SeqCst);
        self.0.live.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(RecordingLease {
            inner,
            stats: self.0.clone(),
            active: true,
        }))
    }
}
fn joined(stats: &WatchStats) {
    let armed = stats.armed.load(Ordering::SeqCst);
    assert!(
        armed > 0,
        "actual real watchdog armed, not fake cancellation success"
    );
    assert_eq!(stats.joined.load(Ordering::SeqCst), armed);
    assert_eq!(stats.live.load(Ordering::SeqCst), 0);
    assert_eq!(stats.abandoned.load(Ordering::SeqCst), 0);
}
enum Fault {
    PartialFull,
    LongValidation,
    Rows(Vec<String>),
}
struct SqliteFault {
    fault: Fault,
    reached: AtomicUsize,
    interrupted: AtomicBool,
    partial: Mutex<Option<PathBuf>>,
}
impl BackupSqliteOps for SqliteFault {
    fn copy(
        &self,
        source: &Connection,
        destination: &mut Connection,
        remaining: Duration,
        busy: Duration,
    ) -> Result<(), BackupError> {
        match self.fault {
            Fault::PartialFull => {
                self.reached.fetch_add(1, Ordering::SeqCst);
                // Commit real bytes to the adapter-owned partial, then make the
                // next actual SQLite page allocation fail SQLITE_FULL.
                destination.execute_batch("CREATE TABLE b13_fault_payload(bytes BLOB); INSERT INTO b13_fault_payload VALUES(zeroblob(512));").unwrap();
                let path = PathBuf::from(destination.path().unwrap());
                assert!(std::fs::metadata(&path).unwrap().len() > 0);
                *self.partial.lock().unwrap() = Some(path);
                let pages: i64 = destination
                    .query_row("PRAGMA page_count", [], |r| r.get(0))
                    .unwrap();
                destination
                    .pragma_update(None, "max_page_count", pages)
                    .unwrap();
                let failure = destination
                    .execute(
                        "INSERT INTO b13_fault_payload VALUES(zeroblob(1048576))",
                        [],
                    )
                    .expect_err("real destination page write must exhaust cap");
                assert!(
                    failure
                        .sqlite_error()
                        .is_some_and(|e| e.extended_code & 0xff == rusqlite::ffi::SQLITE_FULL)
                );
                Err(BackupError::PublishFailed)
            }
            _ => SystemBackupSqliteOps.copy(source, destination, remaining, busy),
        }
    }
    fn integrity(&self, connection: &Connection) -> rusqlite::Result<Vec<String>> {
        self.reached.fetch_add(1, Ordering::SeqCst);
        match &self.fault {
            Fault::LongValidation => {
                let result = connection.query_row("WITH RECURSIVE endless(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM endless) SELECT sum(x) FROM endless", [], |r| r.get::<_, i64>(0));
                let failure = result.expect_err(
                    "adapter-owned remaining-budget watchdog must interrupt real SQLite validation",
                );
                assert!(
                    failure
                        .sqlite_error()
                        .is_some_and(|e| e.extended_code & 0xff == rusqlite::ffi::SQLITE_INTERRUPT)
                );
                self.interrupted.store(true, Ordering::SeqCst);
                Err(failure)
            }
            Fault::Rows(rows) => Ok(rows.clone()),
            Fault::PartialFull => SystemBackupSqliteOps.integrity(connection),
        }
    }
}
fn fault(mode: Fault) -> Arc<SqliteFault> {
    Arc::new(SqliteFault {
        fault: mode,
        reached: AtomicUsize::new(0),
        interrupted: AtomicBool::new(false),
        partial: Mutex::new(None),
    })
}
#[test]
fn r4_actual_destination_page_write_failure_after_partial_cleans_owned_files() {
    let _deadline = ProbeDeadline::new();
    let f = Database::new();
    let old = old_snapshot(&f.to, A, 24, 9);
    let bytes = std::fs::read(&old).unwrap();
    let before = names(&f.to);
    let sqlite = fault(Fault::PartialFull);
    let adapter = SqliteBoardBackup::with_runtime(
        f.root.clone(),
        Arc::new(SystemBackupFileOps),
        sqlite.clone(),
        Arc::new(SystemValidationDeadline),
    );
    let result = adapter.backup(&f.request());
    assert_eq!(std::fs::read(old).unwrap(), bytes);
    error(result, BackupError::PublishFailed);
    assert_eq!(sqlite.reached.load(Ordering::SeqCst), 1);
    let partial = sqlite.partial.lock().unwrap().clone().unwrap();
    assert!(!partial.exists());
    assert!(!PathBuf::from(format!("{}-wal", partial.display())).exists());
    assert!(!PathBuf::from(format!("{}-shm", partial.display())).exists());
    assert!(!f.final_path().exists());
    assert!(
        names(&f.to)
            .iter()
            .all(|n| before.contains(n) || n.ends_with(".lock"))
    );
}
#[test]
fn r5_real_validation_interrupt_cancels_and_joins_watchdog_on_deadline() {
    let _deadline = ProbeDeadline::new();
    let f = Database::new();
    let old = old_snapshot(&f.to, A, 24, 9);
    let bytes = std::fs::read(&old).unwrap();
    let stats = Arc::new(WatchStats::default());
    let sqlite = fault(Fault::LongValidation);
    let adapter = SqliteBoardBackup::with_runtime(
        f.root.clone(),
        Arc::new(SystemBackupFileOps),
        sqlite.clone(),
        Arc::new(RecordingDeadline(stats.clone())),
    );
    let mut request = f.request();
    request.limits.board = Duration::from_millis(50);
    request.limits.copy = Duration::from_millis(25);
    let started = Instant::now();
    let result = adapter.backup(&request);
    assert_eq!(std::fs::read(old).unwrap(), bytes);
    error(result, BackupError::ValidationDeadline);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(sqlite.interrupted.load(Ordering::SeqCst));
    joined(&stats);
    assert!(!f.final_path().exists());
}
#[test]
fn r5_success_cancels_and_joins_validation_watchdog_without_leak() {
    let _deadline = ProbeDeadline::new();
    let f = Database::new();
    let stats = Arc::new(WatchStats::default());
    let adapter = SqliteBoardBackup::with_runtime(
        f.root.clone(),
        Arc::new(SystemBackupFileOps),
        Arc::new(SystemBackupSqliteOps),
        Arc::new(RecordingDeadline(stats.clone())),
    );
    successful(adapter.backup(&f.request()));
    joined(&stats);
    integrity(&f.final_path());
}
fn rows(values: &[&str]) {
    let _deadline = ProbeDeadline::new();
    let f = Database::new();
    let old = old_snapshot(&f.to, A, 24, 9);
    let bytes = std::fs::read(&old).unwrap();
    let stats = Arc::new(WatchStats::default());
    let sqlite = fault(Fault::Rows(values.iter().map(|s| (*s).into()).collect()));
    let adapter = SqliteBoardBackup::with_runtime(
        f.root.clone(),
        Arc::new(SystemBackupFileOps),
        sqlite.clone(),
        Arc::new(RecordingDeadline(stats.clone())),
    );
    let result = adapter.backup(&f.request());
    assert_eq!(std::fs::read(old).unwrap(), bytes);
    error(result, BackupError::IntegrityFailed);
    assert!(sqlite.reached.load(Ordering::SeqCst) > 0);
    joined(&stats);
    assert!(!f.final_path().exists());
}
#[test]
fn r4_integrity_ok_plus_error_row_is_not_exactly_one_ok() {
    rows(&["ok", "CHECK constraint failed"]);
}
#[test]
fn r4_integrity_two_ok_rows_are_not_exactly_one_ok() {
    rows(&["ok", "ok"]);
}
#[test]
fn r4_integrity_empty_rows_are_not_exactly_one_ok() {
    rows(&[]);
}
