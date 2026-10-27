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
        name.ends_with(".sqlite") || name.ends_with(".lock")
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

const BOARD: &str = "0123456789abcdef";
fn bootstrapped_board(temp: &Path) -> PathBuf {
    use crate::application::project_board::ports::ProjectBoardStore;
    use crate::domain::project_board::entities::claim::Identity;
    let root = temp.join("boards");
    std::fs::create_dir(&root).unwrap();
    let store = super::super::super::SqliteProjectBoardStore::open(
        &root.join(format!("{BOARD}.sqlite")),
        Identity {
            name: "Backup Fixture".into(),
            email: "backup@example.test".into(),
        },
    )
    .unwrap();
    store.bootstrap().unwrap();
    root
}
fn back_up(root: &Path, to: &Path, at: &str) -> BackupOutcome {
    let mut report = SqliteBoardBackup::new(root.to_owned())
        .backup(&BackupRequest {
            board: None,
            to: to.to_owned(),
            at: Timestamp::parse(at).unwrap(),
            limits: BackupLimits::default(),
        })
        .unwrap();
    assert_eq!(report.outcomes.len(), 1);
    report.outcomes.remove(0)
}
fn listing(directory: &Path) -> Vec<String> {
    let mut names: Vec<_> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}
/// #2767: validation of a WAL-mode copy left `-wal`/`-shm` beside every
/// publication until the directory hit the admission entry limit.
#[test]
fn backup_publication_and_prior_validation_leave_no_sidecars() {
    let temp = tempfile::tempdir().unwrap();
    let root = bootstrapped_board(temp.path());
    let to = temp.path().join("snapshots");
    let lock = format!(".{BOARD}.backup.lock");
    let first = back_up(&root, &to, "2026-10-25T12:00:00Z");
    assert_eq!(first.error, None);
    assert_eq!(first.stage, BackupStage::Complete);
    let first_name = format!("{BOARD}-20261025T120000Z.sqlite");
    assert_eq!(listing(&to), [lock.clone(), first_name.clone()]);
    // A rollback-journal header (format bytes 18/19 == 1) keeps every later
    // read-only open of this snapshot from creating WAL sidecars.
    let header = std::fs::read(to.join(&first_name)).unwrap();
    assert_eq!(&header[18..20], &[1, 1]);
    // The next day's run validates (and retains) the first snapshot read-only.
    let second = back_up(&root, &to, "2026-10-26T13:00:00Z");
    assert_eq!(second.error, None);
    assert_eq!(
        listing(&to),
        [lock, first_name, format!("{BOARD}-20261026T130000Z.sqlite"),]
    );
}
#[test]
fn backup_retention_prunes_sidecars_of_pruned_and_orphaned_snapshots_by_canonical_name_only() {
    let temp = tempfile::tempdir().unwrap();
    let root = bootstrapped_board(temp.path());
    let to = temp.path().join("snapshots");
    let lock = format!(".{BOARD}.backup.lock");
    let seed = back_up(&root, &to, "2026-10-20T10:00:00Z");
    assert_eq!(seed.error, None);
    let pruned = format!("{BOARD}-20261020T100000Z.sqlite");
    let kept = format!("{BOARD}-20261020T110000Z.sqlite");
    std::fs::copy(to.join(&pruned), to.join(&kept)).unwrap();
    let touch = |name: &str| std::fs::write(to.join(name), b"").unwrap();
    let mut removed = Vec::new();
    for suffix in ["-wal", "-shm", "-journal"] {
        // Sidecars of the snapshot retention prunes in this run.
        removed.push(format!("{pruned}{suffix}"));
        // Orphans whose snapshot an earlier run already pruned.
        removed.push(format!("{BOARD}-20261001T120000Z.sqlite{suffix}"));
    }
    let foreign = [
        // A retained snapshot's sidecar is never an orphan.
        format!("{kept}-shm"),
        // Not the canonical spelling: allowlist refuses each one.
        "notes.sqlite-wal".to_owned(),
        format!("{BOARD}-20261001T120000Z.sqlite-walx"),
        format!("{BOARD}-2026100XT120000Z.sqlite-wal"),
        format!("{BOARD}-20261001T120000Z.sqlite.wal"),
        format!(".{BOARD}-20261001T120000Z.sqlite-wal"),
        // Another board's sidecars belong to that board's locked run.
        "fedcba9876543210-20261001T120000Z.sqlite-wal".to_owned(),
    ];
    for name in removed.iter().chain(&foreign) {
        touch(name);
    }
    let outcome = back_up(&root, &to, "2026-10-25T12:00:00Z");
    assert_eq!(outcome.error, None);
    assert_eq!(outcome.stage, BackupStage::Complete);
    assert_eq!((outcome.retained, outcome.deleted), (2, 1));
    let mut expected: Vec<String> = foreign.to_vec();
    expected.extend([lock, kept, format!("{BOARD}-20261025T120000Z.sqlite")]);
    expected.sort();
    assert_eq!(listing(&to), expected);
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Refuse {
    Nothing,
    WalSidecars,
    WalSidecarsWithIo,
    SecondSnapshot,
}
/// Counts directory syncs; optionally refuses chosen unlinks.
struct CountingSyncs {
    syncs: std::sync::atomic::AtomicUsize,
    snapshot_prunes: std::sync::atomic::AtomicUsize,
    refuse: Refuse,
}
impl CountingSyncs {
    fn new(refuse: Refuse) -> Arc<Self> {
        Arc::new(Self {
            syncs: std::sync::atomic::AtomicUsize::new(0),
            snapshot_prunes: std::sync::atomic::AtomicUsize::new(0),
            refuse,
        })
    }
    fn syncs(&self) -> usize {
        self.syncs.load(std::sync::atomic::Ordering::SeqCst)
    }
}
impl BackupFileOps for CountingSyncs {
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
        SystemBackupFileOps.publish_noreplace(directory, partial, name)
    }
    fn sync_directory(&self, directory: &File) -> io::Result<()> {
        self.syncs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        SystemBackupFileOps.sync_directory(directory)
    }
    fn prune_admitted(&self, directory: &File, candidate: &AdmittedBackupFile) -> io::Result<()> {
        let nth_snapshot = if candidate.name.ends_with(".sqlite") {
            self.snapshot_prunes
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                + 1
        } else {
            0
        };
        let refused = match self.refuse {
            Refuse::Nothing => None,
            // An entry-level refusal, as the anchored check open reports it.
            Refuse::WalSidecars => candidate.name.ends_with("-wal").then_some(libc::EACCES),
            Refuse::WalSidecarsWithIo => candidate.name.ends_with("-wal").then_some(libc::EIO),
            Refuse::SecondSnapshot => (nth_snapshot == 2).then_some(libc::EIO),
        };
        if let Some(errno) = refused {
            Err(io::Error::from_raw_os_error(errno))
        } else {
            SystemBackupFileOps.prune_admitted(directory, candidate)
        }
    }
}
fn back_up_with(
    root: &Path,
    to: &Path,
    at: &str,
    files: Arc<dyn BackupFileOps>,
    limits: BackupLimits,
) -> BackupOutcome {
    let mut report = SqliteBoardBackup::with_files(root.to_owned(), files)
        .backup(&BackupRequest {
            board: None,
            to: to.to_owned(),
            at: Timestamp::parse(at).unwrap(),
            limits,
        })
        .unwrap();
    assert_eq!(report.outcomes.len(), 1);
    report.outcomes.remove(0)
}
#[test]
fn backup_sweep_only_retention_syncs_the_directory() {
    let temp = tempfile::tempdir().unwrap();
    let root = bootstrapped_board(temp.path());
    let to = temp.path().join("snapshots");
    assert_eq!(back_up(&root, &to, "2026-10-20T10:00:00Z").error, None);
    let orphan = format!("{BOARD}-20261001T120000Z.sqlite-wal");
    std::fs::write(to.join(&orphan), b"").unwrap();
    let files = CountingSyncs::new(Refuse::Nothing);
    let outcome = back_up_with(
        &root,
        &to,
        "2026-10-25T12:00:00Z",
        files.clone(),
        BackupLimits::default(),
    );
    assert_eq!(outcome.error, None);
    assert_eq!((outcome.retained, outcome.deleted), (2, 0));
    assert!(!listing(&to).contains(&orphan));
    // One sync for the publication, one for the sweep's unlink.
    assert_eq!(files.syncs(), 2);
}
/// A sweep candidate that cannot be unlinked is left alone; the sweep goes on.
#[test]
fn backup_refused_sidecar_unlink_is_skipped_and_the_prune_is_synced() {
    let temp = tempfile::tempdir().unwrap();
    let root = bootstrapped_board(temp.path());
    let to = temp.path().join("snapshots");
    assert_eq!(back_up(&root, &to, "2026-10-20T10:00:00Z").error, None);
    let pruned = format!("{BOARD}-20261020T100000Z.sqlite");
    std::fs::copy(
        to.join(&pruned),
        to.join(format!("{BOARD}-20261020T110000Z.sqlite")),
    )
    .unwrap();
    std::fs::write(to.join(format!("{pruned}-wal")), b"").unwrap();
    let files = CountingSyncs::new(Refuse::WalSidecars);
    let outcome = back_up_with(
        &root,
        &to,
        "2026-10-25T12:00:00Z",
        files.clone(),
        BackupLimits::default(),
    );
    assert_eq!(outcome.error, None);
    assert_eq!(outcome.stage, BackupStage::Complete);
    assert_eq!(outcome.deleted, 1);
    let names = listing(&to);
    assert!(!names.contains(&pruned));
    assert!(names.contains(&format!("{pruned}-wal")));
    assert_eq!(files.syncs(), 2);
}
#[test]
fn backup_prune_failing_after_an_earlier_prune_still_syncs_before_reporting() {
    let temp = tempfile::tempdir().unwrap();
    let root = bootstrapped_board(temp.path());
    let to = temp.path().join("snapshots");
    assert_eq!(back_up(&root, &to, "2026-10-20T10:00:00Z").error, None);
    let seed = to.join(format!("{BOARD}-20261020T100000Z.sqlite"));
    // Same day: 11:00 is retained, then 10:00 and 09:00 are pruned in that order.
    for hour in ["11", "09"] {
        std::fs::copy(
            &seed,
            to.join(format!("{BOARD}-20261020T{hour}0000Z.sqlite")),
        )
        .unwrap();
    }
    let files = CountingSyncs::new(Refuse::SecondSnapshot);
    let outcome = back_up_with(
        &root,
        &to,
        "2026-10-25T12:00:00Z",
        files.clone(),
        BackupLimits::default(),
    );
    assert_eq!(outcome.error, Some(BackupError::RetentionFailed));
    assert_eq!(outcome.stage, BackupStage::Retention);
    assert_eq!(outcome.deleted, 1);
    let names = listing(&to);
    assert!(!names.contains(&format!("{BOARD}-20261020T100000Z.sqlite")));
    assert!(names.contains(&format!("{BOARD}-20261020T090000Z.sqlite")));
    // The first prune's unlink is made durable before the failure is reported.
    assert_eq!(files.syncs(), 2);
}
#[test]
fn backup_sweep_skips_an_unremovable_canonical_orphan_and_continues() {
    use std::os::unix::fs::PermissionsExt;
    // SAFETY: geteuid takes no pointers and has no caller preconditions.
    if unsafe { libc::geteuid() } == 0 {
        // Root opens a mode-000 file, so the refusal cannot be staged.
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = bootstrapped_board(temp.path());
    let to = temp.path().join("snapshots");
    assert_eq!(back_up(&root, &to, "2026-10-20T10:00:00Z").error, None);
    let locked = format!("{BOARD}-20261001T120000Z.sqlite-wal");
    std::fs::write(to.join(&locked), b"").unwrap();
    std::fs::set_permissions(to.join(&locked), std::fs::Permissions::from_mode(0o000)).unwrap();
    let orphan = format!("{BOARD}-20261009T120000Z.sqlite-wal");
    std::fs::write(to.join(&orphan), b"").unwrap();
    let outcome = back_up(&root, &to, "2026-10-25T12:00:00Z");
    assert_eq!(outcome.error, None);
    assert_eq!(outcome.stage, BackupStage::Complete);
    let names = listing(&to);
    assert!(
        names.contains(&locked),
        "the unremovable entry is left alone"
    );
    assert!(
        !names.contains(&orphan),
        "the later genuine orphan is swept"
    );
}
/// Another board's concurrent run can grow the shared destination past the
/// sweep's bound; the sweep then works on what it listed, never on presence
/// inferred from a partial listing.
#[test]
fn backup_sweep_listing_overflow_is_not_fatal_and_keeps_retained_sidecars() {
    let temp = tempfile::tempdir().unwrap();
    let to = temp.path().join("snapshots");
    std::fs::create_dir(&to).unwrap();
    let retained = format!("{BOARD}-20261020T100000Z.sqlite");
    std::fs::write(to.join(&retained), b"snapshot").unwrap();
    for suffix in ["-wal", "-shm", "-journal"] {
        std::fs::write(to.join(format!("{retained}{suffix}")), b"").unwrap();
    }
    for n in 0..8 {
        std::fs::write(to.join(format!("other-{n}")), b"").unwrap();
    }
    let adapter = SqliteBoardBackup::new(temp.path().to_owned());
    let limits = BackupLimits {
        entries: 1,
        ..BackupLimits::default()
    };
    let budget = Budget {
        owner: &adapter,
        invocation: adapter.time.elapsed(),
        board: adapter.time.elapsed(),
        limits: &limits,
    };
    let directory = fs::directory(&to).unwrap();
    let mut swept = 0;
    assert_eq!(
        adapter.sweep_sidecars(&directory, &to, &budget, BOARD, &mut swept),
        Ok(())
    );
    assert_eq!(swept, 0);
    let names = listing(&to);
    for suffix in ["-wal", "-shm", "-journal"] {
        assert!(names.contains(&format!("{retained}{suffix}")));
    }
}
#[test]
fn backup_sweep_skips_odd_canonical_entries_and_continues() {
    use std::os::unix::{
        ffi::OsStringExt,
        fs::{MetadataExt, symlink},
    };
    let temp = tempfile::tempdir().unwrap();
    let root = bootstrapped_board(temp.path());
    let to = temp.path().join("snapshots");
    assert_eq!(back_up(&root, &to, "2026-10-20T10:00:00Z").error, None);
    std::fs::write(to.join("target.txt"), b"foreign").unwrap();
    let symlinked = format!("{BOARD}-20261001T120000Z.sqlite-wal");
    symlink(to.join("target.txt"), to.join(&symlinked)).unwrap();
    let hardlinked = format!("{BOARD}-20261002T120000Z.sqlite-wal");
    std::fs::write(to.join("peer.txt"), b"peer").unwrap();
    std::fs::hard_link(to.join("peer.txt"), to.join(&hardlinked)).unwrap();
    let directory = format!("{BOARD}-20261003T120000Z.sqlite-wal");
    std::fs::create_dir(to.join(&directory)).unwrap();
    let fifo = format!("{BOARD}-20261004T120000Z.sqlite-wal");
    let fifo_path = std::ffi::CString::new(to.join(&fifo).into_os_string().into_vec()).unwrap();
    // SAFETY: a live NUL-terminated path; mkfifo takes no other pointers.
    assert_eq!(unsafe { libc::mkfifo(fifo_path.as_ptr(), 0o600) }, 0);
    let orphan = format!("{BOARD}-20261009T120000Z.sqlite-wal");
    std::fs::write(to.join(&orphan), b"").unwrap();
    let outcome = back_up(&root, &to, "2026-10-25T12:00:00Z");
    assert_eq!(outcome.error, None);
    assert_eq!(outcome.stage, BackupStage::Complete);
    let names = listing(&to);
    for odd in [&symlinked, &hardlinked, &directory, &fifo] {
        assert!(names.contains(odd), "{odd} is left alone");
    }
    assert!(
        !names.contains(&orphan),
        "the later genuine orphan is swept"
    );
    assert_eq!(std::fs::read(to.join("target.txt")).unwrap(), b"foreign");
    assert_eq!(std::fs::metadata(to.join("peer.txt")).unwrap().nlink(), 2);
}
#[test]
fn backup_sweep_runs_when_preflight_admitted_a_full_directory() {
    let temp = tempfile::tempdir().unwrap();
    let root = bootstrapped_board(temp.path());
    let to = temp.path().join("snapshots");
    assert_eq!(back_up(&root, &to, "2026-10-20T10:00:00Z").error, None);
    for suffix in ["-wal", "-shm"] {
        std::fs::write(
            to.join(format!("{BOARD}-20261001T120000Z.sqlite{suffix}")),
            b"",
        )
        .unwrap();
    }
    let source_entries = std::fs::read_dir(&root).unwrap().count();
    let limit = listing(&to).len();
    assert_eq!(
        limit, 4,
        "lock + snapshot + two orphans fill the limit exactly"
    );
    assert!(source_entries <= limit);
    let limits = BackupLimits {
        entries: limit,
        ..BackupLimits::default()
    };
    let outcome = back_up_with(
        &root,
        &to,
        "2026-10-25T12:00:00Z",
        Arc::new(SystemBackupFileOps),
        limits,
    );
    assert_eq!(outcome.error, None);
    assert_eq!(
        listing(&to),
        [
            format!(".{BOARD}.backup.lock"),
            format!("{BOARD}-20261020T100000Z.sqlite"),
            format!("{BOARD}-20261025T120000Z.sqlite"),
        ]
    );
}
fn sweep_fixture(temp: &Path) -> (SqliteBoardBackup, PathBuf, BackupLimits) {
    let to = temp.join("snapshots");
    std::fs::create_dir(&to).unwrap();
    (
        SqliteBoardBackup::new(temp.to_owned()),
        to,
        BackupLimits::default(),
    )
}
/// Presence of a snapshot is asked of the directory: a listing that names a
/// retained snapshot's sidecars but not the snapshot itself never sweeps them.
#[test]
fn backup_sweep_asks_the_directory_whether_a_listed_sidecars_snapshot_exists() {
    let temp = tempfile::tempdir().unwrap();
    let (adapter, to, limits) = sweep_fixture(temp.path());
    let retained = format!("{BOARD}-20261020T100000Z.sqlite");
    std::fs::write(to.join(&retained), b"snapshot").unwrap();
    let orphan = format!("{BOARD}-20261001T120000Z.sqlite-wal");
    let mut names = vec![format!("{retained}-journal"), format!("{retained}-wal")];
    names.push(orphan.clone());
    for name in &names {
        std::fs::write(to.join(name), b"").unwrap();
    }
    names.sort();
    let budget = Budget {
        owner: &adapter,
        invocation: adapter.time.elapsed(),
        board: adapter.time.elapsed(),
        limits: &limits,
    };
    let directory = fs::directory(&to).unwrap();
    let mut swept = 0;
    assert_eq!(
        adapter.sweep_listed(
            &SweepScope {
                directory: &directory,
                canonical: &to,
                board: BOARD,
            },
            &budget,
            &names,
            &mut swept
        ),
        Ok(())
    );
    assert_eq!(
        swept, 1,
        "the listed canonical orphan was reached and swept"
    );
    assert_eq!(
        listing(&to),
        [
            retained.clone(),
            format!("{retained}-journal"),
            format!("{retained}-wal"),
        ]
    );
}
#[test]
fn backup_sweep_listing_failure_is_a_retention_failure() {
    let temp = tempfile::tempdir().unwrap();
    let (adapter, to, limits) = sweep_fixture(temp.path());
    std::fs::write(to.join("not-a-directory"), b"").unwrap();
    let budget = Budget {
        owner: &adapter,
        invocation: adapter.time.elapsed(),
        board: adapter.time.elapsed(),
        limits: &limits,
    };
    let file = File::open(to.join("not-a-directory")).unwrap();
    let mut swept = 0;
    assert_eq!(
        adapter.sweep_sidecars(&file, &to, &budget, BOARD, &mut swept),
        Err(BackupError::RetentionFailed)
    );
}
/// Only entry-level refusals are skipped; a device-level unlink error fails
/// retention after the earlier unlinks are made durable.
#[test]
fn backup_sweep_io_error_fails_retention_after_syncing() {
    let temp = tempfile::tempdir().unwrap();
    let root = bootstrapped_board(temp.path());
    let to = temp.path().join("snapshots");
    assert_eq!(back_up(&root, &to, "2026-10-20T10:00:00Z").error, None);
    let pruned = format!("{BOARD}-20261020T100000Z.sqlite");
    std::fs::copy(
        to.join(&pruned),
        to.join(format!("{BOARD}-20261020T110000Z.sqlite")),
    )
    .unwrap();
    std::fs::write(to.join(format!("{pruned}-wal")), b"").unwrap();
    let files = CountingSyncs::new(Refuse::WalSidecarsWithIo);
    let outcome = back_up_with(
        &root,
        &to,
        "2026-10-25T12:00:00Z",
        files.clone(),
        BackupLimits::default(),
    );
    assert_eq!(outcome.error, Some(BackupError::RetentionFailed));
    assert_eq!(outcome.stage, BackupStage::Retention);
    assert_eq!(outcome.deleted, 1);
    assert!(listing(&to).contains(&format!("{pruned}-wal")));
    assert_eq!(files.syncs(), 2);
}
