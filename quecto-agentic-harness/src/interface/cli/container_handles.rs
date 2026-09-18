//! The environment-inventory handles `quecto container ls|kill|gc` hold
//! (#2024 S4d). Declared here as a plain struct of use-case handles;
//! composition (`composition::environments`) fills it over a registry
//! restored from the base directory. The interface never constructs a use
//! case behind it.

use std::sync::Arc;

use crate::application::environments::dto::RestoredRegistry;
use crate::application::environments::use_cases::{
    GcOrphanedEnvironments, KillEnvironment, ListEnvironmentsQuery,
};

pub struct ContainerInventoryHandles {
    /// `container ls`.
    pub list: Arc<ListEnvironmentsQuery>,
    /// `container kill <ref|name>`.
    pub kill: Arc<KillEnvironment>,
    /// `container gc`.
    pub gc: Arc<GcOrphanedEnvironments>,
    /// What restoring the registry found, for the presenter's notes.
    pub restore: RestoredRegistry,
}

impl std::fmt::Debug for ContainerInventoryHandles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContainerInventoryHandles")
            .field("restore", &self.restore)
            .finish_non_exhaustive()
    }
}
