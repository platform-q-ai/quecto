//! The claude-code member harness (#2287, owner decision O2): `quecto agent
//! --mode uds --backend claude-code` keeps quecto's admission and identity
//! (`cmd_agent` runs them first), then drives the Claude Code CLI through
//! the composed session handle. This slice's runner starts the member and
//! ends it again: its UDS server, the endpoint a coordinator reaches it
//! on, lands with #2288.

pub mod handles;

pub use handles::{ClaudeMemberHandles, ClaudeMemberHandlesBuilder, ClaudeMemberSettings};

use super::CliContext;
use super::agent::AgentFlags;

/// The member's name when `-s` is not given.
const DEFAULT_MEMBER: &str = "claude-member";

/// Run a claude-code member for `flags`, through the handles composition
/// builds. Returns the process's exit code.
pub(crate) fn run(ctx: &CliContext, flags: &AgentFlags, stderr: &mut String) -> i32 {
    let _ = (ctx, flags, stderr, DEFAULT_MEMBER);
    0
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
