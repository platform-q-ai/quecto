// Security sandbox: workspace path validation and dangerous command blocklist.

use std::path::{Path, PathBuf};

/// Dangerous command patterns that are always blocked regardless of workspace restriction.
const DANGEROUS_PATTERNS: &[&str] = &[
    "rm -rf /",
    "rm -rf /*",
    "mkfs ",
    "mkfs.",
    "dd if=/dev/zero",
    "dd if=/dev/random",
    "dd if=/dev/urandom",
    "shutdown",
    "reboot",
    "halt",
    "poweroff",
    "init 0",
    "init 6",
    ":(){ :",
    "chmod -R 777 /",
    "chown -R ",
    "> /dev/sda",
    "wget|sh",
    "curl|sh",
];

/// Security sandbox that validates file paths and commands.
#[derive(Debug, Clone)]
pub struct Sandbox {
    /// The workspace directory that tools are restricted to.
    pub workspace: Option<PathBuf>,
    /// Whether to enforce workspace restriction.
    pub restrict_to_workspace: bool,
}

impl Sandbox {
    /// Create a new sandbox with the given workspace and restriction setting.
    pub fn new(workspace: Option<PathBuf>, restrict_to_workspace: bool) -> Self {
        Self {
            workspace,
            restrict_to_workspace,
        }
    }

    /// Validate that a file path is within the workspace (if restriction is enabled).
    pub fn validate_path(&self, path: &str) -> Result<PathBuf, SandboxError> {
        let path = Path::new(path);

        if !self.restrict_to_workspace {
            return Ok(path.to_path_buf());
        }

        let workspace = self.workspace.as_ref().ok_or(SandboxError::NoWorkspace)?;

        // Canonicalize the workspace (it must exist)
        // For paths that don't exist yet, we resolve manually
        let canonical_workspace = if workspace.exists() {
            workspace
                .canonicalize()
                .map_err(|e| SandboxError::Io(workspace.display().to_string(), e))?
        } else {
            workspace.to_path_buf()
        };

        // Resolve the target path: normalize ".." components
        let resolved = resolve_path(path);

        if resolved.starts_with(&canonical_workspace) {
            Ok(resolved)
        } else {
            Err(SandboxError::OutsideWorkspace(
                resolved.display().to_string(),
                canonical_workspace.display().to_string(),
            ))
        }
    }

    /// Validate that a command doesn't match any dangerous patterns.
    pub fn validate_command(&self, command: &str) -> Result<(), SandboxError> {
        let lower = command.to_lowercase();
        for pattern in DANGEROUS_PATTERNS {
            if lower.contains(&pattern.to_lowercase()) {
                return Err(SandboxError::DangerousPattern(
                    command.to_string(),
                    pattern.to_string(),
                ));
            }
        }
        Ok(())
    }
}

/// Resolve a path by normalizing ".." and "." components without requiring the path to exist.
fn resolve_path(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                result.pop();
            }
            std::path::Component::CurDir => {}
            other => result.push(other),
        }
    }
    result
}

#[derive(Debug)]
pub enum SandboxError {
    /// Path is outside the allowed workspace directory.
    OutsideWorkspace(String, String),
    /// Command matches a dangerous pattern.
    DangerousPattern(String, String),
    /// No workspace directory configured.
    NoWorkspace,
    /// I/O error during path resolution.
    Io(String, std::io::Error),
}

impl std::fmt::Display for SandboxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SandboxError::OutsideWorkspace(path, workspace) => {
                write!(f, "path '{}' is outside working dir '{}'", path, workspace)
            }
            SandboxError::DangerousPattern(cmd, pattern) => {
                write!(
                    f,
                    "command '{}' matches dangerous pattern '{}'",
                    cmd, pattern
                )
            }
            SandboxError::NoWorkspace => write!(f, "no workspace directory configured"),
            SandboxError::Io(path, err) => {
                write!(f, "I/O error resolving '{}': {}", path, err)
            }
        }
    }
}

impl std::error::Error for SandboxError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn sandbox(workspace: &str, restrict: bool) -> Sandbox {
        Sandbox::new(Some(PathBuf::from(workspace)), restrict)
    }

    #[test]
    fn test_path_inside_workspace_allowed() {
        let sb = sandbox("/tmp/quecto-test", true);
        assert!(sb.validate_path("/tmp/quecto-test/notes.txt").is_ok());
    }

    #[test]
    fn test_path_outside_workspace_blocked() {
        let sb = sandbox("/tmp/quecto-test", true);
        let result = sb.validate_path("/etc/passwd");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("outside working dir"),
            "error should mention 'outside working dir'"
        );
    }

    #[test]
    fn test_path_traversal_blocked() {
        let sb = sandbox("/tmp/quecto-test", true);
        let result = sb.validate_path("/tmp/quecto-test/../evil.txt");
        assert!(result.is_err());
    }

    #[test]
    fn test_restriction_disabled_allows_any_path() {
        let sb = sandbox("/tmp/quecto-test", false);
        assert!(sb.validate_path("/etc/passwd").is_ok());
        assert!(sb.validate_path("/tmp/anywhere/file.txt").is_ok());
    }

    #[test]
    fn test_dangerous_command_rm_rf() {
        let sb = sandbox("/tmp/quecto-test", false);
        let result = sb.validate_command("rm -rf /");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("dangerous pattern")
        );
    }

    #[test]
    fn test_dangerous_command_mkfs() {
        let sb = sandbox("/tmp/quecto-test", false);
        assert!(sb.validate_command("mkfs /dev/sda").is_err());
    }

    #[test]
    fn test_dangerous_command_dd() {
        let sb = sandbox("/tmp/quecto-test", false);
        assert!(sb.validate_command("dd if=/dev/zero of=/dev/sda").is_err());
    }

    #[test]
    fn test_dangerous_command_shutdown() {
        let sb = sandbox("/tmp/quecto-test", false);
        assert!(sb.validate_command("shutdown -h now").is_err());
    }

    #[test]
    fn test_dangerous_command_reboot() {
        let sb = sandbox("/tmp/quecto-test", false);
        assert!(sb.validate_command("reboot").is_err());
    }

    #[test]
    fn test_dangerous_command_fork_bomb() {
        let sb = sandbox("/tmp/quecto-test", false);
        assert!(sb.validate_command(":(){ :|:& };:").is_err());
    }

    #[test]
    fn test_safe_command_allowed() {
        let sb = sandbox("/tmp/quecto-test", false);
        assert!(sb.validate_command("echo hello").is_ok());
        assert!(sb.validate_command("ls -la").is_ok());
        assert!(sb.validate_command("cat file.txt").is_ok());
    }

    #[test]
    fn test_subdirectory_path_allowed() {
        let sb = sandbox("/tmp/quecto-test", true);
        assert!(
            sb.validate_path("/tmp/quecto-test/sub/deep/file.txt")
                .is_ok()
        );
    }

    #[test]
    fn test_resolve_path_normalizes_dotdot() {
        let resolved = resolve_path(Path::new("/a/b/../c"));
        assert_eq!(resolved, PathBuf::from("/a/c"));
    }

    #[test]
    fn test_resolve_path_normalizes_dot() {
        let resolved = resolve_path(Path::new("/a/./b/./c"));
        assert_eq!(resolved, PathBuf::from("/a/b/c"));
    }
}
