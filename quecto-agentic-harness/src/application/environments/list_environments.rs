use crate::domain::environment_registry::{EnvironmentRecord, EnvironmentRegistry};

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

    /// Returns a detached snapshot in the registry's existing iteration order.
    pub fn execute(&self) -> Vec<EnvironmentRecord> {
        #[cfg(test)]
        self.executions
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.registry.entries()
    }
}
