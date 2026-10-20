//! Import existing provider credentials (#1840): `auth login
//! --import-external` takes the OAuth credentials another tool stored and
//! stores them as this harness's, refreshing any that has expired, rather
//! than signing in afresh.

use std::sync::Arc;

use crate::application::auth::dto::{ImportError, ImportReport, ImportedEntry, RefreshFailure};
use crate::application::auth::ports::{
    AuthClock, CredentialStorePort, ExternalCredential, ExternalCredentialSource,
    ExternalCredentialsError, TokenExchange, TokenExchangeError,
};
use crate::domain::auth::credential::{
    AuthMethod, Credential, oauth_expires_at, oauth_expires_at_absolute,
};
use crate::domain::auth::import::{import_refresh_due, importable, refresh_token};

pub struct ImportExternalCredentials {
    source: Arc<dyn ExternalCredentialSource>,
    exchange: Arc<dyn TokenExchange>,
    store: Arc<dyn CredentialStorePort>,
    clock: Arc<dyn AuthClock>,
}

impl ImportExternalCredentials {
    pub fn new(
        source: Arc<dyn ExternalCredentialSource>,
        exchange: Arc<dyn TokenExchange>,
        store: Arc<dyn CredentialStorePort>,
        clock: Arc<dyn AuthClock>,
    ) -> Self {
        Self {
            source,
            exchange,
            store,
            clock,
        }
    }

    /// Import every importable external credential, in the source's order.
    /// One with a refresh token is refreshed first when it has no access
    /// token or would be stored expired; a failed refresh ends the import there (what was
    /// stored stays stored). A failed store is reported and the import
    /// goes on.
    pub fn execute(&self) -> Result<ImportReport, ImportError> {
        let external = self.source.load().map_err(|error| match error {
            ExternalCredentialsError::NotFound { path } => ImportError::NotFound { path },
            ExternalCredentialsError::Unreadable { path, reason } => {
                ImportError::Unreadable { path, reason }
            }
            ExternalCredentialsError::Malformed(reason) => ImportError::Malformed(reason),
        })?;
        let mut report = ImportReport {
            entries: Vec::new(),
            aborted: None,
        };
        for found in external
            .into_iter()
            .filter(|found| importable(&found.access_token, &found.refresh_token))
        {
            let provider = found.provider;
            let stated = as_stated(&found);
            let refreshed = import_refresh_due(&stated, self.clock.now_unix_secs());
            debug_assert!(
                refreshed || !stated.token.is_empty(),
                "a credential without an access token is refreshed, never stored empty"
            );
            let credential = match refreshed {
                true => match self.refreshed(found) {
                    Ok(credential) => credential,
                    Err(failure) => {
                        report.aborted = Some((provider, failure));
                        return Ok(report);
                    }
                },
                false => stated,
            };
            debug_assert_eq!(credential.provider, provider.id());
            let stored = self.store.store(credential).map_err(|error| error.reason);
            report.entries.push(ImportedEntry {
                provider,
                refreshed,
                stored,
            });
        }
        Ok(report)
    }

    /// `found`'s credential after a refresh, which keeps its refresh token
    /// when the provider issues none (or an empty one).
    fn refreshed(&self, found: ExternalCredential) -> Result<Credential, RefreshFailure> {
        let issued = self
            .exchange
            .refresh(found.provider, &found.refresh_token)
            .map_err(|error| match error {
                TokenExchangeError::Rejected(reason) => RefreshFailure::Rejected(reason),
                TokenExchangeError::Unavailable(reason) => RefreshFailure::Unavailable(reason),
                TokenExchangeError::InsideAsyncRuntime => RefreshFailure::InsideAsyncRuntime,
            })?;
        let kept = issued
            .refresh_token
            .and_then(refresh_token)
            .unwrap_or(found.refresh_token);
        Ok(Credential {
            provider: found.provider.id().to_string(),
            token: issued.access_token,
            method: AuthMethod::OAuth,
            expires_at: Some(oauth_expires_at(
                self.clock.now_unix_secs(),
                issued.expires_in,
            )),
            refresh_token: refresh_token(kept),
            account_id: issued.account_id,
        })
    }
}

/// `found`'s credential as the external store states it, as it would be
/// stored: its expiry less the margin.
fn as_stated(found: &ExternalCredential) -> Credential {
    Credential {
        provider: found.provider.id().to_string(),
        token: found.access_token.clone(),
        method: AuthMethod::OAuth,
        expires_at: Some(oauth_expires_at_absolute(found.stated_expiry_secs)),
        refresh_token: refresh_token(found.refresh_token.clone()),
        account_id: found.account_id.clone(),
    }
}

impl std::fmt::Debug for ImportExternalCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImportExternalCredentials")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "import_external_credentials_tests.rs"]
mod tests;
