use super::uds_multi::{ClientCommand, ClientMessage, ConversationSnapshot};

pub(super) struct ReaderDispatchCtx<'a> {
    pub line: String,
    pub cancel_handle: &'a super::uds_cancel::CancelHandle,
    pub turn_control: &'a super::uds_cancel::TurnControl,
    pub snapshot: &'a ConversationSnapshot,
    pub registry: &'a super::uds_ext_protocol::ClientToolRegistry,
    pub subagent_registry:
        &'a Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
    /// Broadcast sender so busy-path `delete_all_subagents` can publish the
    /// empty survivor set to every client (#1626).
    pub broadcast_tx: &'a tokio::sync::broadcast::Sender<String>,
    pub client_id: u64,
    pub cmd_tx: &'a tokio::sync::mpsc::Sender<ClientMessage>,
}

/// Dispatch one decoded client command. Returns false when the command channel closed.
pub(super) async fn dispatch(ctx: ReaderDispatchCtx<'_>) -> bool {
    if super::uds_swarm_control::intercept(&ctx).await {
        return true;
    }
    // Explicit abort is independent of queue admission. Steering is a
    // replacement instruction and may interrupt only after capacity is held.
    if super::uds::is_abort_command(&ctx.line) {
        ctx.turn_control.mark_abort();
        super::uds_cancel::fire_cancel(ctx.cancel_handle);
    }
    if super::uds_busy_sync::intercept(&ctx.line, ctx.snapshot, ctx.registry, ctx.client_id).await {
        return true;
    }
    if super::uds_busy_subagents::intercept(super::uds_busy_subagents::BusySubagentCtx {
        line: &ctx.line,
        subagents: ctx.subagent_registry,
        broadcast_tx: ctx.broadcast_tx,
        clients: ctx.registry,
        client_id: ctx.client_id,
    })
    .await
    {
        return true;
    }
    if super::uds_busy_get_message::intercept(super::uds_busy_get_message::BusyCommandCtx {
        line: &ctx.line,
        snapshot: ctx.snapshot,
        registry: ctx.registry,
        client_id: ctx.client_id,
    })
    .await
    {
        return true;
    }
    if let Some(mut ctrl) = super::uds_control_forward::intercept_control_forward(&ctx.line) {
        if let Some(line) = ctrl.forward_line.take() {
            // Acceptance means retained by dispatch, not merely read from a socket.
            // Never acknowledge work before queue admission, or wait indefinitely
            // behind a full queue while the sender believes delivery succeeded.
            if let Ok(permit) = ctx.cmd_tx.try_reserve() {
                cancel_for_admitted_steer(&ctx);
                permit.send(ClientMessage::Command(ClientCommand {
                    line,
                    client_id: ctx.client_id,
                }));
            } else {
                let request: serde_json::Value =
                    serde_json::from_str(&ctx.line).expect("validated control JSON");
                ctrl.ack_line = super::protocol::AgentEvent::err(
                    request["id"].as_str(), request["type"].as_str().unwrap_or("prompt"),
                    "control queue is full or closed; message was not queued; inspect agent state before retrying",
                ).to_json_line() + "\n";
            }
        }
        super::uds_ext_protocol::ack_accepted_control(ctx.registry, ctx.client_id, ctrl).await;
        // Keep the connection alive long enough for its writer to flush the reply.
        return true;
    }

    let Ok(permit) = ctx.cmd_tx.reserve().await else {
        return false;
    };
    cancel_for_admitted_steer(&ctx);
    permit.send(ClientMessage::Command(ClientCommand {
        line: ctx.line,
        client_id: ctx.client_id,
    }));
    true
}

fn cancel_for_admitted_steer(ctx: &ReaderDispatchCtx<'_>) {
    if super::uds::is_steer_command(&ctx.line) {
        ctx.turn_control.mark_steer();
        super::uds_cancel::fire_cancel(ctx.cancel_handle);
    }
}

#[cfg(test)]
#[path = "uds_reader_dispatch_tests.rs"]
mod tests;
