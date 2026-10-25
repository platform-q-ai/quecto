//! Inward structural history transfer contracts; no artifact interpretation or transport.
use super::BoardHead;
use crate::domain::project_board::entities::claim::Identity;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryAlgorithm {
    Sha1,
    Sha256,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryKind {
    Blob,
    Tree,
    Commit,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryEntry {
    pub mode: String,
    pub name: Vec<u8>,
    pub oid: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryObject {
    pub oid: String,
    pub kind: HistoryKind,
    pub bytes: Vec<u8>,
    pub entries: Vec<HistoryEntry>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryRevision {
    pub sequence: u64,
    pub parent: Option<u64>,
    pub root: String,
    pub writer: Identity,
    pub message: String,
    pub at: i64,
    pub original: Option<HistoryObject>,
}
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BoardHistoryError {
    #[error("history unavailable: {0}")]
    Unavailable(String),
    #[error("malformed history: {0}")]
    Malformed(String),
    #[error("unsupported history: {0}")]
    Unsupported(String),
    #[error("history bounds: {0}")]
    Bounds(String),
    #[error("history conflict: {0}")]
    Conflict(String),
    #[error("history publication unknown: {0}")]
    PublicationUnknown(String),
}
pub use BoardHistoryError as HistoryError;
pub type HistoryResult<T> = Result<T, HistoryError>;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryExportReport {
    pub revisions: u64,
    pub frontier: BoardHead,
    pub git_head: String,
}

/// Structural, pinned view only; artifact interpretation remains the sole Reader.
pub trait BoardHistorySnapshot: Send {
    /// Imported views identify their original Git format. Validated pure-native
    /// framed history and ordinary format-neutral port fakes return None.
    fn algorithm(&self) -> HistoryResult<Option<HistoryAlgorithm>>;
    /// Exact original membership, including empty raw trees and carried original blobs.
    fn is_original(&self, oid: &str) -> HistoryResult<bool>;
    fn frontier(&self) -> HistoryResult<BoardHead>;
    fn next_revision(&self, after: u64) -> HistoryResult<Option<HistoryRevision>>;
    fn object(&self, oid: &str) -> HistoryResult<HistoryObject>;
}
/// Snapshot-only inward capability; no import operation in A.
pub trait BoardHistoryRead: Send + Sync {
    fn snapshot(&self) -> HistoryResult<Box<dyn BoardHistorySnapshot>>;
}
/// Borrowed stored metadata plus translated tree/parent for native continuation.
#[derive(Debug, Clone, Copy)]
pub struct HistoryCommit<'a> {
    pub tree: &'a str,
    pub parent: Option<&'a str>,
    pub writer: &'a Identity,
    pub message: &'a str,
    pub at: i64,
}
/// Git serialization and publication belong outward, never inside the SQL store.
pub trait BoardHistorySink: Send + Sync {
    fn raw(
        &self,
        kind: HistoryKind,
        bytes: &[u8],
        expected_oid: Option<&str>,
        dry_run: bool,
    ) -> HistoryResult<String>;
    fn tree(&self, entries: &[HistoryEntry], dry_run: bool) -> HistoryResult<String>;
    fn commit(&self, revision: HistoryCommit<'_>, dry_run: bool) -> HistoryResult<String>;
    fn publish(&self, expected: Option<&str>, new_head: &str) -> HistoryResult<()>;
    fn finish_dry_run(&self) -> HistoryResult<()>;
}
