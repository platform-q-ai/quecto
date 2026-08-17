use super::DispatchCtx;
use super::uds_dispatch_get_message_forward::{ForwardGetMessage, forward_subagent_get_message};
use super::{AgentCommand, AgentEvent};
use crate::domain::ids::{AgentId, CommandId, MessageId, ToolCallId};
use crate::infrastructure::tools::subagent_routing::{
    InspectionRoute, RoutableInspectionCommand, UDS_INSPECTION_ALLOWLIST, resolve_inspection_route,
};
use crate::interface::cli::uds_session;

/// Pre-router for commands addressed to a spawned sub-agent.
///
/// These commands must be intercepted before parent-local query/history fast
/// paths, otherwise the parent can answer from its own ledger and silently
/// ignore `agent_id`.
pub(super) async fn try_forward_subagent_targeted_command(
    cmd: &AgentCommand,
    ctx: &mut DispatchCtx<'_>,
) -> Option<bool> {
    let routable = RoutableInspectionCommand::from_uds_type(cmd.type_name())?;
    debug_assert!(UDS_INSPECTION_ALLOWLIST.contains(&routable));
    if let AgentCommand::GetState {
        agent_id: Some(agent_id),
        id,
    } = cmd
    {
        let tn = cmd.type_name();
        let ev = forward_subagent_get_state(
            ctx,
            id.as_deref().map(CommandId::from),
            tn,
            AgentId::from(agent_id.as_str()),
        )
        .await;
        super::emit_response_or_frame_limit_error(ctx, id.as_deref(), tn, ev).await;
        return Some(false);
    }
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
        thinking_offset,
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
                thinking_offset: *thinking_offset,
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

pub(super) async fn forward_subagent_get_state(
    ctx: &DispatchCtx<'_>,
    id: Option<CommandId>,
    tn: &str,
    agent_id: AgentId,
) -> AgentEvent {
    use crate::infrastructure::tools::subagent_registry::{
        INSPECTOR_RESPONSE_TIMEOUT, send_subagent_uds_command_with_timeout,
    };
    let id_ref = id.as_ref().map(CommandId::as_str);
    let Some(registry) = ctx.subagent_registry.as_ref() else {
        return AgentEvent::err(id_ref, tn, "no sub-agent registry available");
    };
    let route = match resolve_inspection_route(registry, agent_id.as_str()) {
        Ok(route) => route,
        Err(e) => return AgentEvent::err(id_ref, tn, e),
    };
    let mut cmd = serde_json::json!({ "type": "get_state" });
    if let InspectionRoute::ViaAncestor { target_id, .. } = &route {
        cmd["agent_id"] = serde_json::json!(target_id);
    }
    let socket_path = match &route {
        InspectionRoute::Direct { socket_path } => socket_path,
        InspectionRoute::ViaAncestor {
            ancestor_socket_path,
            ..
        } => ancestor_socket_path,
    };
    match send_subagent_uds_command_with_timeout(
        socket_path,
        &cmd.to_string(),
        INSPECTOR_RESPONSE_TIMEOUT,
    )
    .await
    {
        Ok(line) => match super::uds_forward_response::parse_forwarded_response(&line, "get_state")
        {
            Ok(data) => AgentEvent::ok(id_ref, tn, Some(data)),
            Err(error) => AgentEvent::err(id_ref, tn, error),
        },
        Err(e) => AgentEvent::err(id_ref, tn, e.to_string()),
    }
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
        INSPECTOR_RESPONSE_TIMEOUT, send_subagent_uds_command_with_timeout,
    };
    let id_ref = id.as_ref().map(CommandId::as_str);
    let Some(registry) = ctx.subagent_registry.as_ref() else {
        return AgentEvent::err(id_ref, tn, "no sub-agent registry available");
    };
    let route = match resolve_inspection_route(registry, agent_id.as_str()) {
        Ok(route) => route,
        Err(e) => {
            let historical_session_key = {
                let entries = registry
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if let Some(entry) = entries.get(agent_id.as_str()) {
                    Ok(Some(entry.agent_uuid.as_str().to_string()))
                } else {
                    let mut matches = entries
                        .iter()
                        .filter(|(key, entry)| {
                            entry.effective_display_name(key) == agent_id.as_str()
                        })
                        .map(|(_, entry)| entry.agent_uuid.as_str().to_string());
                    match (matches.next(), matches.next()) {
                        (Some(first), None) => Ok(Some(first)),
                        (Some(_), Some(_)) => Err(format!(
                            "duplicate historical subagent display label '{}'",
                            agent_id.as_str()
                        )),
                        (None, _) => Ok(None),
                    }
                }
            };
            match historical_session_key {
                Ok(Some(session_key)) => {
                    return match ctx.session_store.load(&session_key).await {
                        Ok(Some(session)) => {
                            if let Some(before) = before.as_ref()
                                && uds_session::position_by_message_id(&session.messages, before)
                                    .is_none()
                            {
                                return AgentEvent::err(
                                    id_ref,
                                    tn,
                                    format!("history cursor not found: {}", before.as_str()),
                                );
                            }
                            let data = uds_session::messages_page_json_for_id(
                                &session.messages,
                                count.unwrap_or(session.messages.len()),
                                before.as_ref(),
                            );
                            AgentEvent::ok(id_ref, tn, Some(data))
                        }
                        Ok(None) => AgentEvent::err(
                            id_ref,
                            tn,
                            format!(
                                "no persisted transcript for subagent '{}'",
                                agent_id.as_str()
                            ),
                        ),
                        Err(err) => AgentEvent::err(id_ref, tn, err.to_string()),
                    };
                }
                Err(err) => return AgentEvent::err(id_ref, tn, err),
                Ok(None) => return AgentEvent::err(id_ref, tn, e),
            }
        }
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
    if let InspectionRoute::ViaAncestor { target_id, .. } = &route {
        cmd["agent_id"] = serde_json::json!(target_id);
    }
    let socket_path = match &route {
        InspectionRoute::Direct { socket_path } => socket_path,
        InspectionRoute::ViaAncestor {
            ancestor_socket_path,
            ..
        } => ancestor_socket_path,
    };
    let cmd = cmd.to_string();
    // This forward is awaited inline in the single shared dispatch loop, so it
    // uses the short interactive timeout — a slow/hung sub-agent must not stall
    // steer/abort/new-message for any client for the full agent_cmd 300s (#795).
    match send_subagent_uds_command_with_timeout(socket_path, &cmd, INSPECTOR_RESPONSE_TIMEOUT)
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
