//! What `auth status` reports (#1842): each stored credential's provider,
//! how it was obtained and whether it is usable now. No key material.

use crate::domain::auth::credential::{AuthMethod, CredentialState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderAuthStatus {
    pub provider: String,
    pub method: AuthMethod,
    pub state: CredentialState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthStatusError {
    /// The store could not be read, in its words.
    Unreadable(String),
}
