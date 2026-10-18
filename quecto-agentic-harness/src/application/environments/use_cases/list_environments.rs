use crate::domain::environments::entities::environment_registry::{
    EnvironmentRecord, EnvironmentRegistry,
};
use crate::domain::environments::services::environment_listing::{
    EnvironmentListing, ListingScope, select_listing,
};

/// Synchronous, side-effect-free snapshot query over the session inventory.
#[derive(Clone, Debug)]
pub struct ListEnvironmentsQuery {
    registry: EnvironmentRegistry,
    #[cfg(test)]
    executions: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl ListEnvironmentsQuery {
    pub fn new(registry: EnvironmentRegistry) -> Self {
        Self {
            registry,
            #[cfg(test)]
            executions: Default::default(),
        }
    }

    #[cfg(test)]
    pub(crate) fn execution_count(&self) -> usize {
        self.executions.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Why the inventory may be incomplete (round 2 F-B, #2033): the
    /// durable store could not be read, so only what this session created
    /// is listed. One line for the caller's diagnostics; none when the
    /// store read cleanly — or reads cleanly now (round 3 L2): asking
    /// retries the read, so a repaired document is seen and seeded.
    pub fn diagnostics(&self) -> Vec<String> {
        self.registry
            .read_error()
            .map(|error| {
                format!(
                    "registry unreadable: {error}; only environments this session created are listed"
                )
            })
            .into_iter()
            .collect()
    }

    /// A bounded listing for a model-facing caller (#2220): the snapshot
    /// selected by [`select_listing`] for this registry's session, with
    /// the inventory's diagnostics.
    pub fn listing(&self, scope: ListingScope, limit: usize) -> EnvironmentListing {
        let mut listing = select_listing(self.execute(), self.registry.session(), scope, limit);
        listing.diagnostics = self.diagnostics();
        listing
    }

    /// Returns a detached snapshot in the registry's existing iteration order.
    pub fn execute(&self) -> Vec<EnvironmentRecord> {
        #[cfg(test)]
        self.executions
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.registry.entries()
    }
}

#[cfg(test)]
#[path = "list_environments_listing_tests.rs"]
mod listing_tests;
