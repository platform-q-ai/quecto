//! The store of provider credentials (#1839): one credential per provider,
//! durable across processes: stored, listed and removed by the auth use
//! cases (#1839, #1841, #1842).

use crate::domain::auth::credential::Credential;

/// Why the store could not complete an operation, in the store's words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialStoreError(pub String);

impl std::fmt::Display for CredentialStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
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
    /// provider's untouched; whether there was one. On `Err` the store is
    /// as it was.
    fn remove(&self, provider: &str) -> Result<bool, CredentialStoreError>;
}
