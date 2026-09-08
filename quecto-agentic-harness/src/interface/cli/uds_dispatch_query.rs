use super::super::uds_query::{GetMessageLookup, query_response_data_result};
use super::{
    AgentCommand, AgentEvent, DispatchCtx, emit_event_to_broadcast_or_writer,
    emit_response_or_frame_limit_error, emit_response_or_frame_limit_error_with_message,
};
use crate::domain::ids::{CommandId, MessageId, ToolCallId};

fn is_resume_picker_eligible_key(key: &str) -> bool {
    if let Some(name) = key.strip_prefix("cli:") {
        return crate::interface::cli::is_valid_session_name(name);
    }
    let Some(rest) = key.strip_prefix(crate::domain::session::USER_CHAT_PREFIX) else {
        return false;
    };
    let Some((seconds, unique)) = rest.split_once('-') else {
        return false;
    };
    !seconds.is_empty()
        && seconds.bytes().all(|byte| byte.is_ascii_digit())
        && !unique.is_empty()
        && unique.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(super) fn session_summary_to_json(
    summary: &crate::domain::session::SessionSummary,
) -> serde_json::Value {
    let metadata = summary.latest_execution_metadata.as_ref();
    serde_json::json!({
        "key": summary.key,
        "title": super::display_title(&summary.title),
        "messageCount": summary.message_count,
        "updatedUnixSecs": summary.updated_unix_secs,
        "updatedAt": summary.updated_unix_secs,
        "folderIdentity": metadata.and_then(|m| m.folder_identity()).map(|v| v.encoded_key()),
        "folderLabel": metadata.and_then(|m| m.folder_label()).map(|v| v.as_str()),
        "agentName": metadata.and_then(|m| m.agent_name()).map(|v| v.as_str()),
        "gitBranch": metadata.and_then(|m| m.git_branch()).map(|v| v.as_str()),
    })
}

/// Returns `Some(bool)` if handled, `None` to fall through to the main match.
pub(super) async fn dispatch_fieldless_command(
    cmd: &AgentCommand,
    ctx: &mut DispatchCtx<'_>,
) -> Option<bool> {
    let id = cmd.id();
    let tn = cmd.type_name();
    if let AgentCommand::ListSessions { .. } = cmd {
        // Scope is an execution property owned by the connected server.  The
        // request intentionally has no scope field: accepting one would let a
        // hostile/stale client browse another folder's sessions.
        let current = super::super::uds_lifecycle::capture_execution_metadata(ctx.base_dir, None);
        let scope = current.folder_identity();
        let scope_available = scope.is_some();
        let event = match ctx.session_store.list(None).await {
            Ok(sessions) => AgentEvent::ok(
                id,
                tn,
                Some(serde_json::json!({
                    "sessions": sessions
                        .iter()
                        .filter(|summary| scope.is_some_and(|scope| {
                            summary.latest_execution_metadata.as_ref()
                                .and_then(|metadata| metadata.folder_identity()) == Some(scope)
                        }))
                        .filter(|summary| is_resume_picker_eligible_key(&summary.key))
                        .map(session_summary_to_json)
                        .collect::<Vec<_>>(),
                    "scopeStatus": if scope_available { "available" } else { "unavailable" }
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
    // Catalogue refresh performs (sequential, bounded) blocking HTTP; running
    // it inline would freeze every other UDS command for the whole run, so it
    // executes on a dedicated blocking worker thread while the dispatch loop
    // stays responsive (slice-4 review). `reqwest::blocking` is also only
    // safe off the async runtime's core threads.
    if let AgentCommand::RefreshModels { source, .. } = cmd {
        let base_dir = ctx.base_dir.to_path_buf();
        let source = source.clone();
        let result = tokio::task::spawn_blocking(move || {
            super::super::uds_models::refresh_models_data(&base_dir, source.as_deref())
        })
        .await;
        let event = match result {
            Ok(data) => AgentEvent::ok(id, tn, Some(data)),
            Err(join_err) => AgentEvent::err(id, tn, format!("refresh task failed: {join_err}")),
        };
        emit_response_or_frame_limit_error(ctx, id, tn, event).await;
        return Some(false);
    }
    match query_response_data_result(cmd, ctx) {
        Ok(Some(data)) => {
            emit_response_or_frame_limit_error(ctx, id, tn, AgentEvent::ok(id, tn, Some(data)))
                .await;
            return Some(false);
        }
        Err(e) => {
            let ev = AgentEvent::err(id, tn, &e);
            emit_event_to_broadcast_or_writer(ctx, &ev).await;
            return Some(false);
        }
        Ok(None) => {}
    }
    if matches!(cmd, AgentCommand::ClearHistory { .. }) {
        return Some(super::uds_dispatch_session::handle_clear_history(ctx, id, tn).await);
    }
    None
}

#[cfg(test)]
mod issue_1612_eligibility_tests {
    use super::is_resume_picker_eligible_key;

    #[test]
    fn picker_key_eligibility_is_an_exact_allowlist() {
        for allowed in ["cli:default", "cli:named-session", "chat-1700000000-2a"] {
            assert!(is_resume_picker_eligible_key(allowed), "{allowed}");
        }
        for rejected in [
            "",
            "cli:",
            "cli:a/b",
            "cli:two words",
            "chat-",
            "chat-a-1",
            "chat-1-xyz",
            "chat-1-2-extra",
            "swarm:run",
            "agent:child",
            "workflow:state",
            "other",
        ] {
            assert!(!is_resume_picker_eligible_key(rejected), "{rejected}");
        }
    }
}
