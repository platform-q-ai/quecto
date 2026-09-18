//! Inspect the running authority at a resolved directory (#2024 S3): the
//! `admission-broker status` use case. The directory is resolved by the
//! interface (defaulting to the global config, never the cwd overlay) and
//! passed in, so which broker is addressed never depends on the cwd.

use std::path::Path;
use std::sync::Arc;

use crate::application::admission::dto::{AuthorityAdminError, AuthorityReport};
use crate::application::admission::ports::AuthorityAdmin;

pub struct InspectAuthority {
    admin: Arc<dyn AuthorityAdmin>,
}

impl InspectAuthority {
    pub fn new(admin: Arc<dyn AuthorityAdmin>) -> Self {
        Self { admin }
    }

    pub fn execute(&self, directory: &Path) -> Result<AuthorityReport, AuthorityAdminError> {
        self.admin.inspect(directory)
    }
}

impl std::fmt::Debug for InspectAuthority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InspectAuthority").finish_non_exhaustive()
    }
}
