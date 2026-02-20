// Security sandbox: workspace path validation, dangerous command blocklist, and command allowlist.

use std::path::{Path, PathBuf};

/// Shell metacharacters that indicate command chaining/substitution.
const SHELL_METACHARACTERS: &[&str] = &[";", "&&", "||", "|", "$(", "`", "<(", ">("];

/// Dangerous command patterns that are always blocked regardless of workspace restriction.
/// All patterns MUST be lowercase (compared against lowercased input).
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
    /// Optional command allowlist. When set, only commands whose first token
    /// is in this list are permitted. When `None`, falls back to the denylist.
    pub command_allowlist: Option<Vec<String>>,
    /// Cached canonical workspace path (computed once at construction).
    canonical_workspace: Option<PathBuf>,
}

impl Sandbox {
    /// Create a new sandbox with the given workspace and restriction setting.
    pub fn new(workspace: Option<PathBuf>, restrict_to_workspace: bool) -> Self {
        let canonical_workspace = workspace.as_ref().and_then(|ws| {
            if ws.exists() {
                ws.canonicalize().ok()
            } else {
                Some(ws.clone())
            }
        });
        Self {
            workspace,
            restrict_to_workspace,
            command_allowlist: None,
            canonical_workspace,
        }
    }

    /// Validate that a file path is within the workspace (if restriction is enabled).
    /// Follows symlinks to ensure the resolved real path is inside the workspace.
    pub fn validate_path(&self, path: &str) -> Result<PathBuf, SandboxError> {
        let path = Path::new(path);

        if !self.restrict_to_workspace {
            return Ok(path.to_path_buf());
        }

        let canonical_workspace = self
            .canonical_workspace
            .as_ref()
            .ok_or(SandboxError::NoWorkspace)?;

        // Try to canonicalize the target path to resolve symlinks.
        // If the full path doesn't exist, try canonicalizing the parent
        // (for paths where the file hasn't been created yet).
        let resolved = if path.exists() {
            path.canonicalize()
                .map_err(|e| SandboxError::Io(path.display().to_string(), e))?
        } else if let Some(parent) = path.parent() {
            if parent.exists() {
                let canonical_parent = parent
                    .canonicalize()
                    .map_err(|e| SandboxError::Io(parent.display().to_string(), e))?;
                if let Some(file_name) = path.file_name() {
                    canonical_parent.join(file_name)
                } else {
                    canonical_parent
                }
            } else {
                // Neither path nor parent exists — fall back to textual resolution
                resolve_path(path)
            }
        } else {
            resolve_path(path)
        };

        if resolved.starts_with(canonical_workspace) {
            Ok(resolved)
        } else {
            Err(SandboxError::OutsideWorkspace(
                resolved.display().to_string(),
                canonical_workspace.display().to_string(),
            ))
        }
    }

    /// Validate that a command is permitted.
    ///
    /// The denylist is ALWAYS checked first, regardless of allowlist configuration.
    /// If a command allowlist is configured, the command's first token must also be
    /// in the list, and shell metacharacters are validated.
    ///
    /// If no allowlist is configured, only the denylist is applied.
    pub fn validate_command(&self, command: &str) -> Result<(), SandboxError> {
        // Always check denylist first — dangerous patterns are never permitted
        self.check_denylist(command)?;

        // If allowlist is configured, also validate against it
        if let Some(ref allowlist) = self.command_allowlist {
            return self.validate_command_allowlist(command, allowlist);
        }

        Ok(())
    }

    /// Check command against the dangerous patterns denylist.
    fn check_denylist(&self, command: &str) -> Result<(), SandboxError> {
        let lower = command.to_lowercase();
        for pattern in DANGEROUS_PATTERNS {
            // Patterns are already lowercase, no need to convert them
            if lower.contains(pattern) {
                return Err(SandboxError::DangerousPattern(
                    command.to_string(),
                    pattern.to_string(),
                ));
            }
        }
        Ok(())
    }

    /// Validate a command against the allowlist.
    /// Extracts ALL command tokens from the input (splitting on shell metacharacters)
    /// and ensures every one is in the allowlist.
    fn validate_command_allowlist(
        &self,
        command: &str,
        allowlist: &[String],
    ) -> Result<(), SandboxError> {
        // Check for shell metacharacters that could chain commands
        let has_metachar = SHELL_METACHARACTERS.iter().any(|mc| command.contains(mc));

        if has_metachar {
            // Extract all command tokens by splitting on metacharacters
            let tokens = extract_all_command_tokens(command);
            for token in &tokens {
                if !allowlist.iter().any(|a| a == token) {
                    return Err(SandboxError::NotInAllowlist(
                        command.to_string(),
                        token.to_string(),
                    ));
                }
            }
            return Ok(());
        }

        // No metacharacters — just check the first token
        let first_token = command.split_whitespace().next().unwrap_or("");

        if allowlist.iter().any(|a| a == first_token) {
            Ok(())
        } else {
            Err(SandboxError::NotInAllowlist(
                command.to_string(),
                first_token.to_string(),
            ))
        }
    }
}

/// Extract all command tokens (first words) from a shell command string,
/// splitting on metacharacters like `;`, `|`, `&&`, `||`.
fn extract_all_command_tokens(command: &str) -> Vec<String> {
    // Replace metacharacters with a common separator
    let mut normalized = command.to_string();
    // Order matters: longer patterns first
    for mc in &["&&", "||", "<(", ">(", "$("] {
        normalized = normalized.replace(mc, "\x00");
    }
    for mc in &[";", "|", "`"] {
        normalized = normalized.replace(mc, "\x00");
    }

    normalized
        .split('\x00')
        .filter_map(|segment| {
            let token = segment.split_whitespace().next()?;
            Some(token.to_string())
        })
        .collect()
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
    /// Command is not in the allowlist.
    NotInAllowlist(String, String),
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
            SandboxError::NotInAllowlist(cmd, token) => {
                write!(f, "command '{}': '{}' is not in allowlist", cmd, token)
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
    use tempfile::TempDir;

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

    // --- Sandbox hardening: symlink tests ---

    #[test]
    fn test_symlink_outside_workspace_blocked() {
        let tmp = TempDir::new().unwrap();
        let ws = tmp.path().to_path_buf();
        let sb = Sandbox::new(Some(ws.clone()), true);

        // Create a symlink inside workspace pointing to /etc/passwd
        let link = ws.join("link.txt");
        #[cfg(unix)]
        std::os::unix::fs::symlink("/etc/passwd", &link).unwrap();

        let result = sb.validate_path(link.to_str().unwrap());
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("outside working dir")
        );
    }

    #[test]
    fn test_symlink_inside_workspace_allowed() {
        let tmp = TempDir::new().unwrap();
        let ws = tmp.path().to_path_buf();
        let sb = Sandbox::new(Some(ws.clone()), true);

        // Create a real file and a symlink to it within the workspace
        let real_file = ws.join("real.txt");
        std::fs::write(&real_file, "test").unwrap();
        let link = ws.join("link.txt");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real_file, &link).unwrap();

        let result = sb.validate_path(link.to_str().unwrap());
        assert!(result.is_ok());
    }

    #[test]
    fn test_nested_symlink_chain_blocked() {
        let tmp = TempDir::new().unwrap();
        let ws = tmp.path().to_path_buf();
        let sb = Sandbox::new(Some(ws.clone()), true);

        // Create a symlink to /tmp (outside workspace)
        let step1 = ws.join("step1");
        #[cfg(unix)]
        std::os::unix::fs::symlink("/tmp", &step1).unwrap();

        // Trying to access step1/some-file.txt should be blocked
        let target = ws.join("step1/some-file.txt");
        let result = sb.validate_path(target.to_str().unwrap());
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("outside working dir")
        );
    }

    // --- Sandbox hardening: allowlist tests ---

    #[test]
    fn test_allowlist_permits_listed_command() {
        let mut sb = Sandbox::new(None, false);
        sb.command_allowlist = Some(vec!["echo".to_string(), "ls".to_string()]);
        assert!(sb.validate_command("echo hello").is_ok());
    }

    #[test]
    fn test_allowlist_rejects_unlisted_command() {
        let mut sb = Sandbox::new(None, false);
        sb.command_allowlist = Some(vec!["echo".to_string(), "ls".to_string()]);
        let result = sb.validate_command("curl http://evil.com");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not in allowlist"));
    }

    #[test]
    fn test_allowlist_rejects_semicolon_bypass() {
        let mut sb = Sandbox::new(None, false);
        sb.command_allowlist = Some(vec!["echo".to_string(), "ls".to_string()]);
        let result = sb.validate_command("echo hello; curl evil.com");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not in allowlist"));
    }

    #[test]
    fn test_allowlist_rejects_command_substitution() {
        let mut sb = Sandbox::new(None, false);
        sb.command_allowlist = Some(vec!["echo".to_string(), "ls".to_string()]);
        let result = sb.validate_command("echo $(cat /etc/shadow)");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not in allowlist"));
    }

    #[test]
    fn test_allowlist_rejects_backtick_substitution() {
        let mut sb = Sandbox::new(None, false);
        sb.command_allowlist = Some(vec!["echo".to_string(), "ls".to_string()]);
        let result = sb.validate_command("echo `id`");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not in allowlist"));
    }

    #[test]
    fn test_allowlist_rejects_pipe_to_disallowed() {
        let mut sb = Sandbox::new(None, false);
        sb.command_allowlist = Some(vec!["echo".to_string(), "ls".to_string()]);
        let result = sb.validate_command("ls | bash");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not in allowlist"));
    }

    #[test]
    fn test_empty_allowlist_blocks_all() {
        let mut sb = Sandbox::new(None, false);
        sb.command_allowlist = Some(vec![]);
        let result = sb.validate_command("echo hello");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not in allowlist"));
    }

    #[test]
    fn test_no_allowlist_falls_back_to_denylist() {
        let sb = Sandbox::new(None, false);
        // No allowlist set — should fall back to denylist, allowing safe commands
        assert!(sb.validate_command("echo hello").is_ok());
    }

    #[test]
    fn test_allowlist_still_blocks_dangerous_patterns() {
        // Even with "rm" in the allowlist, dangerous patterns should be blocked
        let mut sb = Sandbox::new(None, false);
        sb.command_allowlist = Some(vec!["rm".to_string(), "echo".to_string()]);
        let result = sb.validate_command("rm -rf /");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("dangerous pattern")
        );
    }

    // --- extract_all_command_tokens tests ---

    #[test]
    fn test_extract_tokens_simple() {
        let tokens = extract_all_command_tokens("echo hello");
        assert_eq!(tokens, vec!["echo"]);
    }

    #[test]
    fn test_extract_tokens_semicolon() {
        let tokens = extract_all_command_tokens("echo hello; curl evil.com");
        assert_eq!(tokens, vec!["echo", "curl"]);
    }

    #[test]
    fn test_extract_tokens_pipe() {
        let tokens = extract_all_command_tokens("ls | bash");
        assert_eq!(tokens, vec!["ls", "bash"]);
    }

    #[test]
    fn test_extract_tokens_command_substitution() {
        let tokens = extract_all_command_tokens("echo $(cat /etc/shadow)");
        assert_eq!(tokens, vec!["echo", "cat"]);
    }

    #[test]
    fn test_extract_tokens_backtick() {
        let tokens = extract_all_command_tokens("echo `id`");
        assert_eq!(tokens, vec!["echo", "id"]);
    }
}
