use super::flag_parse::next_arg;

pub(super) fn parse_inherited_tool_policy_flag(
    args: &[String],
    i: usize,
    stderr: &mut String,
    inherited_tool_policy: &mut Option<
        crate::infrastructure::tools::inherited_tool_policy::InheritedToolPolicySnapshot,
    >,
) -> Option<()> {
    let val = next_arg(
        args,
        i,
        "--inherited-tool-policy-snapshot requires a path",
        stderr,
    )?;
    match crate::infrastructure::tools::inherited_tool_policy::load_validate_unlink(
        std::path::Path::new(val),
    ) {
        Ok(snapshot) => {
            *inherited_tool_policy = Some(snapshot);
            Some(())
        }
        Err(e) => {
            stderr.push_str(&format!("agent: {e}\n"));
            None
        }
    }
}
