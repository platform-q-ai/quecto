//! Admission-operation composition (#2024 S3): the admission use cases over
//! the authority admin socket and the systemd user-unit service manager.
//! `main` hands [`build_admission_handles`] to the CLI entry point; the
//! interface only invokes the handles it receives.

use std::path::PathBuf;
use std::sync::Arc;

use crate::application::admission::use_cases::{
    InspectAuthority, InstallAuthorityService, NegotiateAuthority, ResetAuthority,
    UninstallAuthorityService,
};
use crate::infrastructure::admission::{
    AuthorityDirectory, AuthorityServer, ServerError, SocketAuthorityAdmin,
};
use crate::infrastructure::processes::local::SystemdUserServiceManager;
use crate::infrastructure::providers::routing::provider_runtime_admission::AdmissionRuntimeProposal;
use crate::interface::cli::admission_handles::AdmissionHandles;

pub fn build_admission_handles() -> AdmissionHandles {
    let admin = Arc::new(SocketAuthorityAdmin::new());
    // The user unit directory comes from the process environment. If it cannot
    // be resolved (no $HOME/$XDG_CONFIG_HOME), a manager at a clearly-invalid
    // path surfaces the misconfiguration when install/uninstall runs, rather
    // than failing every admission command at construction.
    let manager = Arc::new(
        SystemdUserServiceManager::for_user_environment().unwrap_or_else(|_| {
            SystemdUserServiceManager::new(PathBuf::from("/nonexistent/systemd/user"))
        }),
    );
    AdmissionHandles {
        inspect: Arc::new(InspectAuthority::new(admin.clone())),
        reset: Arc::new(ResetAuthority::new(admin)),
        install: Arc::new(InstallAuthorityService::new(manager.clone())),
        uninstall: Arc::new(UninstallAuthorityService::new(manager)),
        negotiate: Arc::new(NegotiateAuthority::new()),
    }
}

/// Start an authority over `dir` as composition composes it: every test
/// that runs a broker starts it here, so what the authority is built from is
/// decided in one place (#1910).
pub async fn start_authority(
    dir: AuthorityDirectory,
    proposal: AdmissionRuntimeProposal,
) -> Result<AuthorityServer, ServerError> {
    AuthorityServer::start(dir, proposal).await
}
