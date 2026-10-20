//! What importing external credentials did (#1840).

use crate::domain::auth::provider::LoginProvider;

/// One external credential the import took.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedEntry {
    pub provider: LoginProvider,
    /// It had expired and was refreshed before it was stored.
    pub refreshed: bool,
    /// Whether it was stored; why not, in the store's words.
    pub stored: Result<(), String>,
}

/// Why refreshing an expired external credential failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshFailure {
    /// The provider refused or did not answer.
    Rejected(String),
    /// The refresh could not run here, in the mechanism's words.
    Unavailable(String),
    /// The import was started from inside an async runtime.
    InsideAsyncRuntime,
}

/// The import's outcome: each credential taken, in order, and, when a
/// refresh failed, which provider's ended the import there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportReport {
    pub entries: Vec<ImportedEntry>,
    pub aborted: Option<(LoginProvider, RefreshFailure)>,
}

impl ImportReport {
    /// How many credentials were stored.
    pub fn imported(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.stored.is_ok())
            .count()
    }
}

/// Why nothing could be imported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportError {
    /// There is no external store at `path`.
    NotFound { path: String },
    /// The store at `path` could not be read.
    Unreadable { path: String, reason: String },
    /// The store could not be parsed.
    Malformed(String),
}
