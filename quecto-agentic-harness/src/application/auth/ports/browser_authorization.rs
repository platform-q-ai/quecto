//! Starting a provider's browser OAuth authorization (#1839): the PKCE pair, the
//! state and the URL a user opens in a browser.

use crate::domain::auth::provider::LoginProvider;

/// How the provider hands the authorization code back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorizationReturn {
    /// To a local callback listener, shown to the user as `waiting_on`.
    Loopback { waiting_on: String },
    /// To a provider-hosted page whose code the user pastes.
    PastedCode,
}

/// A started authorization: where the user goes, what a valid return must
/// carry, and the secret PKCE verifier the token exchange proves it with.
#[derive(Clone, PartialEq, Eq)]
pub struct PendingAuthorization {
    pub authorization_url: String,
    pub state: String,
    pub verifier: String,
    pub returns: AuthorizationReturn,
}

impl std::fmt::Debug for PendingAuthorization {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingAuthorization")
            .field("authorization_url", &self.authorization_url)
            .field("state", &self.state)
            .field("verifier", &"[REDACTED]")
            .field("returns", &self.returns)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowserAuthorizationError {
    /// The provider has no OAuth configuration.
    Unsupported,
}

pub trait BrowserAuthorization: Send + Sync {
    /// Start `provider`'s authorization with a fresh PKCE pair and state.
    fn begin(
        &self,
        provider: LoginProvider,
    ) -> Result<PendingAuthorization, BrowserAuthorizationError>;
}
