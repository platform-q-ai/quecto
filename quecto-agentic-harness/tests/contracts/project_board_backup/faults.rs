//! Deterministic failure injection on the actual runtime filesystem dependency.
//! No fake reports or success paths: every nonselected operation delegates to
//! SystemBackupFileOps; the SQLite adapter still owns the complete operation.
use super::{board_backup::old_snapshot, fixture::*};
use quecto::application::project_board::ports::backup::*;
use quecto::infrastructure::persistence::sqlite::project_board::{
    backup::SqliteBoardBackup, backup_files::*,
};
use std::{
    fs::File,
    io,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    DiskFull,
    CreateError,
    FileSync,
    Publish,
    DirectorySync,
    Prune,
}
struct FailAt {
    stage: Stage,
    calls: Arc<AtomicUsize>,
    real: SystemBackupFileOps,
}
impl FailAt {
    fn fail<T>(&self, errno: i32) -> io::Result<T> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(io::Error::from_raw_os_error(errno))
    }
}
impl BackupFileOps for FailAt {
    fn create_partial(&self, directory: &File, name: &str) -> io::Result<File> {
        match self.stage {
            Stage::DiskFull => self.fail(libc::ENOSPC),
            Stage::CreateError => self.fail(libc::EIO),
            _ => self.real.create_partial(directory, name),
        }
    }
    fn sync_file(&self, file: &File) -> io::Result<()> {
        if self.stage == Stage::FileSync {
            self.fail(libc::EIO)
        } else {
            self.real.sync_file(file)
        }
    }
    fn publish_noreplace(
        &self,
        directory: &File,
        partial: &AdmittedBackupFile,
        final_name: &str,
    ) -> io::Result<()> {
        if self.stage == Stage::Publish {
            self.fail(libc::EIO)
        } else {
            self.real.publish_noreplace(directory, partial, final_name)
        }
    }
    fn sync_directory(&self, directory: &File) -> io::Result<()> {
        if self.stage == Stage::DirectorySync {
            self.fail(libc::EIO)
        } else {
            self.real.sync_directory(directory)
        }
    }
    fn prune_admitted(&self, directory: &File, candidate: &AdmittedBackupFile) -> io::Result<()> {
        if self.stage == Stage::Prune {
            self.fail(libc::EACCES)
        } else {
            self.real.prune_admitted(directory, candidate)
        }
    }
}
fn check(stage: Stage, expected: BackupError, published: bool) {
    let _deadline = ProbeDeadline::new();
    let f = Database::new();
    // Healthy valid prior candidates: enough dates to force real retention.
    let days: Vec<_> = if stage == Stage::Prune {
        (1..=16).collect()
    } else {
        vec![24]
    };
    let old: Vec<_> = days
        .into_iter()
        .map(|day| {
            let path = old_snapshot(&f.to, A, day, 9);
            let bytes = std::fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect();
    let previous_names = names(&f.to);
    let foreign = f.to.join(".foreign.partial");
    std::fs::write(&foreign, b"not invocation-owned").unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let files = Arc::new(FailAt {
        stage,
        calls: calls.clone(),
        real: SystemBackupFileOps,
    });
    let adapter = Reached {
        inner: SqliteBoardBackup::with_files(f.root.clone(), files),
        calls: AtomicUsize::new(0),
    };
    let result = adapter.backup(&f.request());
    adapter.assert_called();
    println!(
        "injected {stage:?} runtime calls={} (baseline shell stops before filesystem stage)",
        calls.load(Ordering::SeqCst)
    );
    for (path, bytes) in &old {
        assert_eq!(
            &std::fs::read(path).unwrap(),
            bytes,
            "preexisting healthy snapshot safe when first prune fails"
        );
    }
    assert_eq!(std::fs::read(&foreign).unwrap(), b"not invocation-owned");
    // This assertion is the RED depth. The subsequent injection/publication/
    // cleanup assertions are permanent delivery obligations, not claimed run.
    error(result.clone(), expected.clone());
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "selected real runtime stage must actually execute once"
    );
    if published {
        let report = result.expect("postpublication failure must retain truthful report");
        let outcome = report.outcomes.iter().find(|o| o.board == A).unwrap();
        assert_eq!(outcome.error.as_ref(), Some(&expected));
        assert_eq!(
            outcome.published.as_ref(),
            Some(&f.final_path()),
            "published=true even though durability/pruning failed"
        );
    } else {
        assert!(
            !f.final_path().exists(),
            "prepublication fault creates no completed name"
        );
    }
    assert!(
        names(&f.to).iter().all(|n| previous_names.contains(n)
            || n == ".foreign.partial"
            || n.ends_with(".lock")
            || (published && n == &f.final_path().file_name().unwrap().to_string_lossy())),
        "no owned partial/sidecar leak, foreign prior entries preserved"
    );
    if published {
        integrity(&f.final_path());
    }
}
#[test]
fn r4_destination_full_preserves_old_and_cleans_owned_partial() {
    check(Stage::DiskFull, BackupError::PublishFailed, false);
}
#[test]
fn r4_destination_create_error_preserves_old_and_cleans_owned_partial() {
    check(Stage::CreateError, BackupError::PublishFailed, false);
}
#[test]
fn r4_file_fsync_failure_never_publishes_or_prunes() {
    check(Stage::FileSync, BackupError::PublishFailed, false);
}
#[test]
fn r4_atomic_publication_failure_never_prunes() {
    check(Stage::Publish, BackupError::PublishFailed, false);
}
#[test]
fn r4_directory_fsync_failure_reports_truthful_publication_uncertainty() {
    check(
        Stage::DirectorySync,
        BackupError::PublicationUncertain,
        true,
    );
}
#[test]
fn r4_first_prune_failure_keeps_published_snapshot_and_old_healthy() {
    check(Stage::Prune, BackupError::RetentionFailed, true);
}
