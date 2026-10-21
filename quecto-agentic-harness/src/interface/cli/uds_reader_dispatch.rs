use super::uds_multi::{ClientCommand, ClientMessage};
use super::uds_session_handles::SessionReadHandles;

pub(super) struct ReaderDispatchCtx<'a> {
    pub line: String,
    pub cancel_handle: &'a super::uds_cancel::CancelHandle,
    pub turn_control: &'a super::uds_cancel::TurnControl,
    pub session: &'a SessionReadHandles,
    pub registry: &'a super::uds_ext_protocol::ClientToolRegistry,
    pub subagent_registry:
        &'a Option<crate::infrastructure::tools::agents::subagent_registry::SubagentRegistry>,
    /// The fleet teardown a busy-path `delete_all_subagents` invokes (#1938).
    pub fleet: Option<
        &'a std::sync::Arc<crate::application::subagents::use_cases::TerminateAllDelegatedAgents>,
    >,
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
    if intercept_admission_restart(&ctx) {
        return true;
    }
    if super::uds_sync::intercept(&ctx.line, ctx.session, ctx.registry, ctx.client_id).await {
        return true;
    }
    if super::uds_busy_subagents::intercept(super::uds_busy_subagents::BusySubagentCtx {
        line: &ctx.line,
        subagents: ctx.subagent_registry,
        fleet: ctx.fleet,
        clients: ctx.registry,
        client_id: ctx.client_id,
    })
    .await
    {
        return true;
    }
    if super::uds_busy_get_message::intercept(super::uds_busy_get_message::BusyCommandCtx {
        line: &ctx.line,
        session: ctx.session,
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
                if ctrl.is_steer {
                    mark_steer_and_cancel(&ctx);
                }
                permit.send(ClientMessage::Command(ClientCommand {
                    line,
                    client_id: ctx.client_id,
                    admitted: ctrl.admitted.take(),
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
        // A refused control's receipt is recorded by dispatch (#2422); best
        // effort: a full queue keeps the refusal ack, not the receipt.
        if let Some(refused) = ctrl.refused.take() {
            let _ = ctx.cmd_tx.try_send(ClientMessage::RejectedControl(refused));
        }
        super::uds_ext_protocol::ack_accepted_control(ctx.registry, ctx.client_id, ctrl).await;
        // Keep the connection alive long enough for its writer to flush the reply.
        return true;
    }

    let Ok(permit) = ctx.cmd_tx.reserve().await else {
        return false;
    };
    // A steer's images are admitted once, here (#2422).
    let admitted = super::uds::steer_images(&ctx.line);
    if admitted.is_some() {
        mark_steer_and_cancel(&ctx);
    }
    permit.send(ClientMessage::Command(ClientCommand {
        line: ctx.line,
        client_id: ctx.client_id,
        admitted,
    }));
    true
}

/// `restart_admission` (#2615) is answered here, busy or idle, on its own
/// task and off the async runtime; the controller does the restart.
fn intercept_admission_restart(ctx: &ReaderDispatchCtx<'_>) -> bool {
    use crate::interface::uds::admission::restart_controller::{COMMAND, registered, respond};
    // Most lines are prompts, some carrying images: only a line naming the
    // command is parsed.
    if !ctx.line.contains(COMMAND) {
        return false;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(ctx.line.trim()) else {
        return false;
    };
    match value.get("type").and_then(|kind| kind.as_str()) {
        Some(COMMAND) => {}
        _ => return false,
    }
    let id = value
        .get("id")
        .and_then(|id| id.as_str())
        .map(str::to_owned);
    // The count of other sessions and sub-agents the user acknowledged.
    let confirm = value
        .get("confirm")
        .and_then(serde_json::Value::as_u64)
        .and_then(|count| usize::try_from(count).ok());
    let (clients, client_id) = (ctx.registry.clone(), ctx.client_id);
    tokio::spawn(async move {
        let answered = tokio::task::spawn_blocking(move || respond(registered(), confirm)).await;
        let id = id.as_deref();
        let event = match answered {
            Ok(Ok(data)) => super::protocol::AgentEvent::ok(id, COMMAND, Some(data)),
            Ok(Err(error)) => super::protocol::AgentEvent::err(id, COMMAND, error),
            Err(error) => super::protocol::AgentEvent::err(
                id,
                COMMAND,
                format!("admission restart task failed: {error}"),
            ),
        };
        super::uds_busy_subagents::write_event(&clients, client_id, id, COMMAND, &event).await;
    });
    true
}

fn mark_steer_and_cancel(ctx: &ReaderDispatchCtx<'_>) {
    ctx.turn_control.mark_steer();
    super::uds_cancel::fire_cancel(ctx.cancel_handle);
}

#[cfg(test)]
#[path = "uds_reader_dispatch_tests.rs"]
mod tests;
