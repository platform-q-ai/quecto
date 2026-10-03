//! The port a provider adapter reports a model refusal through (#2435): a
//! provider that refused a model for the account or auth mode in use says
//! so once, and the catalogue stops offering it for the rest of the process.

use crate::application::catalogue::CatalogueSnapshotStore;
use crate::domain::catalogue::ModelRef;

pub trait ModelRefusalSink: Send + Sync + std::fmt::Debug {
    /// Record that `reference` was refused for the account in use, with the
    /// provider's reason. True when this is its first recorded refusal.
    fn record_refusal(&self, reference: &ModelRef, reason: &str) -> bool;
}

impl ModelRefusalSink for CatalogueSnapshotStore {
    fn record_refusal(&self, reference: &ModelRef, reason: &str) -> bool {
        CatalogueSnapshotStore::record_refusal(self, reference, reason)
    }
}
