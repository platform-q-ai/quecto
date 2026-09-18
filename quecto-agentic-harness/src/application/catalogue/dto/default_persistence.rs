//! Where a switched model or effort is recorded as a configured default
//! (#2024 S2): the scope a `persist` names and the receipt of the record.

use std::path::PathBuf;

/// Which configuration layer a default is persisted into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultScope {
    /// The repository overlay (`<cwd>/.quecto/config.json`).
    Local,
    /// The global file (`<base_dir>/config.json`).
    Global,
}

impl DefaultScope {
    /// The wire and CLI spelling of the scope.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Global => "global",
        }
    }
}

/// Where a default landed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedDefault {
    pub scope: DefaultScope,
    pub path: PathBuf,
}
