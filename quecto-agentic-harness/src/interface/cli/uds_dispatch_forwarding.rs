use super::DispatchCtx;
use super::uds_dispatch_get_message_forward::{ForwardGetMessage, forward_subagent_get_message};
use super::{AgentCommand, AgentEvent};
use crate::domain::ids::{AgentId, CommandId, MessageId, ToolCallId};

/// Pre-router for commands addressed to a spawned sub-agent.
///
/// These commands must be intercepted before parent-local query/history fast
/// paths, otherwise the parent can answer from its own ledger and silently
/// ignore `agent_id`.
pub(super) async fn try_forward_subagent_targeted_command(
    cmd: &AgentCommand,
    ctx: &mut DispatchCtx<'_>,
) -> Option<bool> {
    if let AgentCommand::GetMessages {
        count,
        before,
        agent_id: Some(agent_id),
        id,
    } = cmd
    {
        let tn = cmd.type_name();
        let ev = forward_subagent_get_messages(
            ctx,
            id.as_deref().map(CommandId::from),
            tn,
            AgentId::from(agent_id.as_str()),
            *count,
            before.as_deref().map(MessageId::from),
        )
        .await;
        super::emit_response_or_frame_limit_error(ctx, id.as_deref(), tn, ev).await;
        return Some(false);
    }
    if let AgentCommand::GetMessagesTail {
        count,
        agent_id: Some(agent_id),
        id,
    } = cmd
    {
        let tn = cmd.type_name();
        let ev = forward_subagent_get_messages(
            ctx,
            id.as_deref().map(CommandId::from),
            tn,
            AgentId::from(agent_id.as_str()),
            Some(*count),
            None,
        )
        .await;
        super::emit_response_or_frame_limit_error(ctx, id.as_deref(), tn, ev).await;
        return Some(false);
    }
    if let AgentCommand::Sync {
        epoch,
        since_rev,
        agent_id: Some(agent_id),
        id,
    } = cmd
    {
        let tn = cmd.type_name();
        let ev = super::uds_dispatch_sync_forward::forward_subagent_sync(
            ctx,
            id.as_deref().map(CommandId::from),
            tn,
            AgentId::from(agent_id.as_str()),
            *epoch,
            *since_rev,
        )
        .await;
        super::emit_response_or_frame_limit_error(ctx, id.as_deref(), tn, ev).await;
        return Some(false);
    }
    if let AgentCommand::GetMessage {
        message_id,
        agent_id: Some(agent_id),
        tool_call_id,
        offset,
        limit,
        id,
    } = cmd
    {
        let tn = cmd.type_name();
        let ev = forward_subagent_get_message(
            ctx,
            id.as_deref(),
            tn,
            ForwardGetMessage {
                agent_id: AgentId::from(agent_id.as_str()),
                message_id: MessageId::from(message_id.as_str()),
                tool_call_id: tool_call_id.as_deref().map(ToolCallId::from),
                offset: *offset,
                limit: *limit,
            },
        )
        .await;
        // The child sizes the page with the forwarded correlation id, but
        // still guard the final parent envelope so no response can disappear
        // through the generic oversized-event drop path.
        super::emit_response_or_frame_limit_error(ctx, id.as_deref(), tn, ev).await;
        return Some(false);
    }
    None
}

/// Forward a `get_messages` request to a spawned sub-agent and wrap its
/// response as this command's reply (#795/#837/#843). With `count: Some(n)` the
/// child returns its last-N tail; with `None` it returns its full history.
///
/// Reuses the shared sub-agent socket lookup and UDS round-trip helpers rather
/// than re-deriving anything locally — the sub-agent answers from its own
/// conversation history. The child mapping always sends `get_messages` (the
/// optional `count` selects tail-vs-full); "tail" is an implementation detail of
/// that mapping, not this function's contract — hence the name covers both.
pub(super) async fn forward_subagent_get_messages(
    ctx: &DispatchCtx<'_>,
    id: Option<CommandId>,
    tn: &str,
    agent_id: AgentId,
    count: Option<usize>,
    before: Option<MessageId>,
) -> AgentEvent {
    use crate::infrastructure::tools::subagent_registry::{
        INSPECTOR_RESPONSE_TIMEOUT, lookup_subagent_socket, send_subagent_uds_command_with_timeout,
    };
    let id_ref = id.as_ref().map(CommandId::as_str);
    let Some(registry) = ctx.subagent_registry.as_ref() else {
        return AgentEvent::err(id_ref, tn, "no sub-agent registry available");
    };
    let socket_path = match lookup_subagent_socket(registry, agent_id.as_str()) {
        Ok(path) => path,
        Err(e) => return AgentEvent::err(id_ref, tn, e),
    };
    // Omit `count` entirely when None so the child returns its FULL history; a
    // present count requests just the tail (#843).
    let mut cmd = serde_json::json!({ "type": "get_messages" });
    if let Some(count) = count {
        cmd["count"] = serde_json::json!(count);
    }
    if let Some(before) = before.as_ref() {
        cmd["before"] = serde_json::json!(before.as_str());
    }
    let cmd = cmd.to_string();
    // This forward is awaited inline in the single shared dispatch loop, so it
    // uses the short interactive timeout — a slow/hung sub-agent must not stall
    // steer/abort/new-message for any client for the full agent_cmd 300s (#795).
    match send_subagent_uds_command_with_timeout(&socket_path, &cmd, INSPECTOR_RESPONSE_TIMEOUT)
        .await
    {
        // Preserve child failures instead of rewriting them as parent success.
        Ok(line) => match super::uds_forward_response::parse_forwarded_get_messages(&line) {
            Ok(data) => AgentEvent::ok(id_ref, tn, Some(data)),
            Err(error) => AgentEvent::err(id_ref, tn, error),
        },
        Err(e) => AgentEvent::err(id_ref, tn, e.to_string()),
    }
}
