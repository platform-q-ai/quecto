//! What importing another tool's credentials takes and when it refreshes
//! one first (#1840): credential-lifecycle policy, so the domain owns it.

use crate::domain::auth::credential::Credential;

/// Whether an external entry is a credential to import at all: it carries
/// an access token, or a refresh token to get one with (every login
/// provider refreshes).
pub fn importable(access_token: &str, refresh_token: &str) -> bool {
    !access_token.is_empty() || !refresh_token.is_empty()
}

/// A refresh token as stored: an empty one is none.
pub fn refresh_token(token: String) -> Option<String> {
    Some(token).filter(|token| !token.is_empty())
}

/// Whether an imported credential, `stated` as it would be stored (its
/// expiry already less the margin), is refreshed before it is stored: a
/// refresh token is at hand, and it has no access token or would be
/// stored expired. The expiry comparison is the credential's own
/// (`is_expired_at`), the one rule.
pub fn import_refresh_due(stated: &Credential, now: i64) -> bool {
    stated.refresh_token.is_some() && (stated.token.is_empty() || stated.is_expired_at(now))
}

#[cfg(test)]
#[path = "import_tests.rs"]
mod tests;
