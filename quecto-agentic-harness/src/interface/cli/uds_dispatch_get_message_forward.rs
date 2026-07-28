use super::{AgentEvent, DispatchCtx};
use crate::domain::ids::{AgentId, MessageId, ToolCallId};

pub(super) struct ForwardGetMessage {
    pub(super) agent_id: AgentId,
    pub(super) message_id: MessageId,
    pub(super) tool_call_id: Option<ToolCallId>,
    pub(super) offset: Option<usize>,
    pub(super) limit: Option<usize>,
}

/// #1060: forward `get_message` to a child session and return its payload.
pub(super) async fn forward_subagent_get_message(
    ctx: &DispatchCtx<'_>,
    id: Option<&str>,
    tn: &str,
    req: ForwardGetMessage,
) -> AgentEvent {
    use crate::infrastructure::tools::subagent_registry::{
        INSPECTOR_RESPONSE_TIMEOUT, lookup_subagent_socket, send_subagent_uds_command_with_timeout,
    };
    let Some(registry) = ctx.subagent_registry.as_ref() else {
        return AgentEvent::err(id, tn, "no sub-agent registry available");
    };
    let socket_path = match lookup_subagent_socket(registry, req.agent_id.as_str()) {
        Ok(path) => path,
        Err(e) => return AgentEvent::err(id, tn, e),
    };
    let mut cmd = serde_json::json!({
        "type": "get_message",
        "messageId": req.message_id.as_str(),
    });
    if let Some(tool_call_id) = req.tool_call_id.as_ref() {
        cmd["toolCallId"] = serde_json::json!(tool_call_id.as_str());
    }
    if let Some(offset) = req.offset {
        cmd["offset"] = serde_json::json!(offset);
    }
    if let Some(limit) = req.limit {
        cmd["limit"] = serde_json::json!(limit);
    }
    let cmd = cmd.to_string();
    match send_subagent_uds_command_with_timeout(&socket_path, &cmd, INSPECTOR_RESPONSE_TIMEOUT)
        .await
    {
        Ok(line) => match super::uds_forward_response::parse_forwarded_get_message(&line) {
            Ok(data) => AgentEvent::ok(id, tn, Some(data)),
            Err(error) => AgentEvent::err(id, tn, error),
        },
        Err(e) => AgentEvent::err(id, tn, e.to_string()),
    }
}
