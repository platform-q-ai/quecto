//! Failure-only borrowed SQLite compilation surface.
use crate::application::project_board::ports::backup::BackupError;
use rusqlite::{Connection, InterruptHandle};
use std::time::Duration;
pub trait BackupSqliteOps: Send + Sync {
    fn copy(
        &self,
        source: &Connection,
        destination: &mut Connection,
        remaining: Duration,
        busy: Duration,
    ) -> Result<(), BackupError>;
    fn integrity(&self, connection: &Connection) -> rusqlite::Result<Vec<String>>;
}
pub struct SystemBackupSqliteOps;
impl BackupSqliteOps for SystemBackupSqliteOps {
    fn copy(
        &self,
        _: &Connection,
        _: &mut Connection,
        _: Duration,
        _: Duration,
    ) -> Result<(), BackupError> {
        Err(BackupError::Unavailable("B13-lite not implemented".into()))
    }
    fn integrity(&self, _: &Connection) -> rusqlite::Result<Vec<String>> {
        Err(rusqlite::Error::InvalidQuery)
    }
}
pub trait ValidationLease: Send {
    fn cancel_and_join(&mut self) -> Result<(), BackupError>;
}
pub trait BackupValidationDeadline: Send + Sync {
    fn arm(
        &self,
        interrupt: InterruptHandle,
        remaining: Duration,
    ) -> Result<Box<dyn ValidationLease>, BackupError>;
}
pub struct SystemValidationDeadline;
impl BackupValidationDeadline for SystemValidationDeadline {
    fn arm(
        &self,
        _: InterruptHandle,
        _: Duration,
    ) -> Result<Box<dyn ValidationLease>, BackupError> {
        Err(BackupError::Unavailable("B13-lite not implemented".into()))
    }
}
