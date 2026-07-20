use crate::interface::cli::{protocol, uds};

/// Parsed representation of a client-sent `tool_result` command, ready
/// for `handle_tool_result` to consume.
pub(super) struct ParsedToolResult {
    pub(super) tool_call_id: String,
    pub(super) content: String,
    pub(super) is_error: bool,
}

/// Intercept a raw client line that carries a `tool_result` so it can
/// be resolved inline against `client_tool_registry`, bypassing the
/// (blocked-on-prompt) main dispatch loop.  Returns `None` when the
/// line isn't a tool_result, in which case the caller forwards it to
/// the dispatcher via the normal channel.
///
/// Implementation notes:
///
///  * **Cheap gate first.**  Every line the reader observes flows
///    through here, and the vast majority aren't tool_results.  A
///    cheap `contains` check short-circuits the full JSON parse for
///    the common case (prompts, register_tools, set_model, …).  The
///    needle is tight enough to avoid false positives on, say, a
///    prompt that literally discusses tool results.
///
///  * **Canonical parser reuse.**  Once the gate passes, we defer to
///    the same `parse_line` → `AgentCommand` path used by the main
///    dispatcher.  If `AgentCommand::ToolResult` ever gains a field,
///    the intercept path picks it up automatically and stays in sync
///    with the rest of the protocol surface — no hand-rolled JSON
///    extraction to drift.
pub(super) fn try_intercept_tool_result(line: &str) -> Option<ParsedToolResult> {
    if !line.contains(r#""tool_result""#) {
        return None;
    }
    match uds::parse_line(line) {
        uds::LineResult::Command(protocol::AgentCommand::ToolResult {
            tool_call_id,
            content,
            is_error,
        }) => Some(ParsedToolResult {
            tool_call_id,
            content,
            is_error,
        }),
        _ => None,
    }
}

#[cfg(test)]
#[path = "uds_tool_intercept_tests.rs"]
mod tests;
