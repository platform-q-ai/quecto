//! What removing a stored credential did (#1841).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Removal {
    /// The provider's credential was removed.
    Removed,
    /// No credential was stored for the provider.
    NotStored,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoveError {
    /// The store could not be changed, in its words.
    Store(String),
}
