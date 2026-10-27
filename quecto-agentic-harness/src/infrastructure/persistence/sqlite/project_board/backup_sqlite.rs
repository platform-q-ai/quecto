//! Borrowed SQLite operations; the owning adapter holds the pinned transaction.
use crate::application::project_board::ports::backup::BackupError;
use rusqlite::{Connection, InterruptHandle};
use std::{
    sync::mpsc,
    thread::JoinHandle,
    time::{Duration, Instant},
};

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
        source: &Connection,
        destination: &mut Connection,
        remaining: Duration,
        busy: Duration,
    ) -> Result<(), BackupError> {
        let start = Instant::now();
        source
            .busy_timeout(busy.min(remaining))
            .map_err(copy_error)?;
        destination
            .busy_timeout(busy.min(remaining))
            .map_err(copy_error)?;
        let backup = rusqlite::backup::Backup::new(source, destination).map_err(copy_error)?;
        loop {
            if start.elapsed() < remaining {
            } else {
                return Err(BackupError::BusyDeadline);
            }
            match backup.step(128).map_err(copy_error)? {
                rusqlite::backup::StepResult::Done => return Ok(()),
                rusqlite::backup::StepResult::More => {}
                rusqlite::backup::StepResult::Busy | rusqlite::backup::StepResult::Locked => {
                    std::thread::sleep(
                        Duration::from_millis(1).min(remaining.saturating_sub(start.elapsed())),
                    );
                }
                _ => return Err(BackupError::PublishFailed),
            }
        }
    }
    fn integrity(&self, connection: &Connection) -> rusqlite::Result<Vec<String>> {
        connection
            .prepare("PRAGMA integrity_check")?
            .query_map([], |row| row.get(0))?
            .collect()
    }
}
fn copy_error(error: rusqlite::Error) -> BackupError {
    match error.sqlite_error_code() {
        Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked) => {
            BackupError::BusyDeadline
        }
        Some(rusqlite::ErrorCode::OperationInterrupted) => BackupError::ValidationDeadline,
        Some(rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase) => {
            BackupError::IntegrityFailed
        }
        _ => BackupError::PublishFailed,
    }
}
/// Switches a freshly copied snapshot to rollback-journal mode. The copied
/// header carries the source's WAL flag; reading first lets the pager see it so
/// the switch rewrites the header and checkpoints away the copy's own `-wal`.
#[cfg(target_os = "linux")]
pub(super) fn rollback_journal(connection: &Connection) -> Result<(), BackupError> {
    connection
        .query_row("SELECT count(*) FROM sqlite_schema", [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(copy_error)?;
    let mode: String = connection
        .query_row("PRAGMA journal_mode=DELETE", [], |row| row.get(0))
        .map_err(copy_error)?;
    if mode.eq_ignore_ascii_case("delete") {
        Ok(())
    } else {
        Err(BackupError::PublishFailed)
    }
}
#[cfg(target_os = "linux")]
pub(super) fn classify(error: rusqlite::Error) -> BackupError {
    match error.sqlite_error_code() {
        Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked) => {
            BackupError::BusyDeadline
        }
        Some(rusqlite::ErrorCode::OperationInterrupted) => BackupError::ValidationDeadline,
        _ => BackupError::IntegrityFailed,
    }
}
/// Cancellation must join before the connection can be released, including unwind.
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
struct Lease {
    cancel: Option<mpsc::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}
impl ValidationLease for Lease {
    fn cancel_and_join(&mut self) -> Result<(), BackupError> {
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.send(());
        }
        match self.thread.take() {
            Some(thread) => thread
                .join()
                .map_err(|_| BackupError::Unavailable("validation watchdog failed".into())),
            None => Ok(()),
        }
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        let _ = self.cancel_and_join();
    }
}
impl BackupValidationDeadline for SystemValidationDeadline {
    fn arm(
        &self,
        interrupt: InterruptHandle,
        remaining: Duration,
    ) -> Result<Box<dyn ValidationLease>, BackupError> {
        let (cancel, receive) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("board-backup-validation".into())
            .spawn(move || {
                if matches!(
                    receive.recv_timeout(remaining),
                    Err(mpsc::RecvTimeoutError::Timeout)
                ) {
                    interrupt.interrupt();
                }
            })
            .map_err(|_| BackupError::Unavailable("validation watchdog unavailable".into()))?;
        Ok(Box::new(Lease {
            cancel: Some(cancel),
            thread: Some(thread),
        }))
    }
}

#[cfg(test)]
#[path = "backup_sqlite_tests.rs"]
mod tests;
