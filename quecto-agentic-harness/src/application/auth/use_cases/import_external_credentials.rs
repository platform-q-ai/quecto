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
    AuthMethod, Credential, import_refresh_due, oauth_expires_at, oauth_expires_at_absolute,
};

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

    /// Import every external credential that carries an access token, in
    /// the source's order. An expired one with a refresh token is
    /// refreshed first; a failed refresh ends the import there (what was
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
        // An entry without an access token is no credential to import.
        for found in external
            .into_iter()
            .filter(|found| !found.access_token.is_empty())
        {
            let provider = found.provider;
            let refreshed = import_refresh_due(
                self.clock.now_unix_secs(),
                found.expires_at,
                !found.refresh_token.is_empty(),
            );
            let credential = match refreshed {
                true => match self.refreshed(found) {
                    Ok(credential) => credential,
                    Err(failure) => {
                        report.aborted = Some((provider, failure));
                        return Ok(report);
                    }
                },
                false => as_stated(found),
            };
            debug_assert_eq!(credential.provider, provider.id());
            let stored = self.store.store(credential).map_err(|error| error.0);
            report.entries.push(ImportedEntry {
                provider,
                refreshed,
                stored,
            });
        }
        Ok(report)
    }

    /// `found`'s credential after a refresh, which keeps its refresh token
    /// when the provider issues none.
    fn refreshed(&self, found: ExternalCredential) -> Result<Credential, RefreshFailure> {
        let issued = self
            .exchange
            .refresh(found.provider, &found.refresh_token)
            .map_err(|error| match error {
                TokenExchangeError::Rejected(reason) => RefreshFailure::Rejected(reason),
                TokenExchangeError::Unavailable(reason) => RefreshFailure::Unavailable(reason),
                TokenExchangeError::InsideAsyncRuntime => RefreshFailure::InsideAsyncRuntime,
            })?;
        let refresh_token = issued.refresh_token.unwrap_or(found.refresh_token);
        Ok(Credential {
            provider: found.provider.id().to_string(),
            token: issued.access_token,
            method: AuthMethod::OAuth,
            expires_at: Some(oauth_expires_at(
                self.clock.now_unix_secs(),
                issued.expires_in,
            )),
            refresh_token: Some(refresh_token).filter(|token| !token.is_empty()),
            account_id: issued.account_id,
        })
    }
}

/// `found`'s credential as the external store states it.
fn as_stated(found: ExternalCredential) -> Credential {
    Credential {
        provider: found.provider.id().to_string(),
        token: found.access_token,
        method: AuthMethod::OAuth,
        expires_at: Some(oauth_expires_at_absolute(found.expires_at)),
        refresh_token: Some(found.refresh_token).filter(|token| !token.is_empty()),
        account_id: found.account_id,
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
