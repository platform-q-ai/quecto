//! `claude -p` stream-json as an external agent process (#2286).
//!
//! [`ClaudeCodeLauncher`] starts `claude` with the stream-json flags, the
//! member's allowlisted environment ([`super::environment`]) and its
//! checkout as working directory, spawned through the
//! [`OwnedChildSupervisor`] — the one owner and signaller of every child —
//! in a process group of its own. [`ClaudeCodeProcess`] writes user turns
//! as stream-json user messages and decodes stdout through
//! [`super::stream_json`].

use std::ffi::OsString;
use std::sync::Arc;

use crate::application::external_agent::dto::{ExternalAgentLaunchError, ExternalAgentLaunchSpec};
use crate::application::external_agent::ports::{
    ExternalAgentLauncher, ExternalAgentProcess, PortFuture,
};
use crate::infrastructure::processes::owned_child_supervisor::OwnedChildSupervisor;

/// The program looked up on `PATH`.
pub const CLAUDE_PROGRAM: &str = "claude";

/// Starts `claude` member processes.
pub struct ClaudeCodeLauncher {
    supervisor: Arc<OwnedChildSupervisor>,
    parent_environment: Vec<(OsString, OsString)>,
}

impl ClaudeCodeLauncher {
    /// A launcher that adopts its children into `supervisor` and builds
    /// each member's environment from `parent_environment`.
    pub fn new(
        supervisor: Arc<OwnedChildSupervisor>,
        parent_environment: Vec<(OsString, OsString)>,
    ) -> Self {
        Self {
            supervisor,
            parent_environment,
        }
    }
}

/// The argv after the program: the stream-json flags and the spec's values.
pub fn claude_arguments(
    spec: &ExternalAgentLaunchSpec,
) -> Result<Vec<String>, ExternalAgentLaunchError> {
    let _ = spec;
    Ok(Vec::new())
}

impl ExternalAgentLauncher for ClaudeCodeLauncher {
    fn start<'a>(
        &'a self,
        spec: ExternalAgentLaunchSpec,
    ) -> PortFuture<'a, Result<Box<dyn ExternalAgentProcess>, ExternalAgentLaunchError>> {
        let _ = (&self.supervisor, &self.parent_environment, spec);
        Box::pin(async { Err(ExternalAgentLaunchError::Spawn("not implemented".into())) })
    }
}

#[cfg(test)]
#[path = "process_tests.rs"]
mod tests;
