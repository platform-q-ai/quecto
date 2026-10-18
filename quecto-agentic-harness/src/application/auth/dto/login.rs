//! Why a login stored no credential (#1839).

use crate::domain::auth::provider::LoginProvider;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginError {
    /// A provider outside the login allowlist, as named.
    UnknownProvider(String),
    /// The token was empty once trimmed.
    EmptyToken,
    /// The provider offers no device-code flow.
    DeviceCodeUnsupported(LoginProvider),
    /// The login could not run at all here (e.g. no thread could be
    /// started); in the mechanism's words.
    Unavailable(String),
    /// The provider refused or did not answer the device-code request.
    DeviceCodeRequest(String),
    /// The credential could not be stored.
    Store(String),
}
