//! One lifetime owner for pinned online snapshots, validation, publication and retention.
use super::backup_files::{BackupFileOps, SystemBackupFileOps};
use super::backup_sqlite::{
    BackupSqliteOps, BackupValidationDeadline, SystemBackupSqliteOps, SystemValidationDeadline,
};
use crate::application::project_board::ports::{BoardClock, backup::*};
use crate::infrastructure::time::board_clock::SystemBoardClock;
use std::{path::PathBuf, sync::Arc};
#[cfg(target_os = "linux")]
#[path = "backup_admission.rs"]
mod admission;
#[cfg(target_os = "linux")]
#[path = "backup_run.rs"]
mod run;

type SourceResolver = Arc<dyn Fn(&str) -> PathBuf + Send + Sync>;
/// Runtime ownership exists only where the anchored Linux executor is available.
pub struct SqliteBoardBackup {
    #[cfg(target_os = "linux")]
    resolved: Option<(Option<String>, SourceResolver)>,
    #[cfg(target_os = "linux")]
    source_root: PathBuf,
    #[cfg(target_os = "linux")]
    files: Arc<dyn BackupFileOps>,
    #[cfg(target_os = "linux")]
    sqlite: Arc<dyn BackupSqliteOps>,
    #[cfg(target_os = "linux")]
    validation: Arc<dyn BackupValidationDeadline>,
    #[cfg(target_os = "linux")]
    time: Arc<dyn BoardClock>,
}
impl SqliteBoardBackup {
    /// Composition owns common-file paths; the adapter owns bounded discovery and held FDs.
    pub fn with_resolver(
        _root: PathBuf,
        _single: Option<String>,
        _resolve: SourceResolver,
    ) -> Self {
        #[cfg(target_os = "linux")]
        {
            let mut owner = Self::new(_root);
            owner.resolved = Some((_single, _resolve));
            owner
        }
        #[cfg(not(target_os = "linux"))]
        {
            Self::new(_root)
        }
    }
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
        _source_root: PathBuf,
        _files: Arc<dyn BackupFileOps>,
        _sqlite: Arc<dyn BackupSqliteOps>,
        _validation: Arc<dyn BackupValidationDeadline>,
        _time: Arc<dyn BoardClock>,
    ) -> Self {
        Self {
            #[cfg(target_os = "linux")]
            resolved: None,
            #[cfg(target_os = "linux")]
            source_root: _source_root,
            #[cfg(target_os = "linux")]
            files: _files,
            #[cfg(target_os = "linux")]
            sqlite: _sqlite,
            #[cfg(target_os = "linux")]
            validation: _validation,
            #[cfg(target_os = "linux")]
            time: _time,
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
