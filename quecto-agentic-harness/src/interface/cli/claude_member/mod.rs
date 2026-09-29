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
    let Some(build) = ctx.claude_member else {
        stderr.push_str("agent: the claude-code member capability is not composed\n");
        return 1;
    };
    let built = build(&member_settings(ctx, flags));
    let handles = match built {
        Ok(handles) => handles,
        Err(refusal) => {
            stderr.push_str(&format!("agent: {refusal}\n"));
            return 1;
        }
    };
    let runtime = match super::build_tokio_runtime() {
        Ok(runtime) => runtime,
        Err(error) => {
            stderr.push_str(&format!("agent: failed to create runtime: {error}\n"));
            return 1;
        }
    };
    // Started or not, the member is not left running: the run fails.
    stderr.push_str(&runtime.block_on(start_and_end(&handles)));
    1
}

/// Why a member that started is not left running.
const NO_ENDPOINT_YET: &str =
    "agent: the claude-code member serves no endpoint yet (#2288); it was ended\n";
/// What composition builds the member from: `flags` over `ctx`.
fn member_settings(ctx: &CliContext, flags: &AgentFlags) -> ClaudeMemberSettings {
    ClaudeMemberSettings {
        member: flags
            .session_name
            .clone()
            .unwrap_or_else(|| DEFAULT_MEMBER.to_string()),
        model: flags.model_override.clone(),
        checkout: ctx
            .cwd
            .clone()
            .unwrap_or_else(|| std::path::PathBuf::from(".")),
        base_dir: ctx.base_dir(),
        parent: flags.parent_id.clone(),
        config_path: ctx.config_path.clone(),
        log_key: super::agent::member_log_key(flags),
    }
}

/// Start the member and end it again: without its UDS server (#2288)
/// nothing can reach it, so it is not left running. Returns the line that
/// says why it is not running: its start's refusal, or that it was ended.
/// Either way its records are kept before the process exits: dropping the
/// telemetry would not wait for them.
async fn start_and_end(handles: &ClaudeMemberHandles) -> String {
    match handles.session.start().await {
        Ok(()) => {
            let ended = handles.session.close().await;
            assert!(ended.is_ok(), "a started member can be ended: {ended:?}");
            handles.session.finish().await;
            NO_ENDPOINT_YET.to_string()
        }
        Err(refusal) => {
            handles.session.finish().await;
            format!("agent: {refusal}\n")
        }
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
