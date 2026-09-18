//! Reset the running authority at a resolved directory (#2024 S3): the
//! `admission-broker reset` use case. Like inspect, the directory is resolved
//! by the interface and passed in.

use std::path::Path;
use std::sync::Arc;

use crate::application::admission::dto::{AuthorityAdminError, ResetReport};
use crate::application::admission::ports::AuthorityAdmin;

pub struct ResetAuthority {
    admin: Arc<dyn AuthorityAdmin>,
}

impl ResetAuthority {
    pub fn new(admin: Arc<dyn AuthorityAdmin>) -> Self {
        Self { admin }
    }

    pub fn execute(&self, directory: &Path) -> Result<ResetReport, AuthorityAdminError> {
        self.admin.reset(directory)
    }
}

impl std::fmt::Debug for ResetAuthority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResetAuthority").finish_non_exhaustive()
    }
}
