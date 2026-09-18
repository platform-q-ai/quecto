//! The durable side of the environment registry (#2024 S4d): the journal
//! the application installs over its store, what its writes come to, and
//! why a ref could not be minted through it. The registry
//! ([`super::environment_registry::EnvironmentRegistry`]) reports
//! through these and reads the journal back only to retry a startup read
//! that failed (round 3 L2, #2033).

use std::sync::Arc;

use super::environment_registry::{EnvironmentRecord, EnvironmentStatus};

/// What a journal write came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JournalWrite {
    /// The record is on file as given.
    Written,
    /// The record on file no longer had the status the write expected
    /// (another session moved it on); nothing was written and `current`
    /// is what stands.
    Superseded { current: EnvironmentStatus },
    /// The journal could not be written; the account is the journal's own.
    Unavailable,
}

/// Why a ref could not be minted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RefAllocationError {
    /// A durable registry's journal could not allocate: the base
    /// directory's registry is unreadable or unwritable, so a ref minted
    /// from memory could collide with one a live session holds there.
    #[error("environment ref could not be allocated from the durable registry: {0}")]
    JournalUnavailable(String),
}

/// A journal's record write: the record, and the status the write expects
/// on file (`None` writes unconditionally).
pub type RecordedFn =
    dyn Fn(&EnvironmentRecord, Option<&EnvironmentStatus>) -> JournalWrite + Send + Sync;

/// The durable side of the registry (#2024 S4d): where refs are allocated
/// and where every committed record and transition is written. Installed
/// by the application over its store; the registry reports, and reads the
/// journal back only through `reload`, to retry a startup read that failed
/// (restore is the application's, at startup).
#[derive(Clone)]
pub struct EnvironmentJournal {
    /// Allocate the next ref number, unique across every session sharing the
    /// base directory. `Err` means the journal could not allocate; the
    /// registry then refuses to mint (review F9, #2033) — a counter minted
    /// from memory could collide with a ref a live session holds.
    pub allocate_ref: Arc<dyn Fn() -> Result<u64, String> + Send + Sync>,
    /// A record was committed or one of its persisted fields changed. With
    /// `expected` the write is compare-and-set: applied only while the
    /// record on file still has that status (review F5, #2033 — a record
    /// another session created is written conditionally, never replaced
    /// whole, so a joiner's inspect cannot revert its creator's `retained`).
    pub recorded: Arc<RecordedFn>,
    /// A record was removed (a rolled-back create).
    pub forgotten: Arc<dyn Fn(&str) + Send + Sync>,
    /// Retry a startup read that failed (round 3 L2, #2033): the store's
    /// records as a restore would seed them, or the store's account of
    /// why it still cannot be read. Called by a registry that carries a
    /// read error, on its next lookup.
    pub reload: Arc<dyn Fn() -> Result<Vec<EnvironmentRecord>, String> + Send + Sync>,
}

impl std::fmt::Debug for EnvironmentJournal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvironmentJournal").finish_non_exhaustive()
    }
}
