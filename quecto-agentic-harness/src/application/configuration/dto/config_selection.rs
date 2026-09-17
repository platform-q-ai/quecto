//! Request and outcome of configuration-file selection (#1966, #2024).

use std::path::{Path, PathBuf};

/// The repo-local overlay's location relative to the working directory.
pub const OVERLAY_RELATIVE_PATH: &str = ".quecto/config.json";

/// The retired working-directory selection (#1966): a `config.json`
/// directly in the working directory used to *replace* the global file.
/// It is no longer loaded; its presence is reported so the user can move
/// its content into the overlay.
pub const LEGACY_LOCAL_FILE_NAME: &str = "config.json";

/// The candidates one run may load its configuration from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigSelectionRequest {
    /// An explicit `--config` override, taken verbatim when present.
    pub explicit: Option<PathBuf>,
    /// The process working directory; `None` when it is unknown, in which
    /// case no overlay is discovered.
    pub working_directory: Option<PathBuf>,
    /// The global configuration file, which may be absent (defaults apply).
    pub global: PathBuf,
}

/// The layers a run loads its configuration from, and where they came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigSelection {
    /// `--config <path>`: that one file, replacing both layers; it must
    /// exist.
    Explicit(PathBuf),
    /// The global file with the working directory's overlay merged over it.
    Layered(ConfigLayers),
}

/// The two-layer selection: the global file (may be absent) and the
/// overlay candidate the working directory names (may be absent, and is
/// applied only when trusted).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigLayers {
    /// `<base_dir>/config.json`.
    pub global: PathBuf,
    /// `<cwd>/.quecto/config.json`, when the working directory is known.
    pub overlay: Option<PathBuf>,
    /// `<cwd>/config.json`, the retired selection, when the working
    /// directory is known; reported if present, never loaded.
    pub legacy_local: Option<PathBuf>,
}

impl ConfigSelection {
    /// Whether the base file must exist when it is loaded. An explicit
    /// override was asked for by name; a file that has vanished since is
    /// an error, never a quiet fall-through to defaults. Only the global
    /// file may be absent.
    pub fn must_exist(&self) -> bool {
        matches!(self, Self::Explicit(_))
    }

    /// The base file: the explicit selection or the global file. Children
    /// handed one file by path (container spawns) receive this one.
    pub fn path(&self) -> &Path {
        match self {
            Self::Explicit(path) => path,
            Self::Layered(layers) => &layers.global,
        }
    }

    pub fn into_path(self) -> PathBuf {
        match self {
            Self::Explicit(path) => path,
            Self::Layered(layers) => layers.global,
        }
    }

    /// The overlay candidate, when this selection has one.
    pub fn overlay_path(&self) -> Option<&Path> {
        match self {
            Self::Explicit(_) => None,
            Self::Layered(layers) => layers.overlay.as_deref(),
        }
    }
}

#[cfg(test)]
#[path = "config_selection_tests.rs"]
mod tests;
