//! The checkout's `origin` remote behind the environments capability's
//! [`WorkspaceOrigin`] port (#2024 S4e): `git -C <checkout> remote get-url
//! origin`, with the global and system git configuration left in place
//! (an `insteadOf` rewrite is what the clone will see too). `get-url`
//! reads the local configuration only — no network, no credential
//! helper — so the call is unbounded. Not a checkout, or a checkout
//! without `origin`, is `None`; a git that is missing or cannot be run is
//! the error.

use std::path::Path;
use std::process::{Command, Stdio};

use crate::application::environments::ports::WorkspaceOrigin;

#[derive(Debug, Default, Clone, Copy)]
pub struct GitWorkspaceOrigin;

impl WorkspaceOrigin for GitWorkspaceOrigin {
    fn origin(&self, checkout: &Path) -> Result<Option<String>, String> {
        let output = Command::new("git")
            .arg("-C")
            .arg(checkout)
            .args(["remote", "get-url", "origin"])
            .env("GIT_TERMINAL_PROMPT", "0")
            // The checkout asked about is `-C <checkout>`, never a
            // repository the process environment points at.
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_COMMON_DIR")
            .stdin(Stdio::null())
            .output()
            .map_err(|error| format!("cannot run git: {error}"))?;
        let status = output.status;
        if status.success() {
            let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
            return Ok((!url.is_empty()).then_some(url));
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        // `No such remote 'origin'` (exit 2) and `not a git repository`
        // (exit 128) both mean "nothing to derive".
        if stderr.contains("No such remote")
            || stderr.contains("not a git repository")
            || stderr.contains("No such file or directory")
        {
            return Ok(None);
        }
        Err(format!(
            "git remote get-url origin exited {status}: {}",
            stderr.trim()
        ))
    }
}

#[cfg(test)]
#[path = "workspace_origin_tests.rs"]
mod tests;
