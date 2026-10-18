//! A ref minted for a container create (#2173): its number goes back to
//! the journal unless the create committed a record under it — also when
//! the create is dropped before it ran or while it runs.
use crate::domain::environments::entities::environment_registry::{
    EnvironmentRecord, EnvironmentRegistry,
};

pub(super) struct MintedRef {
    environment_ref: String,
    environments: EnvironmentRegistry,
    committed: bool,
}

impl MintedRef {
    pub(super) fn new(environment_ref: String, environments: EnvironmentRegistry) -> Self {
        Self {
            environment_ref,
            environments,
            committed: false,
        }
    }

    pub(super) fn as_str(&self) -> &str {
        &self.environment_ref
    }

    pub(super) fn registry(&self) -> &EnvironmentRegistry {
        &self.environments
    }

    /// Commit the record the create made; the ref is then the record's.
    pub(super) fn commit(mut self, record: EnvironmentRecord) {
        debug_assert_eq!(record.environment_ref, self.environment_ref);
        self.environments.commit(record);
        self.committed = true;
    }
}

impl Drop for MintedRef {
    fn drop(&mut self) {
        match self.committed {
            true => {}
            // Nothing was recorded under the ref: its number goes back.
            false => self.environments.release_ref(&self.environment_ref),
        }
    }
}

#[cfg(test)]
#[path = "spawn_minted_ref_tests.rs"]
mod tests;
