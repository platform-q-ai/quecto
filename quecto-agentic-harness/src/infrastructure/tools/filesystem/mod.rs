// Filesystem tools: read, write, edit, ls.
// Split from the monolithic filesystem.rs (>750 lines) in #137.
// Note: append_file removed in #118 (use write or bash >> instead).

mod edit;
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
