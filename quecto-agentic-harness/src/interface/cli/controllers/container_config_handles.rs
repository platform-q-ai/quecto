//! The container-config handles an agent run's tools hold (#2024 S4a,
//! S4c). Declared here as a plain struct of use-case handles; composition
//! (`composition::container_configs`) fills it through the builder `main`
//! hands to the CLI. The interface never constructs the use cases behind
//! it.

use std::sync::Arc;

use crate::application::environments::use_cases::ListContainerConfigs;
use crate::application::subagents::use_cases::SelectContainerConfig;

#[derive(Clone)]
pub struct ContainerConfigHandles {
    /// Launch policy (#2024 S4a): which entry `container: true` or a
    /// named `container_config` launches with, resolved for the launching
    /// agent's checkout.
    pub selection: Arc<SelectContainerConfig>,
    /// Discovery (#2024 S4c): the effective set as agents may list it —
    /// `agent_cmd get_container_configs` and the spawn description's
    /// roster line — over the same layers the selection reads.
    pub roster: Arc<ListContainerConfigs>,
}

impl std::fmt::Debug for ContainerConfigHandles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContainerConfigHandles")
            .field("selection", &self.selection)
            .field("roster", &self.roster)
            .finish()
    }
}
