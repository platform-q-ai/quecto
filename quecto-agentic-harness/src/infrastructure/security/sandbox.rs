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
use super::policy_rule::PolicyRule;
use super::protected_dirs::HostContext;

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
        static WARNED: std::sync::Once = std::sync::Once::new();
        if config
            .agents
            .defaults
            ._deprecated_command_allowlist
            .is_some()
        {
            WARNED.call_once(|| {
                tracing::warn!(
                    "agents.defaults.command_allowlist is deprecated and ignored (#1620); \
                 command policy is denylist-only. Use the container runtime for isolation."
                )
            });
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
    /// are not executable and do not match. Recursive deletes of the home
    /// directory, the workspace root and top-level system directories are
    /// blocked using locations resolved on this host at check time.
    ///
    /// Syntax the parser cannot resolve — a `$var` in command position, an
    /// unbalanced quote — triggers an explicit fallback to the pre-#1620
    /// whole-string substring scan, so dynamic constructs are never quietly
    /// waved through.
    pub fn validate_command(&self, command: &str) -> Result<(), SandboxError> {
        let host = HostContext::from_host(self.workspace.as_deref());
        denylist::check_with(command, &host).map_err(|v| {
            // The rule id is for the logs; the model reads the explanation.
            tracing::info!(rule = v.rule.id(), site = %v.site, "command refused by command policy");
            SandboxError::Refused {
                command: command.to_string(),
                rule: v.rule,
                site: v.site,
            }
        })
    }
}

#[derive(Debug)]
pub enum SandboxError {
    /// A command-policy rule refused the command.
    Refused {
        /// The full command as submitted.
        command: String,
        /// The rule that matched (its id is logged; its explanation is shown).
        rule: PolicyRule,
        /// The simple command the rule matched at, or a fallback-scan note.
        site: String,
    },
}

impl std::fmt::Display for SandboxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // #2198: what matched, why it is refused, and a way forward.
            SandboxError::Refused {
                command,
                rule,
                site,
            } => write!(
                f,
                "command '{command}' blocked by command policy (rule {}) at `{site}`: {}. \
                 Instead: {}. {}",
                rule.id(),
                rule.reason(),
                rule.instead(),
                rule.closing()
            ),
        }
    }
}

impl std::error::Error for SandboxError {}

#[cfg(test)]
#[path = "sandbox_tests.rs"]
mod tests;
