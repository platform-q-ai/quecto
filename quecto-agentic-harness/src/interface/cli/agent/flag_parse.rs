//! Value-parsing helpers for the `agent` subcommand flag parser, and the
//! parsed [`AgentFlags`] the parser produces.

/// Parsed flags for the `agent` subcommand.
pub(crate) struct AgentFlags {
    pub(crate) session_name: Option<String>,
    pub(crate) no_session: bool,
    pub(crate) message: Option<String>,
    pub(crate) system_prompt: Option<String>,
    pub(crate) model_override: Option<String>,
    pub(crate) max_iterations: Option<u32>,
    pub(crate) max_time: Option<u64>,
    pub(crate) uds_mode: bool,
    pub(crate) no_sandbox: bool,
    pub(crate) socket_path: Option<std::path::PathBuf>,
    pub(crate) persist: bool,
    pub(crate) disabled_tools: Vec<String>,
    pub(crate) effort: Option<crate::domain::provider::EffortLevel>,
    pub(crate) workflow: bool,
    pub(crate) workflow_guards: bool,
    pub(crate) workflow_disabled: bool,
    pub(crate) workflow_spec_path: Option<std::path::PathBuf>,
    pub(crate) inherited_tool_policy:
        Option<crate::infrastructure::tools::inherited_tool_policy::InheritedToolPolicySnapshot>,
    /// `--parent-id`: the spawning agent's id, stamped onto this agent's emitted
    /// events so consumers can reconstruct the unit tree (PRD Stage B). `None`
    /// at the root.
    pub(crate) parent_id: Option<String>,
    /// Internal `--spawned`: set only by SpawnTool child launches. Selects the
    /// minimal child system prompt and parent-only docs filtering. Never inferred
    /// from `--parent-id`, session naming, env, or UDS mode (#1319).
    pub(crate) spawned: bool,
    /// Stable identity used by children to stamp `parent_id` and by workflow
    /// nudge scoping to find this agent's descendant tree. Defaults from the
    /// explicit session name; UDS unnamed generated-chat sessions override it
    /// with their unique chat key to avoid cross-session collisions.
    pub(crate) parent_identity_override: Option<String>,
    /// Pre-resolved runtime session key for UDS generated-chat sessions, so the
    /// tool registry and dispatch loop agree before tools are constructed.
    pub(crate) session_key_override: Option<String>,
}

/// Return `args[i+1]` or push `err_msg` to stderr and return `None`.
pub(super) fn next_arg<'a>(
    args: &'a [String],
    i: usize,
    err_msg: &str,
    stderr: &mut String,
) -> Option<&'a str> {
    if i + 1 < args.len() {
        Some(args[i + 1].as_str())
    } else {
        stderr.push_str(&format!("agent: {err_msg}\n"));
        None
    }
}

/// Parse a positive non-zero u32 for `--max-iterations`.
pub(super) fn parse_pos_u32(val: &str, flag: &str, stderr: &mut String) -> Option<u32> {
    match val.parse::<u32>() {
        Ok(n) if n > 0 => Some(n),
        _ => {
            stderr.push_str(&format!("agent: {flag} requires a positive integer\n"));
            None
        }
    }
}

/// Parse a positive non-zero u64 for `--max-time`.
pub(super) fn parse_pos_u64(val: &str, flag: &str, stderr: &mut String) -> Option<u64> {
    match val.parse::<u64>() {
        Ok(n) if n > 0 => Some(n),
        _ => {
            stderr.push_str(&format!("agent: {flag} requires a positive integer\n"));
            None
        }
    }
}

/// Parse an effort level string, writing an error to `stderr` on failure.
pub(super) fn parse_effort_level(
    val: &str,
    stderr: &mut String,
) -> Option<crate::domain::provider::EffortLevel> {
    crate::domain::provider::EffortLevel::parse(val).or_else(|| {
        stderr.push_str(&format!(
            "agent: invalid effort level '{}'; expected one of: {}\n",
            val,
            crate::domain::provider::EffortLevel::VALID_VALUES
        ));
        None
    })
}

/// Parse and validate a session name from `args[i+1]`.
pub(super) fn parse_session_name(args: &[String], i: usize, stderr: &mut String) -> Option<String> {
    let name = next_arg(args, i, "-s requires a session name", stderr)?;
    if !crate::interface::cli::is_valid_session_name(name) {
        stderr.push_str("agent: session name must contain only alphanumeric, '-', or '_'\n");
        return None;
    }
    Some(name.to_string())
}

/// Parse the `--mode` flag value. Returns `Some(true)` for `"uds"`, `None`
/// (with an error written to `stderr`) for any unknown value.
pub(super) fn parse_agent_mode(val: &str, stderr: &mut String) -> Option<bool> {
    match val {
        "uds" => Some(true),
        other => {
            stderr.push_str(&format!(
                "agent: --mode '{other}' is not valid; supported: uds\n"
            ));
            None
        }
    }
}

#[cfg(test)]
mod flag_parse_1066_tests {
    use super::parse_effort_level;

    /// Issue #1066: every OpenAI-documented effort level (none, low, medium,
    /// high, xhigh) must be accepted at configuration time.
    #[test]
    fn parse_effort_level_accepts_openai_documented_scale_1066() {
        for level in ["none", "low", "medium", "high", "xhigh"] {
            let mut stderr = String::new();
            let parsed = parse_effort_level(level, &mut stderr);
            assert!(
                parsed.is_some(),
                "effort '{level}' must be accepted (#1066); stderr: {stderr}"
            );
            assert!(
                stderr.is_empty(),
                "no error expected for effort '{level}' (#1066); stderr: {stderr}"
            );
        }
    }

    /// Issue #1066: an unknown effort string is rejected with a clear error
    /// naming the full set of valid values.
    #[test]
    fn parse_effort_level_rejection_names_valid_values_1066() {
        let mut stderr = String::new();
        let parsed = parse_effort_level("turbo", &mut stderr);
        assert!(parsed.is_none(), "'turbo' must be rejected");
        assert!(
            stderr.contains("invalid effort level 'turbo'"),
            "rejection must name the offending value; stderr: {stderr}"
        );
        assert!(
            stderr.contains("none, low, medium, high, xhigh, max"),
            "rejection must name the valid values (#1066); stderr: {stderr}"
        );
    }
}
