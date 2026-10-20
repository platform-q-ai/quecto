//! What `auth status` reports (#1842): each stored credential's provider,
//! how it was obtained and whether it is usable now. No key material.

use crate::application::auth::ports::CredentialStoreFailure;
use crate::domain::auth::credential::{AuthMethod, CredentialState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderAuthStatus {
    pub provider: String,
    pub method: AuthMethod,
    pub state: CredentialState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthStatusError {
    /// The store could not be read: what kind of failure, typed where it
    /// happened, and the store's words for it.
    Unreadable {
        failure: CredentialStoreFailure,
        reason: String,
    },
}
