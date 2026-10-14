//! Select the configuration layers for a run (#1966, #2024).
//!
//! Precedence: an explicit override replaces everything; otherwise the
//! global file is the base and the working directory's
//! `.quecto/config.json` is the overlay candidate. Only the working
//! directory itself names an overlay — never its parents — with one
//! exception: the root of a linked git worktree that has no overlay of its
//! own shares its main worktree's (the overlay is untracked, so a fresh
//! worktree never has one). Whether the files are usable, and whether the
//! overlay is trusted, is decided when they are loaded, not here.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::application::configuration::dto::config_selection::{
    LEGACY_LOCAL_FILE_NAME, OVERLAY_RELATIVE_PATH,
};
use crate::application::configuration::dto::{
    ConfigLayers, ConfigSelection, ConfigSelectionRequest, OverlayOrigin,
};
use crate::application::configuration::ports::{
    ConfigDocumentStore, MainWorktreeLocator, WorktreeLink,
};

pub struct SelectConfig {
    store: Arc<dyn ConfigDocumentStore>,
    worktrees: Arc<dyn MainWorktreeLocator>,
}

impl SelectConfig {
    pub fn new(
        store: Arc<dyn ConfigDocumentStore>,
        worktrees: Arc<dyn MainWorktreeLocator>,
    ) -> Self {
        Self { store, worktrees }
    }

    pub fn execute(&self, request: ConfigSelectionRequest) -> ConfigSelection {
        if let Some(explicit) = request.explicit {
            return ConfigSelection::Explicit(explicit);
        }
        let working_directory = request.working_directory.as_deref().map(Path::to_path_buf);
        let candidate = |relative: &str| {
            working_directory
                .as_ref()
                .map(|cwd| cwd.join(relative))
                .filter(|candidate| !same_file(candidate, &request.global))
        };
        let (overlay, overlay_origin) = match (&working_directory, candidate(OVERLAY_RELATIVE_PATH))
        {
            (Some(cwd), Some(own)) => self.overlay_for(cwd, own, &request.global),
            (_, own) => (own, OverlayOrigin::WorkingDirectory),
        };
        ConfigSelection::Layered(ConfigLayers {
            overlay,
            overlay_origin,
            legacy_local: candidate(LEGACY_LOCAL_FILE_NAME),
            global: request.global,
        })
    }

    /// The overlay candidate of `cwd`: its own (`own`) whenever an entry is
    /// there; otherwise, for the root of a linked worktree, its main
    /// worktree's when an entry is there. Anything git's metadata does not
    /// vouch for keeps the own location (fail closed).
    fn overlay_for(
        &self,
        cwd: &Path,
        own: PathBuf,
        global: &Path,
    ) -> (Option<PathBuf>, OverlayOrigin) {
        if self.has_entry(&own) {
            return (Some(own), OverlayOrigin::WorkingDirectory);
        }
        match self.worktrees.locate(cwd) {
            WorktreeLink::Linked { main_worktree } => {
                assert!(
                    main_worktree.is_absolute(),
                    "the locator names a main worktree absolutely: {}",
                    main_worktree.display()
                );
                let shared = main_worktree.join(OVERLAY_RELATIVE_PATH);
                if !same_file(&shared, global) && self.has_entry(&shared) {
                    (Some(shared), OverlayOrigin::MainWorktree)
                } else {
                    (Some(own), OverlayOrigin::WorkingDirectory)
                }
            }
            WorktreeLink::NotLinked => (Some(own), OverlayOrigin::WorkingDirectory),
            WorktreeLink::Unresolved { reason } => {
                (Some(own), OverlayOrigin::MainWorktreeUnresolved { reason })
            }
        }
    }

    /// Whether an entry is at the overlay location (never reading it): only
    /// a clean "absent" is no entry. A link, an unreadable file, or one
    /// that cannot be examined is there, and is reported as itself when it
    /// is loaded.
    fn has_entry(&self, overlay: &Path) -> bool {
        match self.store.has_overlay_entry(overlay) {
            Ok(false) => false,
            Ok(true) | Err(_) => true,
        }
    }
}

impl std::fmt::Debug for SelectConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SelectConfig").finish_non_exhaustive()
    }
}

/// Lexical identity after normalising `.` and `..` components: the global
/// file is never its own overlay or its own retired local file.
fn same_file(a: &Path, b: &Path) -> bool {
    normalise(a) == normalise(b)
}

fn normalise(path: &Path) -> std::path::PathBuf {
    use std::path::Component;
    let mut out = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
#[path = "select_config_tests.rs"]
mod tests;
