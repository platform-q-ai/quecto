//! Recall retained context (#1866, D9 #1978): the read and lifecycle side
//! of the sessions capability's retention namespace — the index of a
//! session's retained entries, one entry by id, the clear of a namespace
//! and the run-end scrub of the ephemeral namespace.
//!
//! Selection lives here: `recall("list")` answers the index, any other id
//! answers the entry or [`RecallOutcome::Missing`], an empty id is refused
//! without a lookup. The recall tool (infrastructure) parses its schema,
//! formats these outcomes and keeps its own diagnostics; it selects
//! nothing. Every operation is keyed by the typed [`SessionIdentity`], the
//! same identity the session transactions switch, clear and rewind under:
//! entries retained under one identity are never visible under another.
use std::sync::Arc;

use crate::application::sessions::dto::retained_context::{
    RecallError, RecallOutcome, RecallQuery,
};
use crate::application::sessions::ports::ContextSpillStore;
use crate::domain::error::DomainError;
use crate::domain::session::SpillEntries;
use crate::domain::session_identity::SessionIdentity;

pub struct RecallContext {
    store: Arc<dyn ContextSpillStore>,
}

impl RecallContext {
    pub fn new(store: Arc<dyn ContextSpillStore>) -> Self {
        Self { store }
    }

    /// Answer `query` for the retention namespace of `identity`.
    pub async fn recall(
        &self,
        identity: &SessionIdentity,
        query: &RecallQuery,
    ) -> Result<RecallOutcome, RecallError> {
        match query {
            RecallQuery::Index => Ok(RecallOutcome::Index(self.list(identity).await?)),
            RecallQuery::Entry(id) => match self.store.recall(identity, id).await? {
                Some(entry) => Ok(RecallOutcome::Entry(entry)),
                None => Ok(RecallOutcome::Missing(id.clone())),
            },
        }
    }

    /// The retained-context index of `identity`, oldest first, without
    /// content: one store call, whatever the size of the namespace.
    pub async fn list(&self, identity: &SessionIdentity) -> Result<SpillEntries, DomainError> {
        self.store.list_entries(identity).await
    }

    /// Drop every retained entry of `identity` (the namespace stays
    /// addressable; the next append starts it again).
    pub async fn clear(&self, identity: &SessionIdentity) -> Result<(), DomainError> {
        self.store.clear(identity).await
    }

    /// Remove the ephemeral namespace at the end of a `--no-session` run
    /// (PR #1048 security review): ephemeral runs retain content only so
    /// their own collapse stubs resolve in-run, and it must not outlive the
    /// run. Synchronous and best-effort, for exit paths without a live
    /// runtime. No-op for a `--session` run: `ephemeral` says whether the
    /// run was one.
    pub fn scrub_ephemeral(&self, ephemeral: bool) {
        if ephemeral {
            self.store.scrub_sync(&SessionIdentity::ephemeral());
        }
    }
}

impl std::fmt::Debug for RecallContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecallContext").finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "recall_context_tests.rs"]
mod tests;
