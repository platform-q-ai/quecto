//! The catalogue snapshot store: the published-generation state every
//! catalogue use case shares, with each source's last-good entries and the
//! models a provider refused for the account in use (#2435).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use crate::application::catalogue::ports::model_refusals::ModelRefusalSink;
use crate::domain::catalogue::entities::entry::CatalogueEntry;
use crate::domain::catalogue::entities::snapshot::CatalogueSnapshot;
use crate::domain::catalogue::value_objects::identity::ModelRef;

/// Atomic holder of the current published snapshot generation, plus the
/// last successfully loaded entries per source so one failing source
/// degrades to its own last-good contribution instead of freezing every
/// other layer's updates (#1581 review — a stale discovery cache must not
/// block a valid `models.json` edit from publishing).
#[derive(Debug, Clone)]
pub struct CatalogueSnapshotStore {
    current: Arc<RwLock<Arc<CatalogueSnapshot>>>,
    /// Last-good raw entries per source id (pre-availability derivation).
    last_good_layers: Arc<Mutex<HashMap<String, Vec<CatalogueEntry>>>>,
    /// Models a provider refused for the account in use (#2435), with the
    /// provider's reason, each held until it expires or is served again.
    refusals: super::refusals::RefusalLedger,
}

impl CatalogueSnapshotStore {
    pub fn new(initial: CatalogueSnapshot) -> Self {
        Self {
            current: Arc::new(RwLock::new(Arc::new(initial))),
            last_good_layers: Arc::new(Mutex::new(HashMap::new())),
            refusals: super::refusals::RefusalLedger::default(),
        }
    }

    /// Hold the provider's refusal of `reference` for the account in use
    /// for `held_for` (#2435). True when it is new: a refusal already held
    /// keeps its first reason and hold.
    pub fn record_refusal(&self, reference: &ModelRef, reason: &str, held_for: Duration) -> bool {
        debug_assert!(!reason.trim().is_empty(), "a refusal says why");
        self.refusals
            .record(reference, reason, Instant::now(), held_for)
    }

    /// Release the refusal of `reference`: its provider served it (#2435).
    /// True when one was held.
    pub fn clear_refusal(&self, reference: &ModelRef) -> bool {
        self.refusals.clear(reference)
    }

    /// The reason the provider refused `reference` for the account in use,
    /// while the refusal is held (#2435).
    pub fn refusal(&self, reference: &ModelRef) -> Option<String> {
        self.refusal_at(reference, Instant::now())
    }

    /// [`Self::refusal`] at `now`.
    pub fn refusal_at(&self, reference: &ModelRef, now: Instant) -> Option<String> {
        self.refusals.reason_at(reference, now)
    }

    /// Remember one source's successfully loaded entries as its retention
    /// fallback for later resolves.
    pub(in crate::application::catalogue) fn remember_layer(
        &self,
        source_id: &str,
        entries: &[CatalogueEntry],
    ) {
        self.last_good_layers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(source_id.to_string(), entries.to_vec());
    }

    /// The last successfully loaded entries for a source, if any resolve on
    /// this store has loaded it.
    pub(in crate::application::catalogue) fn last_good_layer(
        &self,
        source_id: &str,
    ) -> Option<Vec<CatalogueEntry>> {
        self.last_good_layers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(source_id)
            .cloned()
    }

    pub fn empty() -> Self {
        Self::new(CatalogueSnapshot::empty(0))
    }

    /// The current published generation, shared by reference: a read is a
    /// refcount bump, never a deep copy of the catalogue.
    pub fn current(&self) -> Arc<CatalogueSnapshot> {
        // A panic elsewhere must not turn every later catalogue read into a
        // process-killing panic: the snapshot behind a poisoned lock is still
        // a fully published generation (`publish` replaces it wholesale).
        self.current
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Atomically replace the current snapshot. Readers see either the old or
    /// the new generation, never a partial state.
    pub fn publish(&self, snapshot: CatalogueSnapshot) {
        *self
            .current
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Arc::new(snapshot);
    }

    /// Resolve and publish the next generation atomically: the write lock is
    /// held across reading the current generation, building its successor,
    /// and publishing it, so two concurrent resolves can never publish two
    /// different snapshots tagged with the same generation.
    pub fn publish_next<T>(
        &self,
        resolve: impl FnOnce(u64) -> (CatalogueSnapshot, T),
    ) -> (Arc<CatalogueSnapshot>, T) {
        let mut guard = self
            .current
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (snapshot, extra) = resolve(guard.generation() + 1);
        let snapshot = Arc::new(snapshot);
        *guard = snapshot.clone();
        (snapshot, extra)
    }
}

impl ModelRefusalSink for CatalogueSnapshotStore {
    fn record_refusal(&self, reference: &ModelRef, reason: &str, held_for: Duration) -> bool {
        CatalogueSnapshotStore::record_refusal(self, reference, reason, held_for)
    }

    fn clear_refusal(&self, reference: &ModelRef) -> bool {
        CatalogueSnapshotStore::clear_refusal(self, reference)
    }
}

#[cfg(test)]
#[path = "snapshot_store_tests.rs"]
mod tests;
