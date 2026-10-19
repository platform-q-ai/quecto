//! Inspect authentication status (#1842): `auth status` lists each stored
//! credential and whether it is usable now. Whether a credential has
//! expired is the domain's rule (`Credential::state_at`); the provider
//! slots behind the catalogue's `CredentialStatusPort` share that expiry
//! rule. They do not share the presence decision: the catalogue still
//! credits an expired OAuth credential it refreshes lazily (#811), refuses
//! an empty token and counts configured API keys, none of which status
//! reports.

use std::sync::Arc;

use crate::application::auth::dto::{AuthStatusError, ProviderAuthStatus};
use crate::application::auth::ports::{AuthClock, CredentialStorePort};

pub struct InspectAuthStatus {
    store: Arc<dyn CredentialStorePort>,
    clock: Arc<dyn AuthClock>,
}

impl InspectAuthStatus {
    pub fn new(store: Arc<dyn CredentialStorePort>, clock: Arc<dyn AuthClock>) -> Self {
        Self { store, clock }
    }

    /// Every stored credential's status now, ordered by provider (then
    /// method and state). Reading
    /// changes nothing. A hand-edited store may name one provider twice
    /// (under two keys); each entry is reported.
    pub fn execute(&self) -> Result<Vec<ProviderAuthStatus>, AuthStatusError> {
        let stored = self
            .store
            .list()
            .map_err(|error| AuthStatusError::Unreadable(error.0))?;
        let now = self.clock.now_unix_secs();
        let mut statuses: Vec<ProviderAuthStatus> = stored
            .into_iter()
            .map(|credential| ProviderAuthStatus {
                state: credential.state_at(now),
                method: credential.method,
                provider: credential.provider,
            })
            .collect();
        // Ties (a provider a hand-edited store names twice) order by method
        // then state, so the listing never depends on the store's order.
        statuses.sort_by(|a, b| {
            (a.provider.as_str(), a.method.as_str(), a.state.as_str()).cmp(&(
                b.provider.as_str(),
                b.method.as_str(),
                b.state.as_str(),
            ))
        });
        Ok(statuses)
    }
}

impl std::fmt::Debug for InspectAuthStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InspectAuthStatus").finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "inspect_auth_status_tests.rs"]
mod tests;
