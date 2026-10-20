//! The store of provider credentials (#1839): one credential per provider,
//! durable across processes: stored, listed and removed by the auth use
//! cases (#1839, #1841, #1842).

use crate::domain::auth::credential::Credential;

/// What kind of failure the store met, typed where it happened (#2619
/// review), so a caller decides by kind and never by the store's words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialStoreFailure {
    /// The store is there but is not a credential store: it does not parse.
    Malformed,
    /// The store, its lock or its directory refused this user.
    PermissionDenied,
    /// Any other failure to read or write it.
    Unavailable,
}

/// Why the store could not complete an operation: its kind, and the
/// store's words for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialStoreError {
    pub failure: CredentialStoreFailure,
    pub reason: String,
}

impl CredentialStoreError {
    /// A failure of `failure`'s kind, in `reason`'s words.
    pub fn new(failure: CredentialStoreFailure, reason: impl Into<String>) -> Self {
        Self {
            failure,
            reason: reason.into(),
        }
    }
}

impl std::fmt::Display for CredentialStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.reason)
    }
}

pub trait CredentialStorePort: Send + Sync {
    /// Make `credential` its provider's one stored credential, replacing
    /// any before it and leaving every other provider's untouched. On `Ok`
    /// it is durable; on `Err` the store is as it was.
    fn store(&self, credential: Credential) -> Result<(), CredentialStoreError>;

    /// Every stored entry, in no particular order; a hand-edited store may
    /// name a provider twice. Reading writes nothing; an absent store lists
    /// nothing.
    fn list(&self) -> Result<Vec<Credential>, CredentialStoreError>;

    /// Remove `provider`'s stored credential, leaving every other
    /// provider's untouched; whether there was one. Like a store, it is one
    /// read-modify-write under the cross-process credentials lock, so a
    /// concurrent write (a refresh's rotated token) is never lost. When
    /// nothing was stored it still writes: an empty store (with its lock
    /// file) where there was none. On `Err` the store is as it was.
    fn remove(&self, provider: &str) -> Result<bool, CredentialStoreError>;
}
