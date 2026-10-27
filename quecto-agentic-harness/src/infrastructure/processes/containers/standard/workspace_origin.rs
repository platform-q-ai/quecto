//! The checkout's `origin` remote behind the environments capability's
//! [`WorkspaceOrigin`] port (#2024 S4e): `git -C <checkout> remote get-url
//! origin`, with the global and system git configuration left in place
//! (an `insteadOf` rewrite is what the clone will see too). `get-url`
//! reads the local configuration only — no network, no credential
//! helper — so the call is unbounded. Not a checkout, or a checkout
//! without `origin`, is `None`; a git that is missing or cannot be run is
//! the error. The toplevel is `git rev-parse --show-toplevel`, the same
//! way.

use std::path::Path;
use std::process::{Command, Stdio};

use crate::application::environments::ports::container::WorkspaceOrigin;
use crate::application::subagents::ports::container_repository::ProjectOrigin;

#[derive(Debug, Default, Clone, Copy)]
pub struct GitWorkspaceOrigin;

/// `git -C <checkout> <args>`, about that checkout only (never a
/// repository the process environment points at), no prompt.
fn git(checkout: &Path, args: &[&str]) -> Result<std::process::Output, String> {
    Command::new("git")
        .arg("-C")
        .arg(checkout)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("cannot run git: {error}"))
}

impl WorkspaceOrigin for GitWorkspaceOrigin {
    fn origin(&self, checkout: &Path) -> Result<Option<String>, String> {
        let output = git(checkout, &["remote", "get-url", "origin"])?;
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

    fn toplevel(&self, checkout: &Path) -> Result<Option<std::path::PathBuf>, String> {
        let output = git(checkout, &["rev-parse", "--show-toplevel"])?;
        if output.status.success() {
            let top = String::from_utf8_lossy(&output.stdout).trim().to_string();
            return Ok((!top.is_empty()).then(|| std::path::PathBuf::from(top)));
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("not a git repository") || stderr.contains("No such file or directory") {
            return Ok(None);
        }
        Err(format!(
            "git rev-parse --show-toplevel exited {}: {}",
            output.status,
            stderr.trim()
        ))
    }
}

/// The launching project's origin a swarm's container is confirmed
/// against (swarm-container-matches-repo): `remote.origin.url` as written
/// in the checkout's own configuration (`git config --local`), no
/// `insteadOf` applied, so it compares like for like with a create argv's
/// `--repo`, which is read as written too. Not a checkout, a directory
/// that is gone, or no `origin` is `None`.
impl ProjectOrigin for GitWorkspaceOrigin {
    fn origin(&self, checkout: &Path) -> Result<Option<String>, String> {
        let output = git(
            checkout,
            &["config", "--local", "--get", "remote.origin.url"],
        )?;
        let stderr = String::from_utf8_lossy(&output.stderr);
        let nothing_to_read = [
            "can only be used inside a git repository",
            "not a git repository",
            "No such file or directory",
        ];
        match output.status.code() {
            Some(0) => {
                let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
                Ok((!url.is_empty()).then_some(url))
            }
            // `--get` of a key that is not set.
            Some(1) => Ok(None),
            _ if nothing_to_read.iter().any(|said| stderr.contains(said)) => Ok(None),
            _ => Err(format!(
                "git config --get remote.origin.url exited {}: {}",
                output.status,
                stderr.trim()
            )),
        }
    }
}

#[cfg(test)]
#[path = "workspace_origin_tests.rs"]
mod tests;
