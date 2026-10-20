//! Which provider credentials are stored (#1843): presence for `quecto
//! status`, answered by the auth capability's status (#1842). The
//! configuration capability neither reads nor judges credentials itself.

/// One stored credential, as presence: no key material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredCredential {
    pub provider: String,
    /// How it was obtained ("token", "oauth").
    pub method: String,
    /// Whether it is usable now ("active", "expired").
    pub state: String,
}

/// Why the stored credentials cannot be told. Fixed kinds only: the
/// store's own words may quote a hand-edited file's content (#2617), so
/// they never cross this port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredCredentialsUnavailable {
    /// The store exists but does not parse.
    Malformed,
    /// The store may not be read.
    PermissionDenied,
    /// The store could not be read for another reason.
    Unreadable,
}

pub trait StoredCredentialPresence: Send + Sync {
    /// Every stored credential, ordered by provider; or what kind of
    /// failure kept them from being told.
    fn stored(&self) -> Result<Vec<StoredCredential>, StoredCredentialsUnavailable>;
}
