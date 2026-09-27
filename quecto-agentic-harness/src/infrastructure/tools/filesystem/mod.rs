// Filesystem tools: read, write, edit, ls.
// Split from the monolithic filesystem.rs (>750 lines) in #137.
// Note: append_file removed in #118 (use write or bash >> instead).

mod edit;
mod edit_diff;
mod edit_indent;
mod edit_match;
mod edit_refusal;
mod ls;
mod read;
mod write;

// Re-export all public types so the rest of the codebase is unchanged.
pub use edit::EditTool;
pub use ls::LsTool;
pub use read::ReadTool;
pub use write::WriteTool;

use std::path::{Path, PathBuf};

use crate::domain::error::DomainError;
use crate::infrastructure::security::sandbox::Sandbox;
use crate::infrastructure::tools::path_utils::resolve_to_cwd;

/// Resolve a raw relative path within the workspace and validate with sandbox.
pub(super) fn resolve_and_validate(
    workspace: &Path,
    sandbox: &Sandbox,
    raw_path: &str,
) -> Result<PathBuf, DomainError> {
    let full_path = resolve_to_cwd(raw_path, workspace);
    let full_str = full_path.to_string_lossy().to_string();
    sandbox
        .validate_path(&full_str)
        .map_err(|e| DomainError::Security(e.to_string()))
}

/// Wrap a path in single quotes for use in a shell command hint.
pub(super) fn shell_escape_single(path: &str) -> String {
    format!("'{}'", path.replace('\'', "'\\''"))
}

/// For a path that was not found: the target that does not exist when the
/// path is a symbolic link, following a chain of links and resolving
/// relative targets against each link's own directory (#2166); `None` when
/// the path is not a link or its chain cannot be walked.
pub(super) async fn dangling_link_target(path: &Path) -> Option<PathBuf> {
    let mut link = path.to_path_buf();
    // Bounded: a loop of links is reported by the system as such.
    for _ in 0..32 {
        let target = tokio::fs::read_link(&link).await.ok()?;
        let target = match link.parent() {
            Some(dir) if target.is_relative() => dir.join(target),
            Some(_) | None => target,
        };
        match tokio::fs::symlink_metadata(&target).await {
            Ok(meta) if meta.file_type().is_symlink() => link = target,
            Ok(_) => return None,
            Err(_) => {
                // Shown plainly: its directory exists, so resolve it.
                let plain = match (target.parent(), target.file_name()) {
                    (Some(dir), Some(name)) => tokio::fs::canonicalize(dir)
                        .await
                        .map_or_else(|_| target.clone(), |dir| dir.join(name)),
                    _ => target,
                };
                return Some(plain);
            }
        }
    }
    None
}
