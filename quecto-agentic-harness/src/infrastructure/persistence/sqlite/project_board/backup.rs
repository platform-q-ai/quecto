//! One lifetime owner for pinned online snapshots, validation, publication and retention.
use super::backup_files::{BackupFileOps, SystemBackupFileOps};
use super::backup_sqlite::{
    BackupSqliteOps, BackupValidationDeadline, SystemBackupSqliteOps, SystemValidationDeadline,
};
use crate::application::project_board::ports::{BoardClock, backup::*};
use crate::infrastructure::time::board_clock::SystemBoardClock;
use std::{path::PathBuf, sync::Arc};
#[path = "backup_admission.rs"]
mod admission;
#[cfg(target_os = "linux")]
#[path = "backup_run.rs"]
mod run;

pub struct SqliteBoardBackup {
    source_root: PathBuf,
    files: Arc<dyn BackupFileOps>,
    sqlite: Arc<dyn BackupSqliteOps>,
    validation: Arc<dyn BackupValidationDeadline>,
    time: Arc<dyn BoardClock>,
}
impl SqliteBoardBackup {
    pub fn new(source_root: PathBuf) -> Self {
        Self::with_files(source_root, Arc::new(SystemBackupFileOps))
    }
    pub fn with_files(source_root: PathBuf, files: Arc<dyn BackupFileOps>) -> Self {
        Self::with_runtime(
            source_root,
            files,
            Arc::new(SystemBackupSqliteOps),
            Arc::new(SystemValidationDeadline),
        )
    }
    pub fn with_runtime(
        source_root: PathBuf,
        files: Arc<dyn BackupFileOps>,
        sqlite: Arc<dyn BackupSqliteOps>,
        validation: Arc<dyn BackupValidationDeadline>,
    ) -> Self {
        Self::with_runtime_and_clock(
            source_root,
            files,
            sqlite,
            validation,
            Arc::new(SystemBoardClock::default()),
        )
    }
    pub fn with_runtime_and_clock(
        source_root: PathBuf,
        files: Arc<dyn BackupFileOps>,
        sqlite: Arc<dyn BackupSqliteOps>,
        validation: Arc<dyn BackupValidationDeadline>,
        time: Arc<dyn BoardClock>,
    ) -> Self {
        Self {
            source_root,
            files,
            sqlite,
            validation,
            time,
        }
    }
}
impl BoardBackup for SqliteBoardBackup {
    fn backup(&self, request: &BackupRequest) -> Result<BackupReport, BackupError> {
        #[cfg(target_os = "linux")]
        {
            self.run(request)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = request;
            Err(BackupError::UnsupportedPlatform)
        }
    }
}
