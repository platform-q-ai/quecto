//! Use cases of the auth capability. Constructed only by
//! `composition::bootstrap::auth`; the interface holds injected handles.

pub mod import_external_credentials;
pub mod inspect_auth_status;
pub mod log_in_with_oauth;
pub mod log_in_with_token;
pub mod remove_stored_credentials;
pub mod start_device_code_login;

pub use import_external_credentials::ImportExternalCredentials;
pub use inspect_auth_status::InspectAuthStatus;
pub use log_in_with_oauth::LogInWithOAuth;
pub use log_in_with_token::LogInWithToken;
pub use remove_stored_credentials::RemoveStoredCredentials;
pub use start_device_code_login::StartDeviceCodeLogin;
