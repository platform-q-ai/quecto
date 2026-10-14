//! Port: which main worktree, if any, a working directory is a linked git
//! worktree of. Selection asks it only when the working directory has no
//! overlay of its own, so a linked worktree (an untracked overlay is never
//! checked out into one) shares its main worktree's overlay.

use std::path::{Path, PathBuf};

/// How a directory relates to git's worktrees, as far as the overlay is
/// concerned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeLink {
    /// Not the root of a linked worktree: a main worktree, a plain
    /// directory, a subdirectory of either, or a submodule (whose git
    /// directory is its own common directory). Its own overlay location is
    /// the only candidate.
    NotLinked,
    /// The root of a linked worktree whose main worktree, `main_worktree`,
    /// is an existing directory (never a symbolic link), given absolute.
    Linked { main_worktree: PathBuf },
    /// A `.git` pointer is there but does not resolve to a main worktree
    /// (a broken or foreign gitdir, a bare repository, a relocated work
    /// tree). Nothing is shared; `reason` says what could not be resolved.
    Unresolved { reason: String },
}

pub trait MainWorktreeLocator: Send + Sync {
    /// The worktree link of `directory`, read from git's metadata. Never
    /// runs git and never fails: anything it cannot vouch for is
    /// [`WorktreeLink::Unresolved`].
    fn locate(&self, directory: &Path) -> WorktreeLink;
}
