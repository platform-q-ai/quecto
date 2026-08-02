use super::flag_parse::next_arg;

pub(super) fn parse_snapshot_path(
    args: &[String],
    i: usize,
    stderr: &mut String,
) -> Option<std::path::PathBuf> {
    next_arg(
        args,
        i,
        "--inherited-tool-policy-snapshot requires a path",
        stderr,
    )
    .map(std::path::PathBuf::from)
}

pub(super) fn load_inherited_tool_policy_for_valid_child(
    path: &std::path::Path,
    stderr: &mut String,
    flags: &mut super::flag_parse::AgentFlags,
) -> Option<()> {
    if !flags.spawned || !flags.uds_mode {
        stderr.push_str(
            "agent: --inherited-tool-policy-snapshot requires --spawned and --mode uds\n",
        );
        return None;
    }

    match crate::infrastructure::tools::inherited_tool_policy::load_validate_unlink(path) {
        Ok(snapshot) => {
            flags.inherited_tool_policy = Some(snapshot);
            Some(())
        }
        Err(e) => {
            stderr.push_str(&format!("agent: {e}\n"));
            None
        }
    }
}
