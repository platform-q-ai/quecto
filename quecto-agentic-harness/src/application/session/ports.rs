//! Capability-local ports of the session capability (#1960).
//!
//! Session persistence and the context spill store are effects the
//! open/switch/save/clear and recall use cases require; infrastructure
//! implements them over files. Signatures name only domain types.
use std::future::Future;
use std::pin::Pin;

use crate::domain::error::DomainError;
use crate::domain::message::Message;
use crate::domain::session::{Session, SessionSummary, SpillEntries, SpillEntry};
use crate::domain::workflow::WorkflowRunPersisted;

pub type SpillIndexList<'a> =
    Pin<Box<dyn Future<Output = Result<SpillEntries, DomainError>> + Send + 'a>>;
pub type SpillPresence<'a> = Pin<Box<dyn Future<Output = Result<bool, DomainError>> + Send + 'a>>;

/// Port: persistent storage for conversation sessions.
pub trait SessionStore: Send + Sync {
    /// Claim single-writer ownership of `key` before opening or resuming it
    /// for writing (#1460): a key owned by another live process must be
    /// refused HERE, at open time, not only when the first turn is saved —
    /// otherwise a whole paid turn can run before the conflict surfaces.
    /// Default is a no-op for stores without cross-process shared state.
    fn claim(&self, _key: &str) -> Result<(), DomainError> {
        Ok(())
    }

    /// Release this process's ownership claim when a live session switches
    /// away from a key. Stores without explicit ownership can ignore this.
    fn release(&self, _key: &str) {}

    /// Load a session by key. Returns None if no session exists.
    fn load(
        &self,
        key: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Session>, DomainError>> + Send + '_>>;

    /// Save (create or update) a session.
    fn save(
        &self,
        session: &Session,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>>;

    /// Save a session when the caller knows how many messages are already durable.
    fn save_delta<'a>(
        &'a self,
        key: &'a str,
        messages: &'a [Message],
        _previously_persisted: usize,
        workflow_run: Option<WorkflowRunPersisted>,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        let session = Session {
            key: key.to_string(),
            messages: messages.to_vec(),
            workflow_run,
            subagent_roster: Vec::new(),
        };
        Box::pin(async move { self.save(&session).await })
    }

    /// Save a delta when the caller guarantees the durable prefix is unchanged.
    /// Adapters may use this stronger contract to avoid reading that prefix.
    fn save_clean_delta<'a>(
        &'a self,
        key: &'a str,
        messages: &'a [Message],
        previously_persisted: usize,
        workflow_run: Option<WorkflowRunPersisted>,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        self.save_delta(key, messages, previously_persisted, workflow_run)
    }

    /// Check if a session exists.
    fn exists(
        &self,
        key: &str,
    ) -> Pin<Box<dyn Future<Output = Result<bool, DomainError>> + Send + '_>>;

    /// List persisted sessions, newest first when modification times are
    /// available. When `key_prefix` is `Some`, only sessions whose key starts
    /// with it are returned; the caller supplies this policy and the adapter
    /// uses it to skip non-matching files cheaply (without reading/parsing them).
    ///
    /// This is a SUMMARY-ONLY view and is NOT a load guarantee: an
    /// implementation may derive summaries from a lightweight projection of
    /// each session and therefore surface entries whose full bodies are
    /// malformed. A returned [`SessionSummary`] does not guarantee that the
    /// corresponding [`Self::load`] will succeed — callers that open a listed
    /// session must handle a subsequent load failure gracefully.
    fn list(
        &self,
        key_prefix: Option<&str>,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SessionSummary>, DomainError>> + Send + '_>>;
}

/// Port: spill storage used by context pruning and recall().
pub trait ContextSpillStore: Send + Sync {
    fn append(
        &self,
        session_key: &str,
        entry: &SpillEntry,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>>;

    fn recall(
        &self,
        session_key: &str,
        id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<SpillEntry>, DomainError>> + Send + '_>>;

    fn list_entries(&self, session_key: &str) -> SpillIndexList<'_>;

    /// Return whether any spill entry exists without requiring callers to
    /// materialize the complete index. Stores may override this with a cheap
    /// metadata check; the default preserves compatibility for simple stores.
    fn has_entries<'a>(&'a self, session_key: &'a str) -> SpillPresence<'a> {
        Box::pin(async move { Ok(!self.list_entries(session_key).await?.is_empty()) })
    }

    /// Clear all spill entries for a session (e.g. on /reload).
    /// Truncates spill.jsonl to empty so the manifest rebuilds clean.
    fn clear(
        &self,
        session_key: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>>;
}

#[cfg(test)]
#[path = "ports_cov_tests.rs"]
mod cov_tests;
