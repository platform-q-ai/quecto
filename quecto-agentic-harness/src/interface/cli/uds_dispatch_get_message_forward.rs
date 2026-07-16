use super::{AgentEvent, DispatchCtx};

pub(super) struct ForwardGetMessage<'a> {
    pub(super) agent_id: &'a str,
    pub(super) message_id: &'a str,
    pub(super) tool_call_id: Option<&'a str>,
    pub(super) offset: Option<usize>,
    pub(super) limit: Option<usize>,
}

/// #1060: forward `get_message` to a child session and return its payload.
pub(super) async fn forward_subagent_get_message(
    ctx: &DispatchCtx<'_>,
    id: Option<&str>,
    tn: &str,
    req: ForwardGetMessage<'_>,
) -> AgentEvent {
    use crate::infrastructure::tools::subagent_registry::{
        INSPECTOR_RESPONSE_TIMEOUT, lookup_subagent_socket, send_subagent_uds_command_with_timeout,
    };
    let Some(registry) = ctx.subagent_registry.as_ref() else {
        return AgentEvent::err(id, tn, "no sub-agent registry available");
    };
    let socket_path = match lookup_subagent_socket(registry, req.agent_id) {
        Ok(path) => path,
        Err(e) => return AgentEvent::err(id, tn, e),
    };
    let mut cmd = serde_json::json!({
        "type": "get_message",
        "messageId": req.message_id,
    });
    if let Some(tool_call_id) = req.tool_call_id {
        cmd["toolCallId"] = serde_json::json!(tool_call_id);
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
        Ok(line) => {
            let parsed: serde_json::Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(e) => return AgentEvent::err(id, tn, e.to_string()),
            };
            if parsed.get("success").and_then(|v| v.as_bool()) == Some(false) {
                let err = parsed
                    .get("error")
                    .and_then(|e| e.as_str())
                    .unwrap_or("get_message failed");
                return AgentEvent::err(id, tn, err.to_string());
            }
            let data = parsed.get("data").cloned();
            AgentEvent::ok(id, tn, data)
        }
        Err(e) => AgentEvent::err(id, tn, e.to_string()),
    }
}
