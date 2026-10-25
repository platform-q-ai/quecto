use super::*;
/// Pinned SQLite discards CHECK expressions when loading a read-only schema.
/// Keep the source read-only and prove engine-owned temporary-copy validation.
#[test]
fn backup_readonly_anchored_integrity_detects_check_violation() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("candidate.sqlite");
    let c = super::super::super::storage::open(&path).unwrap();
    c.execute_batch("PRAGMA ignore_check_constraints=ON; INSERT INTO objects(oid,kind,bytes) VALUES('bad','invalid',X'00'); PRAGMA ignore_check_constraints=OFF").unwrap();
    assert_eq!(exact_integrity(&c), Err(BackupError::IntegrityFailed));
    drop(c);
    let directory = fs::directory(temp.path()).unwrap();
    let readonly = open_read(
        &fs::path(&directory, "candidate.sqlite"),
        Duration::from_millis(10),
    )
    .unwrap();
    let count: i64 = readonly
        .query_row(
            "SELECT count(*) FROM objects WHERE kind='invalid'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(
        exact_integrity(&readonly),
        Ok(()),
        "pinned dependency's read-only CHECK limitation"
    );
    let mut validation = Connection::open("").unwrap();
    SystemBackupSqliteOps
        .copy(
            &readonly,
            &mut validation,
            Duration::from_secs(1),
            Duration::from_millis(10),
        )
        .unwrap();
    assert_eq!(
        exact_integrity(&validation),
        Err(BackupError::IntegrityFailed)
    );
}

struct LostPublication;
impl BackupFileOps for LostPublication {
    fn create_partial(&self, directory: &File, name: &str) -> io::Result<File> {
        SystemBackupFileOps.create_partial(directory, name)
    }
    fn sync_file(&self, file: &File) -> io::Result<()> {
        SystemBackupFileOps.sync_file(file)
    }
    fn publish_noreplace(
        &self,
        directory: &File,
        partial: &AdmittedBackupFile,
        name: &str,
    ) -> io::Result<()> {
        fs::link(directory, &partial.name, name)?;
        Err(io::Error::other(
            "owned test loses answer after actual link",
        ))
    }
    fn sync_directory(&self, directory: &File) -> io::Result<()> {
        SystemBackupFileOps.sync_directory(directory)
    }
    fn prune_admitted(&self, directory: &File, candidate: &AdmittedBackupFile) -> io::Result<()> {
        SystemBackupFileOps.prune_admitted(directory, candidate)
    }
}
#[test]
fn backup_post_link_error_reports_uncertainty_keeps_snapshot_and_cleans_owned_partial() {
    use crate::application::project_board::ports::ProjectBoardStore;
    use crate::domain::project_board::entities::claim::Identity;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("boards");
    std::fs::create_dir(&root).unwrap();
    let store = super::super::super::SqliteProjectBoardStore::open(
        &root.join("0123456789abcdef.sqlite"),
        Identity {
            name: "Backup Fixture".into(),
            email: "backup@example.test".into(),
        },
    )
    .unwrap();
    store.bootstrap().unwrap();
    let to = temp.path().join("snapshots");
    let adapter = SqliteBoardBackup::with_files(root, Arc::new(LostPublication));
    let report = adapter
        .backup(&BackupRequest {
            board: None,
            to: to.clone(),
            at: Timestamp::parse("2026-10-25T12:00:00Z").unwrap(),
            limits: BackupLimits::default(),
        })
        .unwrap();
    let outcome = &report.outcomes[0];
    assert_eq!(outcome.error, Some(BackupError::PublicationUncertain));
    let published = outcome.published.as_ref().unwrap();
    let c = Connection::open_with_flags(published, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    assert_eq!(exact_integrity(&c), Ok(()));
    assert!(std::fs::read_dir(to).unwrap().all(|entry| {
        let name = entry.unwrap().file_name().into_string().unwrap();
        name.ends_with(".sqlite")
            || name.ends_with(".lock")
            || name.ends_with("-shm")
            || name.ends_with("-wal")
    }));
}

#[test]
fn backup_copy_budget_is_shared_across_actual_sqlite_copies() {
    use crate::application::project_board::ports::{BoardClockError, ReferenceStatus};
    use std::sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    };
    struct Clock(AtomicU64);
    impl BoardClock for Clock {
        fn now(&self) -> Result<Timestamp, BoardClockError> {
            Ok(Timestamp::parse("2026-10-25T12:00:00Z").unwrap())
        }
        fn elapsed(&self) -> Duration {
            Duration::from_millis(self.0.load(Ordering::SeqCst))
        }
        fn reference(&self) -> Option<Timestamp> {
            panic!("offline")
        }
        fn reference_status(&self) -> ReferenceStatus {
            ReferenceStatus::NoReference("offline".into())
        }
        fn pause(&self, _: usize) {
            panic!("no retry")
        }
    }
    struct TimedCopy {
        clock: Arc<Clock>,
        received: Mutex<Vec<Duration>>,
    }
    impl BackupSqliteOps for TimedCopy {
        fn copy(
            &self,
            source: &Connection,
            destination: &mut Connection,
            remaining: Duration,
            busy: Duration,
        ) -> Result<(), BackupError> {
            self.received.lock().unwrap().push(remaining);
            let result = SystemBackupSqliteOps.copy(source, destination, remaining, busy);
            self.clock.0.fetch_add(300, Ordering::SeqCst);
            result
        }
        fn integrity(&self, c: &Connection) -> rusqlite::Result<Vec<String>> {
            SystemBackupSqliteOps.integrity(c)
        }
    }
    let clock = Arc::new(Clock(AtomicU64::new(0)));
    let operations = TimedCopy {
        clock: clock.clone(),
        received: Mutex::new(Vec::new()),
    };
    let adapter = SqliteBoardBackup::with_runtime_and_clock(
        PathBuf::new(),
        Arc::new(SystemBackupFileOps),
        Arc::new(SystemBackupSqliteOps),
        Arc::new(SystemValidationDeadline),
        clock,
    );
    let limits = BackupLimits {
        copy: Duration::from_millis(400),
        board: Duration::from_secs(2),
        invocation: Duration::from_secs(4),
        ..BackupLimits::default()
    };
    let budget = Budget {
        owner: &adapter,
        invocation: Duration::ZERO,
        board: Duration::ZERO,
        limits: &limits,
    };
    let source = Connection::open_in_memory().unwrap();
    source
        .execute_batch("CREATE TABLE sentinel(value); INSERT INTO sentinel VALUES(42)")
        .unwrap();
    let mut remaining = limits.copy;
    for _ in 0..2 {
        let mut destination = Connection::open_in_memory().unwrap();
        adapter
            .copy_with_budget(
                &operations,
                &source,
                &mut destination,
                &budget,
                &mut remaining,
            )
            .unwrap();
        assert_eq!(
            destination
                .query_row("SELECT value FROM sentinel", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            42
        );
    }
    let mut destination = Connection::open_in_memory().unwrap();
    assert_eq!(
        adapter.copy_with_budget(
            &operations,
            &source,
            &mut destination,
            &budget,
            &mut remaining
        ),
        Err(BackupError::BusyDeadline)
    );
    assert_eq!(
        *operations.received.lock().unwrap(),
        [Duration::from_millis(400), Duration::from_millis(100)]
    );
}
