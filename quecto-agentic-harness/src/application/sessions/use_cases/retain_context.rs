//! Retain context (#1866, D9 #1978): the narrow write and presence handles
//! the context-pruning policy holds on the sessions capability's retention
//! namespace. The policy decides when and what to retain, allocates the
//! id (`turn{n}:{tool}:{idx}` for a tool result, `turn{n}:msg:{role}` for a
//! conversation message) and builds the entry; sessions appends it under
//! the typed identity and issues the receipt — [`Retained`] — only after
//! the append succeeded. The store, its file, its cache and its layout
//! are never reached from the policy.
use std::sync::Arc;

use crate::application::sessions::dto::retained_context::Retained;
use crate::application::sessions::ports::ContextSpillStore;
use crate::domain::error::DomainError;
use crate::domain::session::{SpillEntries, SpillEntry};
use crate::domain::session_identity::SessionIdentity;

/// The retention writer.
pub struct RetainContext {
    store: Arc<dyn ContextSpillStore>,
}

impl RetainContext {
    pub fn new(store: Arc<dyn ContextSpillStore>) -> Self {
        Self { store }
    }

    /// Append `entry` under exactly the id it carries. The receipt is the
    /// same id; an append failure returns the store's error and nothing is
    /// retained.
    pub async fn retain(
        &self,
        identity: &SessionIdentity,
        entry: &SpillEntry,
    ) -> Result<Retained, DomainError> {
        self.store.append(identity, entry).await?;
        Ok(Retained {
            id: entry.id.clone(),
        })
    }

    /// Append `entry` under its id made unique within the namespace: an id
    /// the namespace already holds (as itself or as `{id}:{k}`) is
    /// suffixed `:{k+1}`, `k` being the highest suffix taken (the bare id
    /// counting as 1) — the `[:{k}]` of the conversation grammar. One pass
    /// over the cached index, no rescan. `entry.id` is replaced with the
    /// id actually appended, which the receipt also carries; when the
    /// index cannot be read the id is appended as supplied.
    pub async fn retain_deduplicated(
        &self,
        identity: &SessionIdentity,
        entry: &mut SpillEntry,
    ) -> Result<Retained, DomainError> {
        let existing = self.store.list_entries(identity).await.unwrap_or_default();
        let max_n = highest_suffix_taken(&existing, &entry.id);
        if max_n > 0 {
            entry.id = format!("{}:{}", entry.id, max_n + 1);
        }
        self.retain(identity, entry).await
    }
}

/// The highest `:{k}` suffix of `base` the index holds, the bare `base`
/// counting as 1; 0 when the base is free.
fn highest_suffix_taken(existing: &SpillEntries, base: &str) -> usize {
    let mut max_n = 0usize;
    for e in existing.iter() {
        if e.id == base {
            max_n = max_n.max(1);
        } else if let Some(n) =
            e.id.strip_prefix(base)
                .and_then(|rest| rest.strip_prefix(':'))
                .and_then(|n| n.parse::<usize>().ok())
        {
            max_n = max_n.max(n);
        }
    }
    max_n
}

impl std::fmt::Debug for RetainContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RetainContext").finish_non_exhaustive()
    }
}

/// The retention reader the pruning policy holds: the index and its
/// presence, never an entry's content.
pub struct ListRetainedContext {
    store: Arc<dyn ContextSpillStore>,
}

impl ListRetainedContext {
    pub fn new(store: Arc<dyn ContextSpillStore>) -> Self {
        Self { store }
    }

    /// The index of `identity`, oldest first, without content.
    pub async fn list(&self, identity: &SessionIdentity) -> Result<SpillEntries, DomainError> {
        self.store.list_entries(identity).await
    }

    /// Whether `identity` retains anything, without materialising the
    /// index.
    pub async fn retains_entries(&self, identity: &SessionIdentity) -> Result<bool, DomainError> {
        self.store.has_entries(identity).await
    }
}

impl std::fmt::Debug for ListRetainedContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListRetainedContext")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "retain_context_tests.rs"]
mod tests;
