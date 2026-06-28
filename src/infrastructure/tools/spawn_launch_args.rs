// Pure construction of the `quecto agent` child launch arguments.
//
// Extracted from `spawn.rs` so the exact flag set forwarded to a spawned child
// (notably `--model`, #881) is unit-testable without spawning a real process.

use std::ffi::OsString;
use std::path::Path;

use crate::domain::subagent::SubagentConfig;

/// Build the ordered CLI argument list for launching a child `quecto agent` in
/// UDS mode. The caller appends these to a `Command` whose program is the quecto
/// binary, and is responsible for the side-effecting bits (writing the workflow
/// spec file, setting env, stdio) — this function is pure.
///
/// `effective_config` is the resolved `--config` path (explicit arg or inherited
/// runtime config); `workflow_spec_path` is the already-written spec file path,
/// if any.
pub(super) fn build_child_cli_args(
    session_name: &str,
    socket_path: &Path,
    config: &SubagentConfig,
    effective_config: Option<&Path>,
    parent_id: Option<&str>,
    restrict_to_workspace: bool,
    workflow_spec_path: Option<&Path>,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![
        "agent".into(),
        "--mode".into(),
        "uds".into(),
        "-s".into(),
        session_name.into(),
        "--socket".into(),
        socket_path.into(),
        "--persist".into(),
    ];

    if let Some(ref system) = config.system {
        args.push("--system".into());
        args.push(system.into());
    }

    // Explicit model override (#881). The child's CLI already consumes `--model`;
    // precedence (explicit model > --config > default) is realised by the child
    // resolving --model ahead of --config.
    if let Some(ref model) = config.model {
        args.push("--model".into());
        args.push(model.into());
    }

    // Forward --config when a custom (or inherited runtime) config applies, so
    // children share the same tool isolation defaults as the parent.
    if let Some(cfg_path) = effective_config {
        args.push("--config".into());
        args.push(cfg_path.into());
    }

    // Tell the child who its parent is so its own emitted events carry the
    // correct parent_id (PRD Stage B).
    if let Some(parent_id) = parent_id {
        args.push("--parent-id".into());
        args.push(parent_id.into());
    }

    if config.workflow {
        args.push("--workflow".into());
    }
    if config.workflow_guards {
        args.push("--workflow-guards".into());
    }

    if let Some(spec_path) = workflow_spec_path {
        args.push("--workflow-spec".into());
        args.push(spec_path.into());
    }

    // Propagate --no-sandbox so children inherit the parent's workspace posture.
    if !restrict_to_workspace {
        args.push("--no-sandbox".into());
    }

    args
}

#[cfg(test)]
#[path = "spawn_launch_args_tests.rs"]
mod tests;
