//! Offline board maintenance capabilities, separate from ordinary board operations.
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use std::{path::PathBuf, time::Duration};

/// Production defaults are availability policy; callers may inject short test budgets.
#[derive(Debug, Clone)]
pub struct BackupLimits {
    pub copy: Duration,
    pub busy: Duration,
    pub board: Duration,
    pub invocation: Duration,
    pub entries: usize,
    pub boards: usize,
    pub candidates: usize,
}
impl Default for BackupLimits {
    fn default() -> Self {
        Self {
            copy: Duration::from_secs(120),
            busy: Duration::from_millis(250),
            board: Duration::from_secs(300),
            invocation: Duration::from_secs(1200),
            entries: 4096,
            boards: 256,
            candidates: 256,
        }
    }
}
#[derive(Debug, Clone)]
pub struct BackupRequest {
    pub board: Option<String>,
    pub to: PathBuf,
    pub at: Timestamp,
    pub limits: BackupLimits,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupError {
    Unavailable(String),
    NoTargets,
    UnknownBoard,
    NoBoard,
    IdentityMismatch,
    UnsupportedSchema,
    UnsupportedPlatform,
    UnsafePath,
    ResourceLimit,
    BusyDeadline,
    ValidationDeadline,
    InvocationDeadline,
    ClockRegression,
    ClockOutOfRange,
    Collision,
    IntegrityFailed,
    PublishFailed,
    PublicationUncertain,
    RetentionFailed,
}
#[derive(Debug, Clone)]
pub struct BackupOutcome {
    pub board: String,
    pub published: Option<PathBuf>,
    pub error: Option<BackupError>,
    pub taken_at: Timestamp,
    pub retained: usize,
    pub deleted: usize,
    pub stage: BackupStage,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupStage {
    Unattempted,
    Admission,
    Copy,
    Validation,
    Publication,
    Retention,
    Complete,
}
#[derive(Debug, Clone)]
pub struct BackupReport {
    pub outcomes: Vec<BackupOutcome>,
}
/// A single owner manages source snapshot, SQLite handle and publication lifetime.
pub trait BoardBackup: Send + Sync {
    fn backup(&self, request: &BackupRequest) -> Result<BackupReport, BackupError>;
}
