//! The texts a session's home offers the metadata query (#2010): the
//! repository label, the execution path, and the path below a local root.
use super::session_home::{SessionHome, SessionHomeScope, WorkspaceGroup};
use super::session_metadata_text::display_path;
use std::path::{Path, PathBuf};

/// The label of the repository or folder a home belongs to: the name of its
/// group's root ([`group_root`]), a bare repository's without `.git`. Related
/// worktrees share one label. `None` for a home without a directory.
pub fn repository_label(home: &SessionHomeScope) -> Option<String> {
    let SessionHomeScope::Scoped(home) = home else {
        return None;
    };
    let name = display_path(Path::new(group_root(home)?.file_name()?));
    Some(name.strip_suffix(".git").unwrap_or(&name).to_string())
}

/// The directory a workspace group is rooted at: the directory holding a
/// `.git` common dir, a bare repository itself, or the folder.
pub fn group_root(home: &SessionHome) -> Option<&Path> {
    match &home.group {
        WorkspaceGroup::Git { common_dir } if common_dir.ends_with(".git") => common_dir.parent(),
        WorkspaceGroup::Git { common_dir } => Some(common_dir.as_path()),
        WorkspaceGroup::Folder { directory } => Some(directory.as_path()),
    }
}

/// The execution directory a home records, spelled by [`display_path`].
pub fn execution_path(home: &SessionHomeScope) -> Option<String> {
    match home {
        SessionHomeScope::Scoped(home) => Some(display_path(&home.execution_dir)),
        SessionHomeScope::LegacyUnscoped | SessionHomeScope::Unavailable(_) => None,
    }
}

/// What `home`'s execution directory does not share with `root`: the path
/// below it, or — for a linked worktree outside it — below their common
/// ancestor. `None` when nothing is left (the root itself) or no directory.
pub(crate) fn path_below(home: &SessionHomeScope, root: &Path) -> Option<String> {
    let SessionHomeScope::Scoped(home) = home else {
        return None;
    };
    let shared = home.execution_dir.components().zip(root.components());
    let shared = shared.take_while(|(a, b)| a == b).count();
    let below: PathBuf = home.execution_dir.components().skip(shared).collect();
    (!below.as_os_str().is_empty()).then(|| display_path(&below))
}
