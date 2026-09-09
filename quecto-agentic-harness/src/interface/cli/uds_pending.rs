//! Pending turn delivery yields to admitted explicit steering.
use super::{DispatchCtx, run_drained_message};

pub(super) async fn drain_and_run_pending(ctx: &mut DispatchCtx<'_>) {
    let _busy = crate::interface::cli::uds_multi::BusyGuard::new(&ctx.busy); // #828
    loop {
        // Coalesce a burst of buffered sub-agent completion notes into ONE
        // informational note at the idle flush (#894) — K separate notes would
        // send the parent into a catch-up loop. Per-agent dedup already happened
        // at enqueue; #816 deferral is preserved (this only runs at idle).
        // The reader has admitted an explicit replacement instruction. Yield
        // to command dispatch before consuming buffered automated work.
        if !ctx.session.automatic_turns_allowed || ctx.turn_control.is_steer_pending() {
            return;
        }
        let pending =
            crate::interface::cli::uds_session::coalesce_pending(ctx.session.drain_pending());
        if pending.is_empty() {
            break;
        }
        let mut remaining = pending.into_iter();
        while let Some(pending_msg) = remaining.next() {
            if !ctx.session.automatic_turns_allowed || ctx.turn_control.is_steer_pending() {
                ctx.session
                    .restore_pending(std::iter::once(pending_msg).chain(remaining));
                return;
            }
            if let Some(control) = &ctx.turn_control.swarm_control {
                use crate::domain::swarm::{RunControlAction, RunStatus};
                use crate::interface::cli::uds_session::PendingMessage;
                let admission = match control.apply(RunControlAction::Status).await {
                    Ok(receipt) => match receipt.status {
                        RunStatus::Setup | RunStatus::Running => Some(true),
                        RunStatus::Paused => None,
                        RunStatus::Succeeded
                        | RunStatus::Blocked
                        | RunStatus::Failed
                        | RunStatus::Cancelled
                        | RunStatus::BudgetExhausted => Some(matches!(
                            pending_msg,
                            PendingMessage::User(_) | PendingMessage::Control { .. }
                        )),
                    },
                    Err(error) => {
                        let event =
                            super::AgentEvent::err(None, "pending_admission", error.to_string());
                        super::emit_event_to_broadcast_or_writer(ctx, &event).await;
                        None
                    }
                };
                match admission {
                    Some(true) => {}
                    Some(false) => continue,
                    None => {
                        ctx.session
                            .restore_pending(std::iter::once(pending_msg).chain(remaining));
                        return;
                    }
                }
            }
            let correlation = match &pending_msg {
                crate::interface::cli::uds_session::PendingMessage::Control {
                    id, command, ..
                } => Some((id.clone(), command.clone())),
                _ => None,
            };
            if let Some((id, command)) = &correlation {
                ctx.session.record_control(
                    Some(id),
                    command,
                    super::super::protocol::ControlStatus::Started,
                );
            }
            let outcome = run_drained_message(ctx, pending_msg.into_message()).await;
            if let Some((id, command)) = &correlation {
                ctx.session
                    .record_control(Some(id), command, super::control_status(&outcome));
                super::super::uds_snapshots::refresh_busy_snapshots(ctx).await;
            }
        }
    }
}

/// A second-stage dispatch acknowledgment must reflect actual retention.
pub(super) async fn queue_prompt(
    ctx: &mut DispatchCtx<'_>,
    id: Option<&str>,
    type_name: &str,
    message: String,
    steer: bool,
) -> bool {
    let type_name = if steer { "steer" } else { type_name };
    if steer {
        ctx.turn_control.consume_steer();
    }
    let retained = ctx.session.enqueue_control(id, type_name, message, steer);
    ctx.session.record_control(
        id,
        type_name,
        if retained {
            super::super::protocol::ControlStatus::Queued
        } else {
            super::super::protocol::ControlStatus::Rejected
        },
    );
    let event = if retained {
        super::AgentEvent::ok(id, type_name, Some(serde_json::json!({"status":"queued"})))
    } else {
        super::AgentEvent::err(
            id,
            type_name,
            "pending prompt queue is full; instruction was not retained",
        )
    };
    super::emit_event_to_broadcast_or_writer(ctx, &event).await;
    retained
}
