//! Why a login stored no credential (#1839).

use crate::domain::auth::pasted_code::PasteRefusal;
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
    /// The login was started from inside an async runtime, where its
    /// blocking steps cannot run.
    InsideAsyncRuntime,
    /// The provider refused or did not answer the device-code request.
    DeviceCodeRequest(String),
    /// The provider has no OAuth configuration.
    OAuthUnsupported(LoginProvider),
    /// A pasted redirect URL or code was refused.
    Paste(PasteRefusal),
    /// A pasted authorization code was empty once trimmed.
    EmptyAuthorizationCode,
    /// The provider's redirect URI cannot be listened on.
    InvalidRedirect(String),
    /// The provider refused or did not answer the token exchange.
    TokenExchange(String),
    /// The credential could not be stored.
    Store(String),
}
