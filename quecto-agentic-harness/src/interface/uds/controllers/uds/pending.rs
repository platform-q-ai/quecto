//! Pending turn delivery yields to admitted explicit steering.
use super::{DispatchCtx, run_drained_message};

// The workflow-engine poison tests (#2192) build on the dispatch test
// environment beside this module; declared here, at the top, so a test
// module another change appends at the end never touches these lines.
#[cfg(test)]
mod workflow_poison_tests;

pub(super) async fn drain_and_run_pending(ctx: &mut DispatchCtx<'_>) {
    let _busy = crate::interface::uds::framing::uds_multi::BusyGuard::new(&ctx.busy); // #828
    'drain: loop {
        // Coalesce a burst of buffered sub-agent completion notes into ONE
        // informational note at the idle flush (#894) — K separate notes would
        // send the parent into a catch-up loop. Per-agent dedup already happened
        // at enqueue; #816 deferral is preserved (this only runs at idle).
        // The reader has admitted an explicit replacement instruction. Yield
        // to command dispatch before consuming buffered automated work.
        if ctx.turn_control.is_steer_pending() {
            return;
        }
        // #2531: a recoverable failure is re-armed by the next automatic
        // note (bounded), so the domain decides, not the allowed flag.
        let Some(pending) = take_admissible_pending(ctx) else {
            return;
        };
        let pending = crate::interface::cli::uds_session::coalesce_pending(pending);
        if pending.is_empty() {
            break;
        }
        let mut remaining = pending.into_iter();
        while let Some(pending_msg) = remaining.next() {
            // The domain decides which work a suspended member admits.
            let work = work_of(&pending_msg);
            if ctx.turn_control.is_steer_pending() || !ctx.session.automatic_turns().admits(work) {
                ctx.session
                    .restore_pending(std::iter::once(pending_msg).chain(remaining));
                return;
            }
            // Earlier work in this batch may have settled terminal. Recheck
            // each item; a stale note must not hide an instruction behind it.
            let fresh = super::parent_turn::fresh_wake_allowed(ctx)
                && matches!(
                super::parent_turn::pending_admission(&pending_msg),
                crate::application::agent_turn::use_cases::parent_outcome::TurnAdmission::FreshWake
            );
            if matches!(
                (super::parent_turn::admits(ctx, work), fresh),
                (false, false)
            ) {
                ctx.session
                    .restore_pending(std::iter::once(pending_msg).chain(remaining));
                continue 'drain;
            }
            if let Some(control) = &ctx.turn_control.swarm_control {
                use crate::domain::inference::services::turn_suspension::Work;
                use crate::domain::swarm::{
                    value_objects::lifecycle::RunStatus,
                    value_objects::run_control::RunControlAction,
                };
                let status = control.apply(RunControlAction::Status).await;
                if let Ok(receipt) = &status {
                    ctx.turn_control
                        .observe_control_generation(receipt.generation);
                    ctx.session
                        .automatic_turns()
                        .observe_control_generation(Some(receipt.generation));
                }
                let explicit = matches!(work, Work::Instruction(_));
                let admission = match status {
                    Ok(receipt) => match receipt.status {
                        RunStatus::Setup | RunStatus::Running => Some(true),
                        // A run the coordinator ended still takes explicit
                        // instructions so it can report (#1729); a plain
                        // pause keeps everything queued.
                        RunStatus::Paused if receipt.outcome.is_some_and(RunStatus::proposable) => {
                            Some(explicit)
                        }
                        RunStatus::Paused => None,
                        RunStatus::Succeeded
                        | RunStatus::Blocked
                        | RunStatus::Failed
                        | RunStatus::Cancelled
                        | RunStatus::BudgetExhausted => Some(explicit),
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
            ctx.session.automatic_turns().start_turn(work, None);
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
                    crate::interface::uds::wire::protocol::ControlStatus::Started,
                );
            }
            let admission = super::parent_turn::pending_admission(&pending_msg);
            let message = pending_msg.into_message(ctx.messages);
            let outcome = run_drained_message(ctx, message, Some(admission)).await;
            if let Some((id, command)) = &correlation {
                ctx.session
                    .record_control(Some(id), command, super::control_status(&outcome));
                crate::interface::cli::uds_snapshots::refresh_busy_snapshots(ctx).await;
            }
            // A failure during this drain stops the batch: the rest is
            // retained, never retried against a failing provider.
            if !ctx.session.automatic_turns_allowed() {
                ctx.session.restore_pending(remaining);
                return;
            }
        }
    }
}

/// Admit instructions and trusted fresh notes independently: stale internal
/// activity remains queued without hiding fresh input behind it or spinning.
/// Automatic suspension remains authoritative for every kind of note.
fn take_admissible_pending(
    ctx: &mut DispatchCtx<'_>,
) -> Option<Vec<crate::interface::cli::uds_session::PendingMessage>> {
    let (explicit, automatic): (Vec<_>, Vec<_>) = ctx
        .session
        .drain_pending()
        .into_iter()
        .partition(|message| {
            let work = work_of(message);
            ctx.session.automatic_turns().admits(work)
                && (super::parent_turn::admits(ctx, work)
                    || (super::parent_turn::fresh_wake_allowed(ctx) && matches!(super::parent_turn::pending_admission(message),
                        crate::application::agent_turn::use_cases::parent_outcome::TurnAdmission::FreshWake)))
        });
    ctx.session.restore_pending(automatic.into_iter());
    if explicit.is_empty() {
        None
    } else {
        Some(explicit)
    }
}

/// The work a queued message is (#1909): human or parent intent (a prompt
/// or a queued control) is an admitted follow-up instruction; harness
/// notes are automatic work. The domain decides what each may do.
fn work_of(
    message: &crate::interface::cli::uds_session::PendingMessage,
) -> crate::domain::inference::services::turn_suspension::Work {
    use crate::domain::inference::services::turn_suspension::Work;
    use crate::interface::cli::uds_session::PendingMessage;
    match message {
        PendingMessage::User { .. } | PendingMessage::Control { .. } => Work::FOLLOW_UP,
        PendingMessage::Automatic(_)
        | PendingMessage::SubagentNotification { .. }
        | PendingMessage::CoalescedSubagentNotification { .. } => Work::Automatic,
    }
}

/// A second-stage dispatch acknowledgment must reflect actual retention.
pub(super) async fn queue_prompt(
    ctx: &mut DispatchCtx<'_>,
    id: Option<&str>,
    type_name: &str,
    message: crate::interface::cli::uds_session::PromptBody,
    work: crate::domain::inference::services::turn_suspension::Work,
) -> bool {
    use crate::domain::inference::services::turn_suspension::Work;
    let type_name = if work.is_steer() { "steer" } else { type_name };
    if work.is_steer() {
        ctx.turn_control.consume_steer();
    }
    let retained = ctx.session.enqueue_control(id, type_name, message, work);
    // Only a client's instruction was counted as taken (#2467).
    if let (false, Work::Instruction(_)) = (retained, work) {
        ctx.turn_control.client_instruction_refused();
    }
    ctx.session.record_control(
        id,
        type_name,
        if retained {
            crate::interface::uds::wire::protocol::ControlStatus::Queued
        } else {
            crate::interface::uds::wire::protocol::ControlStatus::Rejected
        },
    );
    let event = if retained {
        let data = super::ack_data(ctx, serde_json::json!({"status":"queued"}), id);
        super::AgentEvent::ok(id, type_name, Some(data))
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

#[cfg(test)]
mod persist_tests;

#[cfg(test)]
mod swarm_state_tests;
