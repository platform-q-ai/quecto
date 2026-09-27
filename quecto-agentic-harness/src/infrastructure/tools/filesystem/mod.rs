// Filesystem tools: read, write, edit, ls.
// Split from the monolithic filesystem.rs (>750 lines) in #137.
// Note: append_file removed in #118 (use write or bash >> instead).

mod edit;
mod edit_diff;
mod edit_indent;
mod edit_match;
mod edit_refusal;
mod fs_failure;
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

/// The largest file `read` reads: a larger one is refused before any offset
/// or limit applies (#2254 review), so a hint to page through a file with
/// read holds only up to this size.
pub const MAX_READ_BYTES: u64 = 10 * 1024 * 1024;

/// `read`'s cap in words, e.g. "10.0MB".
pub fn read_cap_text() -> String {
    crate::infrastructure::tools::truncate::format_size(
        usize::try_from(MAX_READ_BYTES).unwrap_or(usize::MAX),
    )
}

/// Bounded paging of `path` with bash, for a file over `read`'s cap: its
/// first 200 lines and, with `and_tail`, its last 200.
pub fn bash_paging_example(path: &str, and_tail: bool) -> String {
    let quoted = shell_escape_single(path);
    match and_tail {
        true => format!("sed -n '1,200p' {quoted} or tail -n 200 {quoted}"),
        false => format!("sed -n '1,200p' {quoted}"),
    }
}

/// Wrap a path in single quotes for use in a shell command hint.
pub(crate) fn shell_escape_single(path: &str) -> String {
    format!("'{}'", path.replace('\'', "'\\''"))
}
