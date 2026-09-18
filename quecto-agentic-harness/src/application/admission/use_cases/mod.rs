//! Use cases of the admission operation capability (#2024 S3). Constructed
//! only by `composition::admission`; the interface holds injected handles.

pub mod inspect_authority;
pub mod install_authority_service;
pub mod negotiate_authority;
pub mod reset_authority;
pub mod uninstall_authority_service;

pub use inspect_authority::InspectAuthority;
pub use install_authority_service::InstallAuthorityService;
pub use negotiate_authority::NegotiateAuthority;
pub use reset_authority::ResetAuthority;
pub use uninstall_authority_service::UninstallAuthorityService;
