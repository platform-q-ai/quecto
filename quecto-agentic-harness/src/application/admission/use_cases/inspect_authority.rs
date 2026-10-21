//! Inspect the running authority at a resolved directory (#2024 S3): the
//! `admission-broker status` use case. The directory is resolved by the
//! interface (defaulting to the global config, never the cwd overlay) and
//! passed in, so which broker is addressed never depends on the cwd.

use std::path::Path;
use std::sync::Arc;

use super::install_authority_service::unit_serves_config;
use crate::application::admission::dto::{
    authority::AuthorityAdminError, authority::AuthorityReport, authority::StandIn,
};
use crate::application::admission::ports::{AuthorityAdmin, AuthorityServiceManager, UnitStatus};

pub struct InspectAuthority {
    admin: Arc<dyn AuthorityAdmin>,
    manager: Arc<dyn AuthorityServiceManager>,
}

impl InspectAuthority {
    pub fn new(admin: Arc<dyn AuthorityAdmin>, manager: Arc<dyn AuthorityServiceManager>) -> Self {
        Self { admin, manager }
    }

    pub fn execute(&self, directory: &Path) -> Result<AuthorityReport, AuthorityAdminError> {
        self.admin.inspect(directory)
    }

    /// What an installed service unit has to do with a broker started on
    /// demand serving `report`'s directory (#2615), for the global config
    /// at `config` that `status` addressed. `None` when no unit is
    /// installed, it cannot be read, or the broker serving is not one
    /// started on demand.
    pub fn standing_in(&self, report: &AuthorityReport, config: &Path) -> Option<StandIn> {
        let UnitStatus::Present { contents } = self.manager.unit_status().ok()? else {
            return None;
        };
        if !report.on_demand {
            return None;
        }
        let unit = self.manager.unit_name();
        let serves_this = unit_serves_config(&contents, config);
        Some(match (serves_this, self.manager.is_active()) {
            // A session started first, so the unit lost the singleton lock
            // and, not restarted on a Busy exit, stays stopped.
            (true, Ok(false)) => StandIn::UnitStopped { unit },
            _ => StandIn::UnitElsewhere { unit },
        })
    }
}

impl std::fmt::Debug for InspectAuthority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InspectAuthority").finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "inspect_authority_tests.rs"]
mod tests;
