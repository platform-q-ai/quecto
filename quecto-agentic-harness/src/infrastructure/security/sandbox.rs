// Shared path hook plus the dangerous-command denylist.
//
// This is NOT OS isolation. `validate_command` is a best-effort tripwire that
// runs in front of the bash tool; it recognises shell structure well enough to
// tell an executed `reboot` from the word "reboot" inside an argument, and it
// falls back to a conservative whole-string scan when it meets syntax it cannot
// resolve. Untrusted deployments must rely on the container runtime for actual
// process, filesystem and network isolation.

use std::path::{Path, PathBuf};

use super::denylist;

/// Shared path-policy hook plus the dangerous-command denylist.
#[derive(Debug, Clone)]
pub struct Sandbox {
    /// The workspace directory used as the default working directory for tools.
    pub workspace: Option<PathBuf>,
}

impl Sandbox {
    /// Create command/path policy with the given workspace.
    pub fn new(workspace: Option<PathBuf>) -> Self {
        Self { workspace }
    }

    /// Build command/path policy for an agent/repl entry point from the parsed config.
    ///
    /// The legacy filesystem sandbox mode and the per-command allowlist have
    /// both been removed; paths are no longer rejected for being outside the
    /// workspace and only the denylist applies. The `command_allowlist` config
    /// key is still accepted for compatibility but ignored.
    pub fn for_agent_workspace(
        config: &crate::infrastructure::config::Config,
        workspace: PathBuf,
    ) -> Self {
        if config
            .agents
            .defaults
            ._deprecated_command_allowlist
            .is_some()
        {
            tracing::warn!(
                "agents.defaults.command_allowlist is deprecated and ignored (#1620); \
                 command policy is denylist-only. Use the container runtime for isolation."
            );
        }
        Self::new(Some(workspace))
    }

    /// Validate or normalize a file path.
    ///
    /// Filesystem workspace confinement has been removed. This shared hook now
    /// accepts paths without rejecting absolute, home-relative, or parent paths.
    pub fn validate_path(&self, path: &str) -> Result<PathBuf, SandboxError> {
        Ok(Path::new(path).to_path_buf())
    }

    /// Validate that a command is permitted by the dangerous-command denylist.
    ///
    /// Rules are matched against the execution site (program word, arguments,
    /// redirects) of each simple command, including commands reached through
    /// substitutions, wrappers such as `sudo`/`env`/`xargs`, and nested shells
    /// such as `bash -c` or `eval`. Quoted prose, filenames and heredoc bodies
    /// are not executable and do not match.
    ///
    /// Syntax the parser cannot resolve — a `$var` in command position, an
    /// unbalanced quote — triggers an explicit fallback to the pre-#1620
    /// whole-string substring scan, so dynamic constructs are never quietly
    /// waved through.
    pub fn validate_command(&self, command: &str) -> Result<(), SandboxError> {
        denylist::check(command).map_err(|v| SandboxError::DangerousPattern {
            command: command.to_string(),
            rule: v.rule,
            site: v.site,
        })
    }
}

#[derive(Debug)]
pub enum SandboxError {
    /// Command matches a dangerous pattern.
    DangerousPattern {
        /// The full command as submitted.
        command: String,
        /// Rule identifier, or the legacy pattern when the fallback scan matched.
        rule: String,
        /// The simple command the rule matched at, or a fallback-scan note.
        site: String,
    },
}

impl std::fmt::Display for SandboxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SandboxError::DangerousPattern {
                command,
                rule,
                site,
            } => write!(
                f,
                "command '{command}' matches dangerous pattern '{rule}' at `{site}`"
            ),
        }
    }
}

impl std::error::Error for SandboxError {}

#[cfg(test)]
#[path = "sandbox_tests.rs"]
mod tests;
