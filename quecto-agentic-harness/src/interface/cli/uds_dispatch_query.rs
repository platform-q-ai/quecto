use super::super::uds_query::query_response_data_result;
use super::{
    AgentCommand, AgentEvent, DispatchCtx, emit_event_to_broadcast_or_writer,
    emit_response_or_frame_limit_error, emit_response_or_frame_limit_error_with_message,
};
use crate::interface::uds::sessions::recover_message_controller::GetMessageFields;

/// Presentation of a summary's raw title datum: the "(untitled)"
/// placeholder and display truncation belong to this edge, not to
/// persistence.
pub(super) fn display_title(raw: &str) -> String {
    const MAX_CHARS: usize = 50;
    if raw.is_empty() {
        return "(untitled)".to_string();
    }
    if raw.chars().count() <= MAX_CHARS {
        return raw.to_string();
    }
    let mut out: String = raw.chars().take(MAX_CHARS).collect();
    out.push('…');
    out
}

#[path = "uds_dispatch_discovery.rs"]
mod uds_dispatch_discovery;
pub(super) use uds_dispatch_discovery::{discovery_json, safe_display};

pub(super) fn session_summary_to_json(
    summary: &crate::domain::session::SessionSummary,
) -> serde_json::Value {
    serde_json::json!({
        "key": summary.key,
        "title": display_title(&safe_display(&summary.title)),
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
    // Export a retained session report (#1859, #1974): the composed report
    // owner selects, previews and (on request) exports; this edge maps
    // `export_raw` and presents the result. The loop serialises requests,
    // so no admission is taken here.
    if let AgentCommand::GetReport { export_raw, .. } = cmd {
        let event = super::super::uds_latest_report::report_event(
            id,
            ctx.sessions.export_report.report(*export_raw).await,
        );
        emit_event_to_broadcast_or_writer(ctx, &event).await;
        return Some(false);
    }
    // List saved sessions (#1861): invoked through the composed controller
    // and presented here; the scope and order are the application's and
    // the store's, never decided at this edge.
    if let AgentCommand::ListSessions { scope, .. } = cmd {
        super::uds_dispatch_session::handle_list_sessions(ctx, id, tn, *scope).await;
        return Some(false);
    }
    // Recover full message/tool-call content (#1858, #1971): the composed
    // recovery owner resolves the ref (ledger full copy before a
    // possibly-collapsed live entry, #1060 review 1a; retention store behind
    // a stub) with the loop's own conversation as fallback; this edge only
    // frames and presents. Every miss is the same structured error.
    if let AgentCommand::GetMessage {
        message_id,
        tool_call_id,
        offset,
        thinking_offset,
        limit,
        ..
    } = cmd
    {
        let recovered = ctx
            .sessions
            .recover_message
            .recover(
                GetMessageFields {
                    message_id,
                    tool_call_id: tool_call_id.as_deref(),
                    offset: *offset,
                    thinking_offset: *thinking_offset,
                    limit: *limit,
                },
                ctx.messages,
            )
            .await;
        let data = recovered
            .ok()
            .and_then(|content| super::super::uds_session::recovered_content_json(&content, id));
        let ev = match data {
            Some(data) => AgentEvent::ok(id, tn, Some(data)),
            None => AgentEvent::err(id, tn, format!("message not found: {message_id}")),
        };
        emit_response_or_frame_limit_error(ctx, id, tn, ev).await;
        return Some(false);
    }
    // Synchronize a client transcript (#1857, #1973): the composed
    // controller reconciles the client's position; this edge frames the
    // typed result and presents it (the busy reader task shares the
    // presenter).
    if let AgentCommand::Sync {
        epoch, since_rev, ..
    } = cmd
    {
        let data = super::super::uds_sync::sync_data(
            &ctx.sessions.synchronize_transcript,
            *epoch,
            *since_rev,
        )
        .await;
        emit_response_or_frame_limit_error_with_message(
            ctx,
            id,
            tn,
            AgentEvent::ok(id, tn, Some(data)),
            super::super::uds_sync::SYNC_OVERSIZED_ERROR,
        )
        .await;
        return Some(false);
    }
    // The fleet teardown (#1938): invoked and presented, never orchestrated
    // here. Idle path only; a busy harness answers from the reader task.
    if matches!(cmd, AgentCommand::DeleteAllSubagents { .. }) {
        let fleet = ctx.fleet_teardown.clone();
        let event = super::super::uds_delete_all_subagents::respond(fleet.as_ref(), id).await;
        emit_response_or_frame_limit_error(ctx, id, tn, event).await;
        return Some(false);
    }
    // Catalogue refresh performs (sequential, bounded) blocking HTTP; running
    // it inline would freeze every other UDS command for the whole run, so it
    // executes on a dedicated blocking worker thread while the dispatch loop
    // stays responsive (slice-4 review). `reqwest::blocking` is also only
    // safe off the async runtime's core threads.
    if let AgentCommand::RefreshModels { source, .. } = cmd {
        let refresh = ctx.catalogue.refresh.clone();
        let source = source.clone();
        let result = tokio::task::spawn_blocking(move || {
            super::super::uds_models::refresh_models_data(&refresh, source.as_deref())
        })
        .await;
        let event = match result {
            Ok(data) => AgentEvent::ok(id, tn, Some(data)),
            Err(join_err) => AgentEvent::err(id, tn, format!("refresh task failed: {join_err}")),
        };
        emit_response_or_frame_limit_error(ctx, id, tn, event).await;
        return Some(false);
    }
    let session_key = ctx.sessions.current_session_key().await;
    match query_response_data_result(cmd, ctx, &session_key) {
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
