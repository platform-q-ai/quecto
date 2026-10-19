//! A browser OAuth login in progress and its outcome (#1839).

use crate::application::auth::dto::LoginError;
use crate::application::auth::ports::{AuthorizationReturn, PendingAuthorization};
use crate::domain::auth::provider::LoginProvider;

/// A started browser login: what its user is shown and how the code comes
/// back. Only `LogInWithOAuth` starts one, and only it reads the state and
/// verifier the login is bound to.
#[derive(Debug)]
pub struct OAuthLoginAttempt {
    provider: LoginProvider,
    pending: PendingAuthorization,
}

impl OAuthLoginAttempt {
    pub(in crate::application::auth) fn new(
        provider: LoginProvider,
        pending: PendingAuthorization,
    ) -> Self {
        Self { provider, pending }
    }

    pub fn provider(&self) -> LoginProvider {
        self.provider
    }

    /// The URL its user opens to authorize the login.
    pub fn authorization_url(&self) -> &str {
        &self.pending.authorization_url
    }

    pub fn returns(&self) -> &AuthorizationReturn {
        &self.pending.returns
    }

    pub(in crate::application::auth) fn pending(&self) -> &PendingAuthorization {
        &self.pending
    }
}

/// An authorization code the login accepted, from its callback or a paste.
pub struct AuthorizationCode(String);

impl AuthorizationCode {
    pub(in crate::application::auth) fn accepted(code: String) -> Self {
        Self(code)
    }

    pub(in crate::application::auth) fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for AuthorizationCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AuthorizationCode([REDACTED])")
    }
}

/// Why the browser callback brought no code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallbackFailure {
    /// The user may paste the code instead; in the listener's words.
    PasteInstead(String),
    /// The login cannot go on.
    Refused(LoginError),
}

/// A stored browser login.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthLogin {
    pub provider: LoginProvider,
    /// An OpenAI login whose token names no ChatGPT account: stored, but
    /// requests that need the account cannot be routed.
    pub account_id_missing: bool,
}
