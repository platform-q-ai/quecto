//! A provider's device-code grant (#1839): the code a user enters on
//! another device to authorize a headless login.

use crate::domain::auth::provider::LoginProvider;

/// What a user needs to authorize a device-code login. The grant's secret
/// device code stays with the adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCodeGrant {
    pub verification_uri: String,
    pub user_code: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceCodeError {
    /// The provider offers no device-code flow.
    Unsupported,
    /// The request could not be run at all here (e.g. no thread could be
    /// started, or the caller is itself inside an async runtime).
    Unavailable(String),
    /// The provider refused or did not answer the request; no response
    /// body is ever quoted.
    Request(String),
}

pub trait DeviceCodeFlow: Send + Sync {
    /// Ask `provider` for a device-code grant. Blocking: never call it from
    /// inside an async runtime (it answers `Unavailable` there).
    fn request(&self, provider: LoginProvider) -> Result<DeviceCodeGrant, DeviceCodeError>;
}
