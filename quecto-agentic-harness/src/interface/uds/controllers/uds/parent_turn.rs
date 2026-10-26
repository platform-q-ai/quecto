//! A trusted root parent's turn-outcome guard on the UDS loop (#2708).
//!
//! Each outer turn a guarded session runs is admitted (`turn_admitted`
//! once its opener is committed), completed with what runs once it was
//! saved, and settled by the application runtime: `turn_outcome` after its
//! `agent_end`, or the one correction run as a turn of its own. A terminal
//! outcome fences every automatic turn until a fresh external instruction.
//! An unguarded session (no runtime) is unchanged, and so is the writer
//! path, whose frames cannot be ordered against a running turn.
use super::{AgentEvent, DispatchCtx, PromptOutcome};
use crate::application::agent_turn::use_cases::parent_outcome::{
    ActiveTurn, ParentVerdict, TurnAdmission, TurnEnd,
};
use crate::domain::agents::services::turn_outcome_policy::admissible_episode_id;
use crate::domain::agents::value_objects::turn_outcome::{
    CORRECTION_NOTE, EPISODE_ID_REFUSAL, WorkLiveness,
};
use crate::domain::conversation::value_objects::message::Message;
use crate::domain::inference::services::turn_suspension::Work;
use crate::interface::cli::uds_session::PendingMessage;
use crate::interface::uds::wire::protocol::ControlStatus;

fn guarded(ctx: &DispatchCtx<'_>) -> bool {
    ctx.broadcast_tx.is_some() && ctx.session.parent_outcome().is_some()
}

/// The episode identity of an external instruction: unchanged when the
/// session is unguarded; on a guarded one, a supplied identity of 1..=512
/// bytes as it is, a missing one minted. `Err` once an out-of-bounds one
/// was refused, before anything was committed.
pub(in crate::interface::uds::controllers) async fn episode_id(
    ctx: &mut DispatchCtx<'_>,
    id: Option<String>,
    type_name: &str,
) -> Result<Option<String>, ()> {
    match (guarded(ctx), id) {
        (false, id) => Ok(id),
        (true, Some(id)) if admissible_episode_id(&id) => Ok(Some(id)),
        (true, None) => Ok(Some(uuid::Uuid::new_v4().to_string())),
        (true, Some(id)) => {
            ctx.session
                .record_control(Some(&id), type_name, ControlStatus::Rejected);
            ctx.turn_control.client_instruction_refused();
            let event = AgentEvent::err(Some(&id), type_name, EPISODE_ID_REFUSAL);
            super::emit_event_to_broadcast_or_writer(ctx, &event).await;
            Err(())
        }
    }
}

/// An instruction's acceptance data: on a guarded session it echoes the
/// episode the instruction opens (or will open, once queued).
pub(in crate::interface::uds::controllers) fn ack_data(
    ctx: &DispatchCtx<'_>,
    mut data: serde_json::Value,
    id: Option<&str>,
) -> serde_json::Value {
    if let (true, Some(id)) = (guarded(ctx), id) {
        data["episodeId"] = serde_json::json!(id);
    }
    data
}

/// Whether `work` may run now: an automatic turn never after a terminal
/// outcome, until a fresh external instruction.
pub(super) fn admits(ctx: &DispatchCtx<'_>, work: Work) -> bool {
    match (work, ctx.session.parent_outcome()) {
        (Work::Instruction(_), _) | (Work::Automatic, None) => true,
        (Work::Automatic, Some(runtime)) => runtime.admits_automatic(),
    }
}

/// What a client's `prompt` opens on a guarded session; it is accepted
/// now, echoing its episode, rather than when it ends.
pub(super) async fn accept_prompt(
    ctx: &mut DispatchCtx<'_>,
    work: Work,
    (id, type_name): (Option<&str>, &str),
) -> Option<TurnAdmission> {
    let (Work::Instruction(_), true, Some(id)) = (work, guarded(ctx), id) else {
        return None;
    };
    if type_name != "steer" {
        let data = ack_data(ctx, serde_json::json!({}), Some(id));
        let event = AgentEvent::ok(Some(id), type_name, Some(data));
        super::emit_event_to_broadcast_or_writer(ctx, &event).await;
    }
    Some(TurnAdmission::External {
        episode_id: id.to_owned(),
    })
}

/// What a drained message opens: a queued instruction its own episode, a
/// harness note a wake of the work it reports.
pub(super) fn pending_admission(message: &PendingMessage) -> TurnAdmission {
    match message {
        PendingMessage::Control { id, .. } => TurnAdmission::External {
            episode_id: id.clone(),
        },
        PendingMessage::User { .. } => TurnAdmission::External {
            episode_id: uuid::Uuid::new_v4().to_string(),
        },
        PendingMessage::Automatic(_)
        | PendingMessage::SubagentNotification { .. }
        | PendingMessage::CoalescedSubagentNotification { .. } => TurnAdmission::WorkWake,
    }
}

/// Admit the turn `opener` opens, on a guarded session.
pub(super) async fn admit(
    ctx: &DispatchCtx<'_>,
    admission: Option<TurnAdmission>,
    opener: &Message,
) -> Option<ActiveTurn> {
    let (Some(admission), true) = (admission, guarded(ctx)) else {
        return None;
    };
    let runtime = ctx.session.parent_outcome()?;
    let session_key = ctx.sessions.current_session_key().await;
    runtime.admit(&session_key, admission, &opener.id().to_string())
}

/// Announce an admitted turn whose opener was just committed.
pub(super) async fn announce(
    broadcast_tx: &Option<tokio::sync::broadcast::Sender<String>>,
    turn: &Option<ActiveTurn>,
) {
    if let (Some(tx), Some(turn)) = (broadcast_tx, turn) {
        let event = AgentEvent::TurnAdmitted(turn.admission());
        super::EventSink::Broadcast(tx.clone()).emit(&event).await;
    }
}

/// Whether anything this root owns still runs: its sub-agents, a queued
/// instruction or note, a note not yet queued.
fn observe(ctx: &DispatchCtx<'_>) -> WorkLiveness {
    use crate::infrastructure::tools::agents::owned_work::registry_liveness;
    let queued = ctx.session.holds_pending_work()
        || ctx
            .notification_rx
            .as_ref()
            .is_some_and(|rx| !rx.is_empty());
    let queued = match queued {
        true => WorkLiveness::Live,
        false => WorkLiveness::Empty,
    };
    registry_liveness(ctx.subagent_registry.as_ref()).and(queued)
}

/// Settle a guarded turn that ran: publish its outcome after its
/// `agent_end`, or run the one correction and settle that.
pub(super) async fn settle(
    ctx: &mut DispatchCtx<'_>,
    mut turn: Option<ActiveTurn>,
    mut outcome: PromptOutcome,
) -> PromptOutcome {
    loop {
        let (Some(active), Some(runtime)) = (turn.take(), ctx.session.parent_outcome()) else {
            return outcome;
        };
        let end = match outcome {
            PromptOutcome::Success => TurnEnd::Completed(observe(ctx)),
            PromptOutcome::Error | PromptOutcome::Cancelled => TurnEnd::Interrupted,
        };
        match runtime.complete(active, end) {
            ParentVerdict::Unsettled => return outcome,
            ParentVerdict::Published(record) => {
                super::emit_event_to_broadcast_or_writer(ctx, &AgentEvent::TurnOutcome(record))
                    .await;
                crate::interface::cli::uds_snapshots::refresh_state_snapshot(ctx).await;
                return outcome;
            }
            ParentVerdict::Correct => {
                let note = crate::domain::conversation::services::turn_origin::harness_note(
                    CORRECTION_NOTE.into(),
                    ctx.messages,
                );
                turn = admit(ctx, Some(TurnAdmission::Correction), &note).await;
                assert!(turn.is_some(), "the correction the policy owes is admitted");
                let armed = crate::interface::uds::controllers::uds_cancel::arm_swarm_cancel(
                    &ctx.cancel_handle,
                    &ctx.turn_control,
                )
                .await;
                outcome = match armed {
                    Some(cancel_rx) => {
                        super::run_turn(
                            ctx,
                            note,
                            cancel_rx,
                            super::TurnAdmissionGuard::None,
                            &turn,
                        )
                        .await
                    }
                    None => PromptOutcome::Cancelled,
                };
            }
        }
    }
}

#[cfg(test)]
#[path = "parent_turn_tests.rs"]
mod issue_2708_uds_parent_tests;
