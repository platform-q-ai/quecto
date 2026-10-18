//! Inspect authentication status (#1842): `auth status` lists each stored
//! credential and whether it is usable now. Whether a credential has
//! expired is the domain's rule (`Credential::state_at`), the same one the
//! provider slots behind the catalogue's `CredentialStatusPort` ask, so the
//! two can never disagree on presence.

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

    /// Every stored credential's status now, ordered by provider. Reading
    /// changes nothing.
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
        statuses.sort_by(|a, b| a.provider.cmp(&b.provider));
        debug_assert!(
            statuses
                .windows(2)
                .all(|pair| pair[0].provider < pair[1].provider),
            "the store holds one credential per provider"
        );
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
