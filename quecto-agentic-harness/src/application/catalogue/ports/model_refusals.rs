//! The port a provider adapter reports a model refusal through (#2435): a
//! provider that refused a model for the account or auth mode in use says
//! so, and the catalogue stops offering it while the refusal is held; a
//! later reply the provider serves for it releases it.

use std::time::Duration;

use crate::application::catalogue::CatalogueSnapshotStore;
use crate::domain::catalogue::ModelRef;

pub trait ModelRefusalSink: Send + Sync + std::fmt::Debug {
    /// Hold the provider's refusal of `reference` for the account in use,
    /// with its reason, for `held_for`. True when it is new.
    fn record_refusal(&self, reference: &ModelRef, reason: &str, held_for: Duration) -> bool;

    /// Release the refusal of `reference`: the provider served it. True
    /// when one was held.
    fn clear_refusal(&self, reference: &ModelRef) -> bool;
}

impl ModelRefusalSink for CatalogueSnapshotStore {
    fn record_refusal(&self, reference: &ModelRef, reason: &str, held_for: Duration) -> bool {
        CatalogueSnapshotStore::record_refusal(self, reference, reason, held_for)
    }

    fn clear_refusal(&self, reference: &ModelRef) -> bool {
        CatalogueSnapshotStore::clear_refusal(self, reference)
    }
}
