use super::super::uds_query::{GetMessageLookup, query_response_data};
use super::{
    AgentCommand, AgentEvent, DispatchCtx, emit_event_to_broadcast_or_writer,
    emit_response_or_frame_limit_error, emit_response_or_frame_limit_error_with_message,
};
use crate::domain::ids::{CommandId, MessageId, ToolCallId};

pub(super) fn session_summary_to_json(
    summary: &crate::domain::session::SessionSummary,
) -> serde_json::Value {
    serde_json::json!({
        "key": summary.key,
        "title": super::display_title(&summary.title),
        "messageCount": summary.message_count,
        "updatedUnixSecs": summary.updated_unix_secs,
        "updatedAt": summary.updated_unix_secs,
    })
}

/// Returns `Some(bool)` if handled, `None` to fall through to the main match.
pub(super) async fn dispatch_fieldless_command(
    cmd: &AgentCommand,
    ctx: &mut DispatchCtx<'_>,
) -> Option<bool> {
    let id = cmd.id();
    let tn = cmd.type_name();
    if matches!(cmd, AgentCommand::ListSessions { .. }) {
        let event = match ctx
            .session_store
            .list(Some(crate::domain::session::USER_CHAT_PREFIX))
            .await
        {
            Ok(sessions) => AgentEvent::ok(
                id,
                tn,
                Some(serde_json::json!({
                    "sessions": sessions
                        .iter()
                        .map(session_summary_to_json)
                        .collect::<Vec<_>>()
                })),
            ),
            Err(err) => AgentEvent::err(id, tn, err.to_string()),
        };
        emit_event_to_broadcast_or_writer(ctx, &event).await;
        return Some(false);
    }
    // #1060 review 1a: resolve get_message against the id-addressable ledger
    // (full copies) before the live conversation, so a ref pruned/collapsed
    // from `ctx.messages` still resolves to full content. The ledger wins over
    // a possibly-collapsed live entry.
    if let AgentCommand::GetMessage {
        message_id,
        tool_call_id,
        offset,
        thinking_offset,
        limit,
        ..
    } = cmd
    {
        let resolved = super::super::uds_snapshots::resolve_get_message(
            &ctx.conversation_snapshot,
            message_id,
        )
        .await
        .and_then(|msg| match tool_call_id.as_deref() {
            Some(tool_call_id) => {
                super::super::uds_session::tool_call_arguments_to_json_range_for_response(
                    &msg,
                    tool_call_id,
                    *offset,
                    *limit,
                    id,
                )
            }
            None => Some(
                super::super::uds_session::message_to_json_range_for_response(
                    &msg,
                    *offset,
                    *thinking_offset,
                    *limit,
                    id,
                ),
            ),
        });
        let ev = match resolved.or_else(|| {
            super::super::uds_query::get_message_response_data(GetMessageLookup {
                message_id: MessageId::from(message_id.as_str()),
                tool_call_id: tool_call_id.as_deref().map(ToolCallId::from),
                offset: *offset,
                thinking_offset: *thinking_offset,
                limit: *limit,
                request_id: id.map(CommandId::from),
                ctx,
            })
        }) {
            Some(data) => AgentEvent::ok(id, tn, Some(data)),
            None => AgentEvent::err(id, tn, format!("message not found: {message_id}")),
        };
        emit_response_or_frame_limit_error(ctx, id, tn, ev).await;
        return Some(false);
    }
    // A supplied paging cursor is a stable message id. Treat a stale/unknown
    // id as an error instead of silently restarting at the newest page, which a
    // client would otherwise prepend and duplicate as "older" history.
    if let AgentCommand::GetMessages {
        before: Some(cursor),
        ..
    } = cmd
        && super::super::uds_session::position_by_wire_id(ctx.messages, cursor).is_none()
    {
        let ev = AgentEvent::err(id, tn, format!("history cursor not found: {cursor}"));
        emit_event_to_broadcast_or_writer(ctx, &ev).await;
        return Some(false);
    }
    if let AgentCommand::Sync {
        epoch, since_rev, ..
    } = cmd
    {
        let data = ctx
            .conversation_snapshot
            .read()
            .await
            .sync_json(*epoch, *since_rev);
        emit_response_or_frame_limit_error_with_message(
            ctx,
            id,
            tn,
            AgentEvent::ok(id, tn, Some(data)),
            super::super::uds_busy_sync::SYNC_OVERSIZED_ERROR,
        )
        .await;
        return Some(false);
    }
    if let Some(data) = query_response_data(cmd, ctx) {
        emit_response_or_frame_limit_error(ctx, id, tn, AgentEvent::ok(id, tn, Some(data))).await;
        return Some(false);
    }
    if matches!(cmd, AgentCommand::ClearHistory { .. }) {
        return Some(super::uds_dispatch_session::handle_clear_history(ctx, id, tn).await);
    }
    None
}
