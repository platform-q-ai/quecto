//! Admission-operation composition (#2024 S3): the admission use cases over
//! the authority admin socket and the systemd user-unit service manager.
//! `main` hands [`build_admission_handles`] to the CLI entry point; the
//! interface only invokes the handles it receives.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::application::admission::ports::inference_authority_ports::SharedJournal;
use crate::application::admission::use_cases::{
    AdvanceAuthorityEpoch, BindScope, CompleteAdmission, DisconnectScope, DispatchAdmissions,
    InspectAuthority, InstallAuthorityService, NegotiateAuthority, ResetAuthority,
    RestoreAuthority, SettleAuthority, UninstallAuthorityService,
};
use crate::infrastructure::admission::{
    AuthorityDirectory, AuthorityUseCases, FileJournal, SocketAuthorityAdmin,
};
#[cfg(any(test, feature = "test-support"))]
use crate::infrastructure::admission::{AuthorityServer, ServerError};
use crate::infrastructure::processes::local::SystemdUserServiceManager;
#[cfg(any(test, feature = "test-support"))]
use crate::infrastructure::providers::routing::provider_runtime_admission::AdmissionRuntimeProposal;
use crate::interface::cli::controllers::admission_handles::AdmissionHandles;

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
        authority: build_authority_use_cases,
    }
}

/// The running authority's use cases over the one journal of `dir` (#1910):
/// the durability sequences share it, and settling dispatches through it.
/// The broker's run path receives this builder through the admission
/// handles; the test start functions below pass the same one.
pub fn build_authority_use_cases(dir: &AuthorityDirectory) -> AuthorityUseCases {
    let journal: SharedJournal = Arc::new(Mutex::new(FileJournal::new(dir)));
    AuthorityUseCases {
        restore: RestoreAuthority::new(journal.clone()),
        settle: SettleAuthority::new(DispatchAdmissions::new(journal.clone())),
        complete: CompleteAdmission::new(journal.clone()),
        advance_epoch: AdvanceAuthorityEpoch::new(journal),
        bind: BindScope::new(),
        disconnect: DisconnectScope::new(),
    }
}

// The authority server under test, started as composition composes it
// (#1910): every test that runs a broker starts it through one of these, so
// what the authority is built from is decided in one place. The production
// broker starts through the `admission-broker run` handles, over the same
// builder.

/// Start an authority over `dir`.
#[cfg(any(test, feature = "test-support"))]
pub async fn start_authority(
    dir: AuthorityDirectory,
    proposal: AdmissionRuntimeProposal,
) -> Result<AuthorityServer, ServerError> {
    AuthorityServer::start(dir, proposal, build_authority_use_cases).await
}

/// Start an authority over `dir`, accepting a ledger missing after prior
/// operation (`--accept-missing-ledger`).
#[cfg(any(test, feature = "test-support"))]
pub async fn start_authority_accepting_missing_ledger(
    dir: AuthorityDirectory,
    proposal: AdmissionRuntimeProposal,
) -> Result<AuthorityServer, ServerError> {
    AuthorityServer::start_accepting_missing_ledger(dir, proposal, build_authority_use_cases).await
}

/// Start an authority over `dir` with a framing deadline other than the
/// production one, so stalled-frame tests need not wait it out.
#[cfg(any(test, feature = "test-support"))]
pub async fn start_authority_with_frame_deadline(
    dir: AuthorityDirectory,
    proposal: AdmissionRuntimeProposal,
    frame_deadline: std::time::Duration,
) -> Result<AuthorityServer, ServerError> {
    AuthorityServer::start_with_frame_deadline(
        dir,
        proposal,
        build_authority_use_cases,
        frame_deadline,
    )
    .await
}
