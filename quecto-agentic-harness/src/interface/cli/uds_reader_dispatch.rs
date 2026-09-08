use super::uds_multi::{ClientCommand, ClientMessage, ConversationSnapshot};

pub(super) struct ReaderDispatchCtx<'a> {
    pub line: String,
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
            if ctx
                .cmd_tx
                .try_send(ClientMessage::Command(ClientCommand {
                    line,
                    client_id: ctx.client_id,
                }))
                .is_err()
            {
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

    ctx.cmd_tx
        .send(ClientMessage::Command(ClientCommand {
            line: ctx.line,
            client_id: ctx.client_id,
        }))
        .await
        .is_ok()
}

#[cfg(test)]
#[path = "uds_reader_dispatch_tests.rs"]
mod tests;
