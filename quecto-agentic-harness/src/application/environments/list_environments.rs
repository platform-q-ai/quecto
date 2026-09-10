use crate::domain::environment_registry::{EnvironmentRecord, EnvironmentRegistry};

/// Synchronous, side-effect-free snapshot query over the session inventory.
#[derive(Clone, Debug)]
pub struct ListEnvironmentsQuery {
    registry: EnvironmentRegistry,
}

impl ListEnvironmentsQuery {
    pub fn new(registry: EnvironmentRegistry) -> Self {
        Self { registry }
    }

    /// Returns a detached snapshot in the registry's existing iteration order.
    pub fn execute(&self) -> Vec<EnvironmentRecord> {
        self.registry.entries()
    }
}
