// Pure construction of the `quecto agent` child launch arguments.
//
// Extracted from `spawn.rs` so the exact flag set forwarded to a spawned child
// (notably `--model`, #881) is unit-testable without spawning a real process.

use std::ffi::OsString;
use std::path::Path;

use crate::domain::subagent::SubagentConfig;

/// Resolved launch context for a child agent: the deterministic inputs that are
/// not carried on [`SubagentConfig`]. Grouped into a struct so the builder has a
/// single descriptive parameter rather than a long positional list.
pub(super) struct ChildLaunchSpec<'a> {
    pub session_name: &'a str,
    pub socket_path: &'a Path,
    pub config: &'a SubagentConfig,
    /// Resolved `--config` path (explicit arg or inherited runtime config).
    pub effective_config: Option<&'a Path>,
    pub parent_id: Option<&'a str>,
    pub restrict_to_workspace: bool,
    /// Already-written workflow spec file path, if any.
    pub workflow_spec_path: Option<&'a Path>,
}

/// Build the ordered CLI argument list for launching a child `quecto agent` in
/// UDS mode. The caller appends these to a `Command` whose program is the quecto
/// binary, and is responsible for the side-effecting bits (writing the workflow
/// spec file, setting env, stdio) — this function is pure.
pub(super) fn build_child_cli_args(spec: &ChildLaunchSpec<'_>) -> Vec<OsString> {
    let ChildLaunchSpec {
        session_name,
        socket_path,
        config,
        effective_config,
        parent_id,
        restrict_to_workspace,
        workflow_spec_path,
    } = *spec;

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
    // precedence (explicit model > --config > default) is realised by the child's
    // `resolve_agent_model` preferring `--model` over the config default, not by
    // the order in which these flags are emitted here.
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
