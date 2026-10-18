//! Exchanging an authorization code for tokens (#1839).

use crate::domain::auth::provider::LoginProvider;

/// Tokens a provider issued, with the account they belong to when the
/// provider names one (OpenAI's `chatgpt_account_id`).
#[derive(Clone, PartialEq, Eq)]
pub struct IssuedToken {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: u64,
    pub account_id: Option<String>,
}

impl std::fmt::Debug for IssuedToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IssuedToken")
            .field("access_token", &"[REDACTED]")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("expires_in", &self.expires_in)
            .field("account_id", &self.account_id)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenExchangeError {
    /// The exchange could not run at all here (e.g. no thread could be
    /// started, or the caller is itself inside an async runtime).
    Unavailable(String),
    /// The provider refused or did not answer; no response body is quoted.
    Rejected(String),
}

pub trait TokenExchange: Send + Sync {
    /// Exchange `code`, proven with the PKCE `verifier`, for `provider`'s
    /// tokens. Blocking: never call it from inside an async runtime (it
    /// answers `Unavailable` there).
    fn exchange_code(
        &self,
        provider: LoginProvider,
        code: &str,
        verifier: &str,
    ) -> Result<IssuedToken, TokenExchangeError>;
}
