use crate::project_board_history_fixture::{fingerprint, writer};
use quecto::application::project_board::ports::{backup::*, *};
use quecto::domain::project_board::value_objects::timestamp::Timestamp;
use quecto::infrastructure::persistence::sqlite::project_board::{
    SqliteProjectBoardStore, backup::SqliteBoardBackup,
};
use rusqlite::Connection;
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};

pub const A: &str = "0123456789abcdef";
pub const B: &str = "fedcba9876543210";

/// A hard per-probe bound independent of SQLite/Git retry implementations.
/// Normal unwind cancels and joins it; a hung probe terminates without a core dump.
pub struct ProbeDeadline(
    Option<mpsc::Sender<()>>,
    Option<std::thread::JoinHandle<()>>,
);
impl ProbeDeadline {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            if matches!(
                rx.recv_timeout(Duration::from_secs(45)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ) {
                unsafe { libc::_exit(124) }
            }
        });
        Self(Some(tx), Some(thread))
    }
}
impl Drop for ProbeDeadline {
    fn drop(&mut self) {
        self.0.take().unwrap().send(()).ok();
        self.1.take().unwrap().join().unwrap();
    }
}

pub struct Database {
    pub dir: tempfile::TempDir,
    pub root: PathBuf,
    pub to: PathBuf,
    pub path: PathBuf,
    pub store: SqliteProjectBoardStore,
    pub keeper: Connection,
}
impl Database {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("boards");
        let to = dir.path().join("snapshots");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&to).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&to, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let path = root.join(format!("{A}.sqlite"));
        let store = SqliteProjectBoardStore::open(&path, writer()).unwrap();
        store.bootstrap().unwrap();
        let keeper = Connection::open(&path).unwrap();
        keeper
            .execute_batch("PRAGMA wal_autocheckpoint=0; PRAGMA wal_checkpoint(TRUNCATE);")
            .unwrap();
        store
            .fixture_put_raw("tasks/sentinel.md", b"synthetic committed WAL sentinel")
            .unwrap();
        let f = Self {
            dir,
            root,
            to,
            path,
            store,
            keeper,
        };
        f.assert_wal();
        f
    }
    pub fn assert_wal(&self) {
        let wal = self.path.with_extension("sqlite-wal");
        assert!(
            std::fs::metadata(&wal).unwrap().len() > 32,
            "real committed uncheckpointed WAL"
        );
        assert_eq!(
            self.keeper
                .query_row("PRAGMA journal_mode", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "wal"
        );
        assert_eq!(
            self.keeper
                .query_row("PRAGMA synchronous", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            2
        );
        let ro =
            Connection::open_with_flags(&self.path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap();
        assert!(
            ro.query_row(
                "SELECT count(*) FROM objects WHERE bytes=?1",
                [b"synthetic committed WAL sentinel".as_slice()],
                |r| r.get::<_, i64>(0)
            )
            .unwrap()
                > 0
        );
        println!(
            "fixture WAL bytes={} schema=3 sentinel admitted",
            std::fs::metadata(wal).unwrap().len()
        );
    }
    pub fn request(&self) -> BackupRequest {
        BackupRequest {
            board: Some(A.into()),
            to: self.to.clone(),
            at: Timestamp::parse("2026-10-25T12:00:00Z").unwrap(),
            limits: short_limits(),
        }
    }
    pub fn adapter(&self) -> Reached<SqliteBoardBackup> {
        Reached {
            inner: SqliteBoardBackup::new(self.root.clone()),
            calls: AtomicUsize::new(0),
        }
    }
    pub fn final_path(&self) -> PathBuf {
        self.to.join(format!("{A}-20261025T120000Z.sqlite"))
    }
}

/// Records dispatch at the real operational port, not a success-producing fake.
pub struct Reached<T> {
    pub inner: T,
    pub calls: AtomicUsize,
}
impl<T: BoardBackup> BoardBackup for Reached<T> {
    fn backup(&self, r: &BackupRequest) -> Result<BackupReport, BackupError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.backup(r)
    }
}
impl<T> Reached<T> {
    pub fn assert_called(&self) {
        assert_eq!(self.calls.load(Ordering::SeqCst), 1);
        println!("operational BoardBackup::backup reached once");
    }
}
pub fn short_limits() -> BackupLimits {
    BackupLimits {
        copy: Duration::from_millis(400),
        busy: Duration::from_millis(10),
        board: Duration::from_secs(2),
        invocation: Duration::from_secs(4),
        ..BackupLimits::default()
    }
}
pub use crate::board_backup::successful;
pub fn integrity(path: &Path) {
    let c = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let rows: Vec<String> = c
        .prepare("PRAGMA integrity_check")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(rows, ["ok"]);
}
pub fn logical(path: &Path) -> Vec<(String, Vec<Vec<String>>)> {
    fingerprint(path)
}
pub fn names(path: &Path) -> Vec<String> {
    let mut names: Vec<_> = std::fs::read_dir(path)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_str().unwrap().to_owned())
        .collect();
    names.sort();
    names
}
pub fn error(result: Result<BackupReport, BackupError>, expected: BackupError) {
    match result {
        Err(found) => assert_eq!(found, expected, "capability-qualified failure stage"),
        Ok(report) => {
            assert!(
                report
                    .outcomes
                    .iter()
                    .any(|o| o.error.as_ref() == Some(&expected)),
                "explicit per-board failure, not silent skip: {report:?}"
            );
        }
    }
}
