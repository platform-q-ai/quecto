//! A trusted root parent's turn-outcome guard on the UDS loop (#2708): each
//! outer turn of a guarded session is admitted, completed once saved, and
//! settled (`turn_outcome`, or the one correction). An unguarded session is
//! unchanged.
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
    ctx.session.parent_outcome().is_some()
}

/// The identity of an external instruction: unchanged when the session is
/// unguarded or none was supplied (its episode is minted at admission); on
/// a guarded one a supplied identity of 1..=512 bytes as it is. `Err` once
/// an out-of-bounds one was refused, before anything was committed.
pub(in crate::interface::uds::controllers) async fn episode_id(
    ctx: &mut DispatchCtx<'_>,
    id: Option<String>,
    type_name: &str,
) -> Result<Option<String>, ()> {
    match (guarded(ctx), id) {
        (false, id) | (true, id @ None) => Ok(id),
        (true, Some(id)) if admissible_episode_id(&id) => Ok(Some(id)),
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

/// What a client's `prompt` opens on a guarded session: the episode it
/// named, or one minted for it, acknowledged now rather than when it ends.
pub(super) async fn accept_prompt(
    ctx: &mut DispatchCtx<'_>,
    work: Work,
    (id, type_name): (Option<&str>, &str),
) -> Option<TurnAdmission> {
    let (Work::Instruction(_), true) = (work, guarded(ctx)) else {
        return None;
    };
    let episode_id = id.map_or_else(|| uuid::Uuid::new_v4().to_string(), str::to_owned);
    if type_name != "steer" {
        let data = serde_json::json!({ "episodeId": episode_id });
        let event = AgentEvent::ok(id, type_name, Some(data));
        super::emit_event_to_broadcast_or_writer(ctx, &event).await;
    }
    Some(TurnAdmission::External { episode_id })
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

/// Whether anything this root owns still runs: its sub-agents, a queued
/// instruction or note, a note not yet queued.
fn observe(ctx: &DispatchCtx<'_>) -> WorkLiveness {
    use crate::infrastructure::tools::agents::owned_work::registry_liveness;
    let queued = ctx.session.holds_pending_work()
        || ctx
            .notification_rx
            .as_ref()
            .is_some_and(|rx| !rx.is_empty());
    registry_liveness(ctx.subagent_registry.as_ref()).and(WorkLiveness::of(queued))
}

/// A guarded session stays busy across back-to-back turns (#2708).
pub(in crate::interface::uds::controllers) fn busy_across_turns(ctx: &DispatchCtx) -> Held {
    guarded(ctx).then(|| BusyGuard::new(&ctx.busy))
}

/// A guarded loop is busy from taking an instruction until no command is
/// queued behind it; any other message (a state change) ends that (#2708).
pub(in crate::interface) fn busy_for(ctx: &DispatchCtx, msg: &ClientMessage, held: &mut Held) {
    let keep = match (guarded(ctx), msg) {
        (true, ClientMessage::Command(command)) => {
            let kind: serde_json::Value = serde_json::from_str(&command.line).unwrap_or_default();
            matches!(
                kind["type"].as_str(),
                Some("prompt" | "steer" | "follow_up")
            )
        }
        _ => false,
    };
    *held = keep.then(|| held.take().unwrap_or_else(|| BusyGuard::new(&ctx.busy)));
}
use crate::interface::uds::framing::uds_multi::{BusyGuard, ClientMessage};
type Held = Option<BusyGuard>;

/// Run a client's prompt: guarded, admitted, run and settled; unguarded,
/// exactly as before the guard, one `run_agent_message`.
pub(super) async fn dispatch_prompt(
    ctx: &mut DispatchCtx<'_>,
    message: Message,
    cancel_rx: tokio::sync::oneshot::Receiver<()>,
    admission: Option<TurnAdmission>,
) -> PromptOutcome {
    let _busy = BusyGuard::new(&ctx.busy); // #828
    let Some(turn) = admit(ctx, admission, &message).await else {
        let mut sink = super::make_event_sink(&ctx.broadcast_tx, &mut ctx.stdout, &ctx.wire_mode);
        return super::run_agent_message(super::PromptRun {
            agent: ctx.agent,
            messages: ctx.messages,
            active_session: Some(ctx.sessions.active_session.clone()),
            execution_state: Some(ctx.execution_state.clone()),
            session: ctx.session,
            sink: &mut sink,
            message,
            system_prompt: ctx.system_prompt,
            cancel_rx,
            notification_rx: &mut ctx.notification_rx,
            subagent_registry: &ctx.subagent_registry,
            turn_save: Some(super::TurnSave(ctx.save_session.clone())),
            admitted: None,
        })
        .await;
    };
    let turn = Some(turn);
    let guard = super::TurnAdmissionGuard::None;
    let outcome = super::run_turn(ctx, message, cancel_rx, guard, &turn).await;
    settle(ctx, turn, outcome).await
}

/// Settle a guarded turn that ran: publish its outcome after its
/// `agent_end`, or run the one correction and settle that.
pub(super) async fn settle(
    ctx: &mut DispatchCtx<'_>,
    mut turn: Option<ActiveTurn>,
    outcome: PromptOutcome,
) -> PromptOutcome {
    // The prompt's own result is what its receipt reports, whatever a
    // correction after it does.
    let mut completed = matches!(outcome, PromptOutcome::Success);
    loop {
        let (Some(active), Some(runtime)) = (turn.take(), ctx.session.parent_outcome()) else {
            return outcome;
        };
        let end = match completed {
            true => TurnEnd::Completed(observe(ctx)),
            false => TurnEnd::Interrupted,
        };
        match runtime.complete(active, end) {
            ParentVerdict::Unsettled => return outcome,
            ParentVerdict::Published(record) => {
                // Snapshot first: a client reacting to the frame reads it.
                crate::interface::cli::uds_snapshots::refresh_state_snapshot(ctx).await;
                super::emit_event_to_broadcast_or_writer(ctx, &AgentEvent::TurnOutcome(record))
                    .await;
                return outcome;
            }
            ParentVerdict::Correct => {
                let mut note = crate::domain::conversation::services::turn_origin::harness_note(
                    CORRECTION_NOTE.into(),
                    ctx.messages,
                );
                turn = admit(ctx, Some(TurnAdmission::Correction), &note).await;
                assert!(turn.is_some(), "the correction the policy owes is admitted");
                // As any drained automatic turn: started as such, saved first.
                ctx.session
                    .automatic_turns()
                    .start_turn(Work::Automatic, None);
                if let Err(err) = super::persist_user_prompt_before_run(ctx, &mut note).await {
                    tracing::warn!("failed to persist the correction before its turn: {err}");
                }
                let armed = crate::interface::uds::controllers::uds_cancel::arm_swarm_cancel(
                    &ctx.cancel_handle,
                    &ctx.turn_control,
                )
                .await;
                let guard = super::TurnAdmissionGuard::None;
                completed = match armed {
                    Some(cancel_rx) => matches!(
                        super::run_turn(ctx, note, cancel_rx, guard, &turn).await,
                        PromptOutcome::Success
                    ),
                    None => false,
                };
            }
        }
    }
}

#[cfg(test)]
mod issue_2708_uds_parent_tests;
#[cfg(test)]
mod review_2762_tests;
#[cfg(test)]
mod run16_lifecycle_tests;
