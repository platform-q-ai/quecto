//! The one socket-path validator for every quecto-tui connect (#1460).
//!
//! Both the CLI `--socket` flag and the subagent roster/feed gates must apply
//! the same policy; a private duplicate predicate drifted (it lacked the
//! allowed-roots check), so the policy now lives here alone.

use std::os::unix::fs::FileTypeExt;
use std::path::{Component, Path, PathBuf};

/// Validate that a socket path is under a safe, expected directory.
///
/// Accepts paths under /tmp, $TMPDIR, $XDG_RUNTIME_DIR, or the user's home.
/// Rejects absolute paths under system directories to prevent the TUI from
/// connecting to arbitrary sockets if the agent binary is compromised.
pub(crate) fn validate_socket_path(path: &Path) -> Result<(), String> {
    let path_str = path.to_string_lossy();

    if !path.is_absolute() {
        return Err(format!("socket path is not absolute: {path_str}"));
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(format!("socket path must not contain '..': {path_str}"));
    }

    let metadata = std::fs::symlink_metadata(path)
        .map_err(|e| format!("socket path '{}' is not accessible: {e}", path_str))?;
    if metadata.file_type().is_symlink() {
        return Err(format!("socket path must not be a symlink: {path_str}"));
    }
    if !metadata.file_type().is_socket() {
        return Err(format!("socket path is not a Unix socket: {path_str}"));
    }

    let parent = path
        .parent()
        .ok_or_else(|| format!("socket path has no parent directory: {path_str}"))?;
    let canonical_parent = std::fs::canonicalize(parent).map_err(|e| {
        format!(
            "socket parent '{}' is not accessible: {e}",
            parent.display()
        )
    })?;
    let allowed_roots = canonical_allowed_socket_roots();

    if allowed_roots
        .iter()
        .any(|prefix| canonical_parent.starts_with(prefix))
    {
        return Ok(());
    }

    Err(format!(
        "socket path '{}' is not under an expected directory (/tmp, $TMPDIR, $XDG_RUNTIME_DIR, $HOME)",
        path_str
    ))
}

/// Boolean form of [`validate_socket_path`] for roster/feed gating: a
/// missing or blank path is unusable, everything else applies the shared
/// policy verbatim.
pub(crate) fn usable_socket_path(path: Option<&str>) -> bool {
    path.is_some_and(|p| {
        let p = p.trim();
        !p.is_empty() && validate_socket_path(Path::new(p)).is_ok()
    })
}

pub(crate) fn canonical_allowed_socket_roots() -> Vec<PathBuf> {
    let mut roots = vec![PathBuf::from("/tmp")];
    if let Ok(tmpdir) = std::env::var("TMPDIR") {
        roots.push(PathBuf::from(tmpdir));
    }
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        roots.push(PathBuf::from(xdg));
    }
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(home));
    }
    canonicalize_socket_roots(roots)
}

/// Keep only absolute, canonicalizable roots. Split out from
/// [`canonical_allowed_socket_roots`] so the relative-path rejection can be
/// tested without mutating the process environment (which races with other
/// tests that read `TMPDIR`/`XDG_RUNTIME_DIR` under parallel execution).
pub(crate) fn canonicalize_socket_roots(roots: Vec<PathBuf>) -> Vec<PathBuf> {
    roots
        .into_iter()
        .filter(|root| root.is_absolute())
        .filter_map(|root| std::fs::canonicalize(root).ok())
        .collect()
}
