//! Use cases of the admission capability (#2024 S3, #1910). Constructed only
//! by `composition::admission`; the interface holds the operation handles and
//! the authority server holds the running authority's use cases, all
//! injected.

pub mod advance_authority_epoch;
pub mod bind_scope;
pub mod complete_admission;
pub mod disconnect_scope;
pub mod dispatch_admissions;
mod durable_ledger;
pub mod inspect_authority;
pub mod install_authority_service;
pub mod negotiate_authority;
pub mod reset_authority;
pub mod restart_authority;
pub mod restore_authority;
pub mod settle_authority;
pub mod uninstall_authority_service;

pub use advance_authority_epoch::AdvanceAuthorityEpoch;
pub use bind_scope::BindScope;
pub use complete_admission::CompleteAdmission;
pub use disconnect_scope::DisconnectScope;
pub use dispatch_admissions::DispatchAdmissions;
pub use inspect_authority::InspectAuthority;
pub use install_authority_service::InstallAuthorityService;
pub use negotiate_authority::NegotiateAuthority;
pub use reset_authority::ResetAuthority;
pub use restart_authority::RestartAuthority;
pub use restore_authority::RestoreAuthority;
pub use settle_authority::SettleAuthority;
pub use uninstall_authority_service::UninstallAuthorityService;
