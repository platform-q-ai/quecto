//! Remove stored provider credentials (#1841): `auth logout --provider P`
//! ends the locally stored access for `P`, leaving every other provider's
//! credential in place. One provider at a time: there is no "remove all".

use std::sync::Arc;

use crate::application::auth::dto::{Removal, RemoveError};
use crate::application::auth::ports::CredentialStorePort;

pub struct RemoveStoredCredentials {
    store: Arc<dyn CredentialStorePort>,
}

impl RemoveStoredCredentials {
    pub fn new(store: Arc<dyn CredentialStorePort>) -> Self {
        Self { store }
    }

    /// Remove `provider`'s stored credential, whatever provider it names:
    /// the store, not the login allowlist, decides what is stored. One not
    /// stored is reported as such, not as an error.
    pub fn execute(&self, provider: &str) -> Result<Removal, RemoveError> {
        match self.store.remove(provider) {
            Ok(true) => Ok(Removal::Removed),
            Ok(false) => Ok(Removal::NotStored),
            Err(error) => Err(RemoveError::Store(error.0)),
        }
    }
}

impl std::fmt::Debug for RemoveStoredCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoveStoredCredentials")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "remove_stored_credentials_tests.rs"]
mod tests;
