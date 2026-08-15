// Pure construction of the `quecto agent` child launch arguments.
//
// Extracted from `spawn.rs` so the exact flag set forwarded to a spawned child
// (notably `--model`, #881) is unit-testable without spawning a real process.
// Also hosts small launch helpers kept out of `spawn.rs` for the file-size cap.

use std::ffi::OsString;
use std::path::Path;

use crate::domain::subagent::SubagentConfig;

/// Write `data` to `path`, creating it privately: `O_CREAT|O_EXCL` (so a
/// pre-planted symlink at the path is rejected rather than followed) with
/// owner-only `0600` permissions. A stale file left by a crashed prior spawn is
/// removed and recreated once (the retry still uses `O_EXCL`). Falls back to a
/// plain write on non-unix platforms.
#[cfg(unix)]
pub(super) fn write_private_new(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    fn create_excl(path: &std::path::Path) -> std::io::Result<std::fs::File> {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
    }
    let mut file = match create_excl(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let _ = std::fs::remove_file(path);
            create_excl(path)?
        }
        Err(e) => return Err(e),
    };
    file.write_all(data)
}

#[cfg(not(unix))]
pub(super) fn write_private_new(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, data)
}

/// Parse and validate the raw `effort` argument from spawn tool args,
/// honoring the target model's effort vocabulary when a model is specified.
///
/// Returns `Ok(None)` when absent/null/empty, `Ok(Some(level))` for a valid
/// level, and `Err` for a non-string value or an invalid/out-of-vocabulary
/// level. Lives here (rather than in `spawn.rs`) so the spawn tool stays under
/// the repository file-size cap while keeping the validation unit-testable.
pub(super) fn parse_effort_arg(
    arg: Option<&serde_json::Value>,
    model: Option<&str>,
) -> Result<Option<String>, String> {
    use crate::domain::provider::EffortLevel;
    match arg {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(s)) => match s.trim() {
            "" => Ok(None),
            level => validate_effort(level, model).map(Some),
        },
        Some(_) => Err(format!(
            "effort must be a string; valid values: {}",
            EffortLevel::VALID_VALUES
        )),
    }
}

/// Validate a spawn `effort` level, honoring the target model's effort
/// vocabulary when a model is specified. Returns the normalized level string.
pub(super) fn validate_effort(level: &str, model: Option<&str>) -> Result<String, String> {
    use crate::domain::provider::EffortLevel;
    let parsed = EffortLevel::parse(level).ok_or_else(|| {
        format!(
            "invalid effort '{level}'; valid values: {}",
            EffortLevel::VALID_VALUES
        )
    })?;
    if let Some(valid) = model.map(EffortLevel::levels_for_model) {
        if !valid.contains(&parsed) {
            return Err(format!(
                "invalid effort '{level}'; valid values: {}",
                EffortLevel::levels_list(valid)
            ));
        }
    }
    Ok(parsed.as_str().to_string())
}

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
    pub inherited_tool_policy_path: Option<&'a Path>,
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
        inherited_tool_policy_path,
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
        // Explicit internal provenance flag (#1319). Always set for SpawnTool
        // children; never inferred from --parent-id / session / env / UDS.
        "--spawned".into(),
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

    if let Some(ref effort) = config.effort {
        args.push("--effort".into());
        args.push(effort.into());
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

    if let Some(policy_path) = inherited_tool_policy_path {
        args.push("--inherited-tool-policy-snapshot".into());
        args.push(policy_path.into());
    }

    // Forward each read-only tool restriction as `--disable-tool <name>` so the
    // child disables/hides it before the session starts and denies later runtime
    // re-registration (#957/#1276). The child CLI already consumes
    // `--disable-tool`.
    for tool in &config.disable_tools {
        args.push("--disable-tool".into());
        args.push(tool.into());
    }

    let _ = restrict_to_workspace;

    args
}

#[cfg(test)]
#[path = "spawn_launch_args_tests.rs"]
mod tests;
