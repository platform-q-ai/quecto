//! The admission-operation handles the CLI holds (#2024 S3). Declared here as
//! a plain struct of use-case handles; composition (`composition::admission`)
//! fills it. The interface never constructs a use case behind it.

use std::sync::Arc;

use crate::application::admission::use_cases::{
    InspectAuthority, InstallAuthorityService, NegotiateAuthority, ResetAuthority,
    UninstallAuthorityService,
};

#[derive(Clone)]
pub struct AdmissionHandles {
    /// `admission-broker status`.
    pub inspect: Arc<InspectAuthority>,
    /// `admission-broker reset`.
    pub reset: Arc<ResetAuthority>,
    /// `admission-broker install-service`.
    pub install: Arc<InstallAuthorityService>,
    /// `admission-broker uninstall-service`.
    pub uninstall: Arc<UninstallAuthorityService>,
    /// Startup negotiation: how a process joins the authority (#2023).
    pub negotiate: Arc<NegotiateAuthority>,
}

impl std::fmt::Debug for AdmissionHandles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdmissionHandles").finish_non_exhaustive()
    }
}
