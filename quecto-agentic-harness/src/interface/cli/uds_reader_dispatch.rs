use super::uds_multi::{ClientCommand, ClientMessage, ConversationSnapshot};

pub(super) struct ReaderDispatchCtx<'a> {
    pub line: String,
    pub snapshot: &'a ConversationSnapshot,
    pub registry: &'a super::uds_ext_protocol::ClientToolRegistry,
    pub client_id: u64,
    pub cmd_tx: &'a tokio::sync::mpsc::Sender<ClientMessage>,
}

/// Dispatch one decoded client command. Returns false when the command channel closed.
pub(super) async fn dispatch(ctx: ReaderDispatchCtx<'_>) -> bool {
    if super::uds_busy_sync::intercept(&ctx.line, ctx.snapshot, ctx.registry, ctx.client_id).await {
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
    if let Some(ctrl) = super::uds_control_forward::intercept_control_forward(&ctx.line) {
        if let Some(forward_line) =
            super::uds_ext_protocol::ack_accepted_control(ctx.registry, ctx.client_id, ctrl).await
        {
            return ctx
                .cmd_tx
                .send(ClientMessage::Command(ClientCommand {
                    line: forward_line,
                    client_id: ctx.client_id,
                }))
                .await
                .is_ok();
        }
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
