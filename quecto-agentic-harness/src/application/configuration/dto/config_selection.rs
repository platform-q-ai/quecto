//! Request and outcome of configuration-file selection (#1966).

use std::path::PathBuf;

/// The candidates one run may load its configuration from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigSelectionRequest {
    /// An explicit `--config` override, taken verbatim when present.
    pub explicit: Option<PathBuf>,
    /// The process working directory; `None` when it is unknown, in which
    /// case nothing local is discovered.
    pub working_directory: Option<PathBuf>,
    /// The global configuration file, which may be absent (defaults apply).
    pub global: PathBuf,
}

/// The one configuration file selected for a run, and where it came from.
/// Exactly one is selected; local and global files are never merged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigSelection {
    /// `--config <path>`.
    Explicit(PathBuf),
    /// `config.json` in the working directory, verified to be a usable
    /// regular file.
    WorkingDirectory(PathBuf),
    /// `<base_dir>/config.json`; may be absent.
    Global(PathBuf),
}

impl ConfigSelection {
    /// Whether the selected file must exist when it is loaded. An explicit
    /// override was asked for by name and a working-directory file was
    /// selected because it was present; a file that has vanished since is
    /// an error, never a quiet fall-through to defaults. Only the global
    /// file may be absent.
    pub fn must_exist(&self) -> bool {
        !matches!(self, Self::Global(_))
    }

    pub fn path(&self) -> &std::path::Path {
        match self {
            Self::Explicit(path) | Self::WorkingDirectory(path) | Self::Global(path) => path,
        }
    }

    pub fn into_path(self) -> PathBuf {
        match self {
            Self::Explicit(path) | Self::WorkingDirectory(path) | Self::Global(path) => path,
        }
    }
}

/// Why a present local `config.json` could not be selected. Only absence
/// falls back to the global file; anything else is reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalConfigRejection {
    /// The entry exists but is not a regular file (a directory, a socket…).
    NotRegularFile,
    /// The entry exists but cannot be read (permissions, a dangling link…).
    Unreadable(String),
}

/// A present-but-unusable local configuration file, named so the user can
/// act on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigSelectionError {
    pub path: PathBuf,
    pub rejection: LocalConfigRejection,
}

impl std::fmt::Display for ConfigSelectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.rejection {
            LocalConfigRejection::NotRegularFile => write!(
                f,
                "local config {} is not a regular file; move it aside or pass --config",
                self.path.display()
            ),
            LocalConfigRejection::Unreadable(reason) => write!(
                f,
                "local config {} cannot be read: {reason}; move or fix it, or pass --config",
                self.path.display()
            ),
        }
    }
}

impl std::error::Error for ConfigSelectionError {}

#[cfg(test)]
#[path = "config_selection_tests.rs"]
mod tests;
