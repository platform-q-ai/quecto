//! The checkout's `origin` remote behind the environments capability's
//! [`WorkspaceOrigin`] port (#2024 S4e): `git -C <checkout> remote get-url
//! origin`, with the global and system git configuration left in place
//! (an `insteadOf` rewrite is what the clone will see too). Not a
//! checkout, or a checkout without `origin`, is `None`; a git that is
//! missing or cannot be run is the error.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::application::environments::ports::WorkspaceOrigin;

/// A bound so a hung credential helper or a stuck filesystem cannot stall
/// `init`; `get-url` reads the local config only.
const GIT_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Default, Clone, Copy)]
pub struct GitWorkspaceOrigin;

impl WorkspaceOrigin for GitWorkspaceOrigin {
    fn origin(&self, checkout: &Path) -> Result<Option<String>, String> {
        let mut child = Command::new("git")
            .arg("-C")
            .arg(checkout)
            .args(["remote", "get-url", "origin"])
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("cannot run git: {error}"))?;
        let started = std::time::Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if started.elapsed() < GIT_TIMEOUT => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "git remote get-url origin did not answer within {GIT_TIMEOUT:?}"
                    ));
                }
                Err(error) => return Err(format!("cannot wait for git: {error}")),
            }
        };
        let output = child
            .wait_with_output()
            .map_err(|error| format!("cannot read git's output: {error}"))?;
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
