//! Invocation expiry after real publication: remaining selected boards must be
//! explicit unattempted outcomes, and a completed snapshot must not roll back.
use super::fixture::*;
use crate::project_board_history_fixture::writer;
use quecto::application::project_board::ports::{backup::*, *};
use quecto::domain::project_board::value_objects::timestamp::Timestamp;
use quecto::infrastructure::persistence::sqlite::project_board::{
    SqliteProjectBoardStore, backup::SqliteBoardBackup, backup_files::*, backup_sqlite::*,
};
use std::{
    fs::File,
    io,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
struct ElapsedAfterPublication(Arc<AtomicBool>);
impl BoardClock for ElapsedAfterPublication {
    fn now(&self) -> Result<Timestamp, BoardClockError> {
        Ok(Timestamp::parse("2026-10-25T12:00:00Z").unwrap())
    }
    fn elapsed(&self) -> Duration {
        if self.0.load(Ordering::SeqCst) {
            Duration::from_secs(10)
        } else {
            Duration::ZERO
        }
    }
    fn reference(&self) -> Option<Timestamp> {
        panic!("offline backup never asks network time")
    }
    fn reference_status(&self) -> ReferenceStatus {
        ReferenceStatus::NoReference("offline".into())
    }
    fn pause(&self, _: usize) {
        panic!("no CAS retry in backup")
    }
}
struct ExpireOnDurablePublish {
    expired: Arc<AtomicBool>,
    completed: PathBuf,
    real: SystemBackupFileOps,
}
impl BackupFileOps for ExpireOnDurablePublish {
    fn create_partial(&self, directory: &File, name: &str) -> io::Result<File> {
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
        self.real.sync_directory(directory)?;
        if self.completed.is_file() {
            self.expired.store(true, Ordering::SeqCst);
        }
        Ok(())
    }
    fn prune_admitted(&self, directory: &File, candidate: &AdmittedBackupFile) -> io::Result<()> {
        self.real.prune_admitted(directory, candidate)
    }
}
#[test]
fn r5_expiry_after_publication_keeps_success_and_reports_remaining_unattempted() {
    let _deadline = ProbeDeadline::new();
    let f = Database::new();
    let second_path = f.root.join(format!("{B}.sqlite"));
    let second = SqliteProjectBoardStore::open(&second_path, writer()).unwrap();
    second.bootstrap().unwrap();
    let before_b = logical(&second_path);
    let expired = Arc::new(AtomicBool::new(false));
    let files = Arc::new(ExpireOnDurablePublish {
        expired: expired.clone(),
        completed: f.final_path(),
        real: SystemBackupFileOps,
    });
    let adapter = SqliteBoardBackup::with_runtime_and_clock(
        f.root.clone(),
        files,
        Arc::new(SystemBackupSqliteOps),
        Arc::new(SystemValidationDeadline),
        Arc::new(ElapsedAfterPublication(expired.clone())),
    );
    let mut request = f.request();
    request.board = None;
    request.limits.invocation = Duration::from_secs(1);
    let result = adapter.backup(&request);
    assert!(
        result.is_ok(),
        "partial invocation must report both completed and unattempted targets: {result:?}"
    );
    let report = result.unwrap();
    assert!(
        expired.load(Ordering::SeqCst),
        "deadline advances only after actual file+directory publication"
    );
    assert_eq!(
        report
            .outcomes
            .iter()
            .map(|o| o.board.as_str())
            .collect::<Vec<_>>(),
        [A, B]
    );
    assert_eq!(report.outcomes[0].published.as_ref(), Some(&f.final_path()));
    assert!(matches!(
        report.outcomes[0].error,
        None | Some(BackupError::InvocationDeadline)
    ));
    assert_eq!(report.outcomes[1].published, None);
    assert_eq!(
        report.outcomes[1].error,
        Some(BackupError::InvocationDeadline)
    );
    assert_eq!(logical(&second_path), before_b);
    assert!(!f.to.join(format!("{B}-20261025T120000Z.sqlite")).exists());
    integrity(&f.final_path());
}
