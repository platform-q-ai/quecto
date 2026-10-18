//! Persisting a refreshed OAuth credential: the expiry margin every
//! credential storage path applies and the rotation-aware write shared by
//! the lazy provider refresh and the interface's eager key resolution.

use crate::domain::auth::credential::{AuthMethod, Credential, oauth_expires_at};
use crate::infrastructure::auth::credential_store::CredentialStore;

/// `expires_at` for a token issued now for `expires_in` seconds: the
/// domain's margin rule ([`oauth_expires_at`]) on the system clock, for the
/// refresh paths that persist a refreshed credential.
pub fn expires_at_with_margin(expires_in: u64) -> i64 {
    oauth_expires_at(
        crate::infrastructure::time::unix_timestamp_secs(),
        expires_in,
    )
}

/// Process an OAuth token refresh result: build and persist the new credential.
///
/// Returns `Some(access_token)` on success, `None` on failure (logged as warning).
/// Shared by both sync and async refresh paths to avoid credential-building duplication.
///
/// `previous_refresh_token` is preserved when the server response omits
/// `refresh_token` (valid per RFC 6749 §5.1 — the field is OPTIONAL).
pub fn persist_refreshed_token(
    store: &CredentialStore,
    provider: &str,
    previous_refresh_token: &str,
    refresh_result: Result<
        crate::infrastructure::auth::oauth::OAuthTokenResponse,
        crate::domain::shared::error::DomainError,
    >,
) -> Option<String> {
    match refresh_result {
        Ok(token_resp) => {
            let expires_at = expires_at_with_margin(token_resp.expires_in);
            let account_id = if provider == "openai" {
                crate::infrastructure::auth::oauth::extract_openai_account_id(
                    &token_resp.access_token,
                )
            } else {
                None
            };
            let effective_refresh = token_resp
                .refresh_token
                .unwrap_or_else(|| previous_refresh_token.to_string());
            let new_cred = Credential {
                provider: provider.to_string(),
                token: token_resp.access_token.clone(),
                method: AuthMethod::OAuth,
                expires_at: Some(expires_at),
                refresh_token: Some(effective_refresh),
                account_id,
            };
            // Rotation-aware persist: if another agent process refreshed
            // concurrently (its rotated refresh token is already on disk),
            // keep its credential instead of overwriting it (#1460 review).
            match store.store_refreshed(new_cred, previous_refresh_token) {
                Ok(authoritative) => Some(authoritative.token),
                Err(e) => {
                    tracing::warn!("failed to persist refreshed token for {}: {}", provider, e);
                    Some(token_resp.access_token)
                }
            }
        }
        Err(e) => {
            tracing::warn!("failed to refresh OAuth token for {}: {}", provider, e);
            None
        }
    }
}

#[cfg(test)]
#[path = "token_refresh_tests.rs"]
mod tests;
