//! Real path/identity/lock and report admission contracts, all scratch-only.
use super::{board_backup::old_snapshot, fixture::*};
use crate::project_board_history_fixture::writer;
use quecto::application::project_board::ports::{backup::*, *};
use quecto::composition::bootstrap::board_backup::build_backup_from_ports;
use quecto::domain::project_board::value_objects::timestamp::Timestamp;
use quecto::infrastructure::persistence::sqlite::project_board::{
    SqliteProjectBoardStore, backup::SqliteBoardBackup, backup_files::*,
};
use std::{
    fs::File,
    io,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

pub(super) fn inventory(root: &Path) -> Vec<(PathBuf, Option<Vec<u8>>)> {
    fn visit(root: &Path, here: &Path, result: &mut Vec<(PathBuf, Option<Vec<u8>>)>) {
        for entry in std::fs::read_dir(here).unwrap() {
            let path = entry.unwrap().path();
            let metadata = std::fs::symlink_metadata(&path).unwrap();
            result.push((
                path.strip_prefix(root).unwrap().to_owned(),
                if metadata.is_file() {
                    Some(std::fs::read(&path).unwrap())
                } else {
                    None
                },
            ));
            if metadata.is_dir() {
                visit(root, &path, result);
            }
        }
    }
    let mut result = Vec::new();
    visit(root, root, &mut result);
    result.sort();
    result
}
#[derive(Clone, Copy)]
enum Mutation {
    SourceReplace,
    DirectoryReplace,
    SourceLink,
}
struct MutateAtPartial {
    source: PathBuf,
    to: PathBuf,
    saved: PathBuf,
    mutation: Mutation,
    reached: AtomicBool,
    real: SystemBackupFileOps,
}
impl BackupFileOps for MutateAtPartial {
    fn create_partial(&self, directory: &File, name: &str) -> io::Result<File> {
        self.reached.store(true, Ordering::SeqCst);
        match self.mutation {
            Mutation::SourceReplace => {
                std::fs::rename(&self.source, &self.saved)?;
                std::fs::write(
                    &self.source,
                    b"foreign replacement, never initialize or delete",
                )?;
            }
            Mutation::DirectoryReplace => {
                std::fs::rename(&self.to, &self.saved)?;
                std::fs::create_dir(&self.to)?;
                std::fs::write(
                    self.to.join("foreign"),
                    b"replacement directory must stay untouched",
                )?;
            }
            Mutation::SourceLink => std::fs::hard_link(&self.source, &self.saved)?,
        }
        self.real.create_partial(directory, name)
    }
    fn sync_file(&self, file: &File) -> io::Result<()> {
        self.real.sync_file(file)
    }
    fn publish_noreplace(
        &self,
        directory: &File,
        partial: &AdmittedBackupFile,
        final_name: &str,
    ) -> io::Result<()> {
        self.real.publish_noreplace(directory, partial, final_name)
    }
    fn sync_directory(&self, directory: &File) -> io::Result<()> {
        self.real.sync_directory(directory)
    }
    fn prune_admitted(&self, directory: &File, candidate: &AdmittedBackupFile) -> io::Result<()> {
        self.real.prune_admitted(directory, candidate)
    }
}
fn replacement(mutation: Mutation) {
    let _deadline = ProbeDeadline::new();
    let f = Database::new();
    let files = Arc::new(MutateAtPartial {
        source: f.path.clone(),
        to: f.to.clone(),
        saved: f.dir.path().join("admitted-original"),
        mutation,
        reached: AtomicBool::new(false),
        real: SystemBackupFileOps,
    });
    let adapter = SqliteBoardBackup::with_files(f.root.clone(), files.clone());
    let result = adapter.backup(&f.request());
    error(result, BackupError::UnsafePath);
    assert!(
        files.reached.load(Ordering::SeqCst),
        "mutation after identity admission/pin must actually occur"
    );
    match mutation {
        Mutation::SourceReplace => {
            assert_eq!(
                std::fs::read(&f.path).unwrap(),
                b"foreign replacement, never initialize or delete"
            );
            assert!(files.saved.exists());
        }
        Mutation::DirectoryReplace => {
            assert_eq!(
                std::fs::read(f.to.join("foreign")).unwrap(),
                b"replacement directory must stay untouched"
            );
            assert!(
                names(&files.saved).iter().all(|n| n.ends_with(".lock")),
                "anchored old destination cleanup only"
            );
        }
        Mutation::SourceLink => {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(std::fs::metadata(&f.path).unwrap().nlink(), 2);
        }
    }
    assert!(!f.final_path().exists());
}
#[test]
fn r5_source_identity_replacement_after_pin_refuses_foreign_mutation() {
    replacement(Mutation::SourceReplace);
}
#[test]
fn r5_destination_identity_replacement_refuses_foreign_mutation() {
    replacement(Mutation::DirectoryReplace);
}
#[cfg(unix)]
#[test]
fn r5_source_nlink_change_after_pin_refuses() {
    replacement(Mutation::SourceLink);
}
#[cfg(unix)]
#[test]
fn r5_unsafe_writable_ancestor_is_rejected_without_leaf_changes() {
    use std::os::unix::fs::PermissionsExt;
    let _deadline = ProbeDeadline::new();
    let f = Database::new();
    let unsafe_parent = f.dir.path().join("unsafe");
    std::fs::create_dir(&unsafe_parent).unwrap();
    let leaf = unsafe_parent.join("private");
    std::fs::create_dir(&leaf).unwrap();
    std::fs::set_permissions(&leaf, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::set_permissions(&unsafe_parent, std::fs::Permissions::from_mode(0o777)).unwrap();
    let before = inventory(&unsafe_parent);
    let mut request = f.request();
    request.to = leaf;
    let result = f.adapter().backup(&request);
    assert_eq!(inventory(&unsafe_parent), before);
    std::fs::set_permissions(&unsafe_parent, std::fs::Permissions::from_mode(0o700)).unwrap();
    error(result, BackupError::UnsafePath);
}
#[cfg(unix)]
#[test]
fn r5_real_cooperative_per_board_lock_contention_never_publishes_or_prunes() {
    use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};
    let _deadline = ProbeDeadline::new();
    let f = Database::new();
    let old = old_snapshot(&f.to, A, 24, 9);
    let bytes = std::fs::read(&old).unwrap();
    // Permanent cooperating-producer lock name contract; never unlink a held lock.
    let path = f.to.join(format!(".{A}.backup.lock"));
    let lock = std::fs::OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(&path)
        .unwrap();
    assert_eq!(
        // SAFETY: flock on a descriptor this test opened and still owns.
        unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    let result = f.adapter().backup(&f.request());
    assert_eq!(std::fs::read(&old).unwrap(), bytes);
    assert!(path.exists());
    assert!(!f.final_path().exists());
    drop(lock);
    error(result, BackupError::BusyDeadline);
}
#[test]
fn r5_final_hardlink_candidate_never_overwrites_or_prunes_alias() {
    let _deadline = ProbeDeadline::new();
    let f = Database::new();
    let target = f.dir.path().join("foreign");
    std::fs::write(&target, b"hardlink target preserved").unwrap();
    std::fs::hard_link(&target, f.final_path()).unwrap();
    let result = f.adapter().backup(&f.request());
    assert_eq!(
        std::fs::read(&target).unwrap(),
        b"hardlink target preserved"
    );
    assert_eq!(
        std::fs::read(f.final_path()).unwrap(),
        b"hardlink target preserved"
    );
    error(result, BackupError::UnsafePath);
}
fn prior(sql: &str, expected: BackupError) {
    let _deadline = ProbeDeadline::new();
    let f = Database::new();
    let path = old_snapshot(&f.to, A, 1, 9);
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute_batch(sql).unwrap();
    drop(connection);
    let before = logical(&path);
    let result = f.adapter().backup(&f.request());
    assert_eq!(
        logical(&path),
        before,
        "old matching name is not sufficient deletion authority"
    );
    error(result, expected);
}
#[test]
fn r5_prior_candidate_wrong_board_binding_is_never_pruned() {
    prior(
        "UPDATE history_meta SET board_id='fedcba9876543210'",
        BackupError::IdentityMismatch,
    );
}
#[test]
fn r5_prior_candidate_integrity_failure_is_never_pruned() {
    prior(
        "PRAGMA ignore_check_constraints=ON; INSERT INTO objects(oid,kind,bytes) VALUES ('bad','invalid',X'00')",
        BackupError::IntegrityFailed,
    );
}
#[test]
fn r5_backward_clock_before_existing_healthy_snapshot_never_prunes() {
    let _deadline = ProbeDeadline::new();
    let f = Database::new();
    let path = old_snapshot(&f.to, A, 25, 9);
    let before = std::fs::read(&path).unwrap();
    let mut request = f.request();
    request.at = Timestamp::parse("2026-10-24T23:59:59Z").unwrap();
    let result = f.adapter().backup(&request);
    assert_eq!(std::fs::read(path).unwrap(), before);
    error(result, BackupError::ClockRegression);
}
#[test]
fn r5_invocation_deadline_reports_every_sorted_unattempted_board() {
    let _deadline = ProbeDeadline::new();
    let f = Database::new();
    let second = f.root.join(format!("{B}.sqlite"));
    let store = SqliteProjectBoardStore::open(&second, writer()).unwrap();
    store.bootstrap().unwrap();
    let before_a = logical(&f.path);
    let before_b = logical(&second);
    let mut request = f.request();
    request.board = None;
    request.limits.invocation = Duration::ZERO;
    let result = f.adapter().backup(&request);
    assert!(
        result.is_ok(),
        "deadline must retain selected-board outcomes, not omit unattempted targets: {result:?}"
    );
    let report = result.unwrap();
    assert_eq!(
        report
            .outcomes
            .iter()
            .map(|o| o.board.as_str())
            .collect::<Vec<_>>(),
        [A, B]
    );
    assert!(
        report
            .outcomes
            .iter()
            .all(|o| o.published.is_none() && o.error == Some(BackupError::InvocationDeadline))
    );
    assert_eq!(logical(&f.path), before_a);
    assert_eq!(logical(&second), before_b);
    assert!(!f.final_path().exists());
}
struct InvalidClock(AtomicUsize);
impl BoardClock for InvalidClock {
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn now(&self) -> Result<Timestamp, BoardClockError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(BoardClockError::OutOfRange("synthetic before epoch".into()))
    }
    fn reference(&self) -> Option<Timestamp> {
        panic!("offline backup must not read network reference time")
    }
    fn reference_status(&self) -> ReferenceStatus {
        ReferenceStatus::NoReference("offline".into())
    }
    fn pause(&self, _: usize) {
        panic!("no retry for invalid clock")
    }
}
#[test]
fn r5_out_of_range_injected_clock_refuses_before_backup_io() {
    let _deadline = ProbeDeadline::new();
    let f = Database::new();
    let before = logical(&f.path);
    let clock = Arc::new(InvalidClock(AtomicUsize::new(0)));
    let adapter = Arc::new(f.adapter());
    let use_case = build_backup_from_ports(adapter.clone(), clock.clone());
    let result = use_case.execute(&f.request());
    assert_eq!(logical(&f.path), before);
    assert!(names(&f.to).is_empty());
    error(result, BackupError::ClockOutOfRange);
    assert_eq!(clock.0.load(Ordering::SeqCst), 1);
    assert_eq!(
        adapter.calls.load(Ordering::SeqCst),
        0,
        "invalid wall time before any adapter I/O"
    );
}

#[test]
fn r5_unknown_requested_board_preserves_existing_root_and_destination() {
    let _deadline = ProbeDeadline::new();
    let f = Database::new();
    f.assert_wal();
    std::fs::write(
        f.to.join("foreign-sentinel"),
        b"destination is not selected for mutation",
    )
    .unwrap();
    let missing = f.root.join(format!("{B}.sqlite"));
    assert!(!missing.exists());
    let source_view = logical(&f.path);
    let source_files = inventory(&f.root);
    let destination_files = inventory(&f.to);
    let mut request = f.request();
    request.board = Some(B.into());
    let adapter = f.adapter();
    let result = adapter.backup(&request);
    adapter.assert_called();
    assert_eq!(
        inventory(&f.root),
        source_files,
        "unknown requested ID must not open/create a source, native state, WAL or sidecar"
    );
    assert_eq!(
        inventory(&f.to),
        destination_files,
        "refusal before destination lock, partial, publication or pruning"
    );
    assert!(!missing.exists());
    assert_eq!(logical(&f.path), source_view);
    error(result, BackupError::UnknownBoard);
}
