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
/// applied only when trusted). Neither candidate is ever the global file
/// itself (a run started from the base directory's parent would otherwise
/// see its global file as an untrusted overlay of itself).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigLayers {
    /// `<base_dir>/config.json`.
    pub global: PathBuf,
    /// `<cwd>/.quecto/config.json`, when the working directory is known —
    /// or, for a linked git worktree with no overlay of its own, its main
    /// worktree's (see `overlay_origin`).
    pub overlay: Option<PathBuf>,
    /// Where `overlay` was found.
    pub overlay_origin: OverlayOrigin,
    /// `<cwd>/config.json`, the retired selection, when the working
    /// directory is known; reported if present, never loaded.
    pub legacy_local: Option<PathBuf>,
}

/// Where the overlay candidate comes from. A linked git worktree never has
/// the untracked overlay checked out, so one with no overlay of its own
/// shares its main worktree's: that file, judged by that file's trust
/// record, exactly as a run in the main worktree judges it. Nothing is
/// copied and no trust is recorded for the linked worktree.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum OverlayOrigin {
    /// The working directory's own `.quecto/config.json`.
    #[default]
    WorkingDirectory,
    /// `<main worktree>/.quecto/config.json`, shared with the linked
    /// worktree the working directory is the root of. It is read (and
    /// trusted) there and never written from here.
    MainWorktree,
    /// The working directory's own location only: it looks like a linked
    /// worktree, but its main worktree could not be resolved (fail
    /// closed); `reason` says why, for the operator.
    MainWorktreeUnresolved { reason: String },
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

    /// Where the overlay candidate comes from; the working directory when
    /// there is none.
    pub fn overlay_origin(&self) -> &OverlayOrigin {
        const OWN: &OverlayOrigin = &OverlayOrigin::WorkingDirectory;
        match self {
            Self::Explicit(_) => OWN,
            Self::Layered(layers) => &layers.overlay_origin,
        }
    }
}

#[cfg(test)]
#[path = "config_selection_tests.rs"]
mod tests;
