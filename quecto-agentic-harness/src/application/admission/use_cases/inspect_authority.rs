//! Inspect the running authority at a resolved directory (#2024 S3): the
//! `admission-broker status` use case. The directory is resolved by the
//! interface (defaulting to the global config, never the cwd overlay) and
//! passed in, so which broker is addressed never depends on the cwd.

use std::path::Path;
use std::sync::Arc;

use crate::application::admission::dto::{
    authority::AuthorityAdminError, authority::AuthorityReport,
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

    /// The installed service unit a broker started on demand stands in for
    /// (#2615): a session started first, so the unit lost the singleton lock
    /// and, not restarted on a Busy exit, stays stopped until restarted.
    /// `None` when the unit is not installed, cannot be read, or the broker
    /// serving is not one started on demand.
    pub fn stood_down_unit(&self, report: &AuthorityReport) -> Option<String> {
        match (report.on_demand, self.manager.unit_status()) {
            (true, Ok(UnitStatus::Present { .. })) => Some(self.manager.unit_name()),
            _ => None,
        }
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
