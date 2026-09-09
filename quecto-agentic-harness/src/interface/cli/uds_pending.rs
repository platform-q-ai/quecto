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
        if ctx.turn_control.is_steer_pending() {
            return;
        }
        let pending = if ctx.session.automatic_turns_allowed {
            crate::interface::cli::uds_session::coalesce_pending(ctx.session.drain_pending())
        } else {
            // #1712: a suspended member still obeys explicit instructions
            // (a parent's fast-acked prompt arrives as a queued follow-up);
            // an admitted one re-arms it, while buffered automatic notes wait.
            let Some(explicit) = take_explicit_instructions(ctx) else {
                return;
            };
            explicit
        };
        if pending.is_empty() {
            break;
        }
        let mut remaining = pending.into_iter();
        while let Some(pending_msg) = remaining.next() {
            let explicit = is_explicit(&pending_msg);
            if ctx.turn_control.is_steer_pending()
                || (!ctx.session.automatic_turns_allowed && !explicit)
            {
                ctx.session
                    .restore_pending(std::iter::once(pending_msg).chain(remaining));
                return;
            }
            if let Some(control) = &ctx.turn_control.swarm_control {
                use crate::domain::swarm::{RunControlAction, RunStatus};
                use crate::interface::cli::uds_session::PendingMessage;
                let status = control.apply(RunControlAction::Status).await;
                if let Ok(receipt) = &status {
                    ctx.turn_control
                        .observe_control_generation(receipt.generation);
                    ctx.session
                        .observe_control_generation(Some(receipt.generation));
                }
                let admission = match status {
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
            // #1712: only an explicit instruction the run admits re-arms a
            // suspended member; a paused run or a failed probe keeps both the
            // suspension and the instruction.
            if !ctx.session.automatic_turns_allowed {
                ctx.session.resume_automatic_turns();
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
            // A failure during this drain stops the batch: the rest is
            // retained, never retried against a failing provider.
            if !ctx.session.automatic_turns_allowed {
                ctx.session.restore_pending(remaining);
                return;
            }
        }
    }
}

/// Split the pending queue: explicit instructions (user prompts, queued
/// controls) are returned when there are any; automatic notes go back to
/// the queue untouched. `None` when nothing explicit is waiting.
fn take_explicit_instructions(
    ctx: &mut DispatchCtx<'_>,
) -> Option<Vec<crate::interface::cli::uds_session::PendingMessage>> {
    let (explicit, automatic): (Vec<_>, Vec<_>) = ctx
        .session
        .drain_pending()
        .into_iter()
        .partition(is_explicit);
    ctx.session.restore_pending(automatic.into_iter());
    if explicit.is_empty() {
        None
    } else {
        Some(explicit)
    }
}

/// Human or parent intent (a prompt or a queued control), as opposed to
/// harness-generated notes and nudges.
fn is_explicit(message: &crate::interface::cli::uds_session::PendingMessage) -> bool {
    use crate::interface::cli::uds_session::PendingMessage;
    matches!(
        message,
        PendingMessage::User(_) | PendingMessage::Control { .. }
    )
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
