//! A stored provider credential and its expiry rules (#1839).

/// How the credential was obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMethod {
    Token,
    OAuth,
}

impl AuthMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            AuthMethod::Token => "token",
            AuthMethod::OAuth => "oauth",
        }
    }
}

/// Whether a credential is still usable as stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialState {
    Active,
    Expired,
}

impl CredentialState {
    pub fn as_str(&self) -> &'static str {
        match self {
            CredentialState::Active => "active",
            CredentialState::Expired => "expired",
        }
    }
}

/// Safety margin (seconds) subtracted from an OAuth `expires_in` when its
/// `expires_at` is computed. Compensates for clock skew and network latency
/// so tokens are refreshed before they actually expire on the server side.
pub const OAUTH_EXPIRY_MARGIN_SECS: i64 = 300;

/// The `expires_at` of an OAuth token issued at `now` for `expires_in`
/// seconds: `now + expires_in - OAUTH_EXPIRY_MARGIN_SECS`, the one margin
/// every credential storage path (login, import, refresh) applies.
/// Saturating, so an absurd `expires_in` cannot wrap into the past.
pub fn oauth_expires_at(now: i64, expires_in: u64) -> i64 {
    let lifetime = i64::try_from(expires_in).unwrap_or(i64::MAX);
    now.saturating_add(lifetime)
        .saturating_sub(OAUTH_EXPIRY_MARGIN_SECS)
}

/// The `expires_at` stored for a token whose server-side expiry is
/// `expires_at` (Unix seconds), as an imported credential states it: the
/// same margin earlier as [`oauth_expires_at`] keeps.
pub fn oauth_expires_at_absolute(expires_at: i64) -> i64 {
    expires_at.saturating_sub(OAUTH_EXPIRY_MARGIN_SECS)
}

/// A stored credential for a provider.
///
/// `Debug` is manually implemented to redact the token field, preventing
/// accidental exposure of secrets in debug logs, panic backtraces, or
/// `unwrap()` failure messages.
#[derive(Clone)]
pub struct Credential {
    pub provider: String,
    pub token: String,
    pub method: AuthMethod,
    /// Unix timestamp (seconds) when this credential expires, or None if no expiry.
    pub expires_at: Option<i64>,
    /// Refresh token for OAuth credentials. Used to obtain new access tokens.
    pub refresh_token: Option<String>,
    /// Account ID for OpenAI OAuth (chatgpt_account_id from JWT).
    pub account_id: Option<String>,
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credential")
            .field("provider", &self.provider)
            .field("token", &"[REDACTED]")
            .field("method", &self.method)
            .field("expires_at", &self.expires_at)
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("account_id", &self.account_id)
            .finish()
    }
}

impl Credential {
    /// Whether this credential has expired at `now` (Unix seconds): from
    /// its `expires_at` instant on. A credential without one never expires.
    pub fn is_expired_at(&self, now: i64) -> bool {
        self.expires_at.is_some_and(|expires_at| now >= expires_at)
    }

    /// This credential's state at `now`.
    pub fn state_at(&self, now: i64) -> CredentialState {
        if self.is_expired_at(now) {
            CredentialState::Expired
        } else {
            CredentialState::Active
        }
    }
}

#[cfg(test)]
#[path = "credential_tests.rs"]
mod tests;
