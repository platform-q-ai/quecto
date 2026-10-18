//! The store of provider credentials (#1839): one credential per provider,
//! durable across processes. Later use cases add the reads and removals
//! they need (#1840, #1841).

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

    /// Every stored credential, each provider's one, in no particular
    /// order. Reading writes nothing; an absent store lists nothing.
    fn list(&self) -> Result<Vec<Credential>, CredentialStoreError>;
}
