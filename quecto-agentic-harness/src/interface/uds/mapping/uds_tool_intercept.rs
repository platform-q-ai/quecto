use crate::interface::uds::wire::protocol;

/// Parsed representation of a client-sent `tool_result` command, ready
/// for `handle_tool_result` to consume.
pub(in crate::interface::uds) struct ParsedToolResult {
    pub(in crate::interface::uds) tool_call_id: String,
    pub(in crate::interface::uds) content: String,
    pub(in crate::interface::uds) is_error: bool,
    /// The `imageBlocks` field as sent (#2423), admitted on delivery.
    pub(in crate::interface::uds) image_blocks: Option<protocol::WireImageBlocks>,
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
///  * **Read straight into the fields (#2423 review M1).**  The line is
///    read into [`ToolResultLine`], not through the internally tagged
///    `AgentCommand`, which buffers the whole value before it looks at
///    the tag: read directly, `imageBlocks` streams, so a list past the
///    image limit is counted, never held. The fields mirror
///    `AgentCommand::ToolResult`'s (its wire names and defaults; unknown
///    fields ignored alike), and a test checks both readings agree.
pub(in crate::interface::uds) fn try_intercept_tool_result(line: &str) -> Option<ParsedToolResult> {
    if !line.contains(r#""tool_result""#) {
        return None;
    }
    let parsed: ToolResultLine = serde_json::from_str(line.trim()).ok()?;
    match parsed.kind.as_str() {
        "tool_result" => Some(ParsedToolResult {
            tool_call_id: parsed.tool_call_id,
            content: parsed.content,
            is_error: parsed.is_error,
            image_blocks: parsed.image_blocks,
        }),
        _ => None,
    }
}

/// A `tool_result` command's fields as the wire spells them (see
/// `AgentCommand::ToolResult`), with its `type`.
#[derive(serde::Deserialize)]
struct ToolResultLine {
    #[serde(rename = "type")]
    kind: String,
    #[serde(rename = "toolCallId")]
    tool_call_id: String,
    content: String,
    #[serde(rename = "isError", default)]
    is_error: bool,
    #[serde(rename = "imageBlocks", default)]
    image_blocks: Option<protocol::WireImageBlocks>,
}

#[cfg(test)]
mod tests;
// The reader-side interception tests (#994) once beside the multi-client
// loop; they exercise only this parser, so they stay its siblings.
#[cfg(test)]
mod multi_interception_tests;
