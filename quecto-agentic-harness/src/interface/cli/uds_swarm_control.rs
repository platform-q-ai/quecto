//! Typed supervisor control independent of model execution and queued prompts.
use super::uds_reader_dispatch::ReaderDispatchCtx;
use crate::domain::swarm::{RunControlAction, RunStatus};
use serde::Deserialize;

/// Type name of the harness's own wake nudges: never a human instruction.
pub(super) const SWARM_WAKE: &str = "swarm_wake";

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    Pause,
    Resume,
    Close,
    Extend,
    Status,
    Wake,
    /// #2390: another member's harness changed the run's control state;
    /// only this process's run watch is nudged.
    Watch,
    UsageBudget,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Command {
    SwarmControl {
        id: Option<String>,
        action: Action,
        reason: Option<String>,
        generation: Option<u64>,
        agent_id: Option<String>,
        token_limit: Option<u64>,
        strict_unknown: Option<bool>,
        deadline_seconds: Option<u64>,
    },
}

pub(super) async fn intercept(ctx: &ReaderDispatchCtx<'_>) -> bool {
    let Ok(Command::SwarmControl {
        id,
        action,
        reason,
        generation,
        agent_id,
        token_limit,
        strict_unknown,
        deadline_seconds,
    }) = serde_json::from_str(&ctx.line)
    else {
        return false;
    };
    if let Some(agent_id) = agent_id {
        let command: serde_json::Value =
            serde_json::from_str(&ctx.line).expect("validated command");
        let subagents = ctx.subagent_registry.clone();
        let clients = ctx.registry.clone();
        let client_id = ctx.client_id;
        tokio::spawn(async move {
            let event = super::uds_busy_subagents::forward_child_command(
                &subagents,
                id.as_deref(),
                &agent_id,
                command,
            )
            .await;
            super::uds_busy_subagents::write_event(
                &clients,
                client_id,
                id.as_deref(),
                "swarm_control",
                &event,
            )
            .await;
        });
        return true;
    }
    if matches!(action, Action::Watch) {
        // #2390: the watch reads the board now. Nothing else: no control
        // action, no model turn, no queued prompt or wake, and the wake
        // cursor is left as it is.
        let event = match ctx.turn_control.swarm_control.as_ref() {
            Some(control) => {
                control.nudge_watch();
                super::protocol::AgentEvent::ok(
                    id.as_deref(),
                    "swarm_control",
                    Some(serde_json::json!({"status":"nudged"})),
                )
            }
            None => super::protocol::AgentEvent::err(
                id.as_deref(),
                "swarm_control",
                "watch unavailable",
            ),
        };
        if let Some(writer) = super::uds_ext_protocol::client_writer_tx(ctx.registry, ctx.client_id)
        {
            let _ = writer.send(event.to_json_line() + "\n").await;
        }
        return true;
    }
    if matches!(action, Action::Wake) {
        // #2390 (d): a wake hint whose generation validates also nudges
        // the run watch (review round 2, L4).
        if let (Some(_), Some(control)) = (generation, ctx.turn_control.swarm_control.as_ref()) {
            control.nudge_watch();
        }
        let event = match (
            generation,
            ctx.turn_control.swarm_control.as_ref(),
            ctx.cmd_tx.try_reserve(),
        ) {
            (Some(generation), Some(_), Ok(permit)) => {
                if ctx.turn_control.queue_swarm_wake(generation) {
                    permit.send(super::uds_multi::ClientMessage::SwarmWake { generation });
                }
                super::protocol::AgentEvent::ok(
                    id.as_deref(),
                    "swarm_control",
                    Some(serde_json::json!({"status":"accepted"})),
                )
            }
            (Some(generation), Some(_), Err(tokio::sync::mpsc::error::TrySendError::Full(_))) => {
                // A wake message is already pending only when the coalescing
                // slot was occupied: then this generation rides that message.
                // If the slot was empty nothing would drain it, so refuse.
                if ctx.turn_control.queue_swarm_wake(generation) {
                    ctx.turn_control.take_swarm_wake(0);
                    super::protocol::AgentEvent::err(
                        id.as_deref(),
                        "swarm_control",
                        "wake queue full; the durable inbox remains authoritative",
                    )
                } else {
                    super::protocol::AgentEvent::ok(
                        id.as_deref(),
                        "swarm_control",
                        Some(serde_json::json!({"status":"coalesced"})),
                    )
                }
            }
            _ => {
                super::protocol::AgentEvent::err(id.as_deref(), "swarm_control", "wake unavailable")
            }
        };
        if let Some(writer) = super::uds_ext_protocol::client_writer_tx(ctx.registry, ctx.client_id)
        {
            let _ = writer.send(event.to_json_line() + "\n").await;
        }
        return true;
    }
    let action = match action {
        Action::UsageBudget => {
            let value: serde_json::Value =
                serde_json::from_str(&ctx.line).expect("validated command");
            if value.get("token_limit").is_none() {
                let event = super::protocol::AgentEvent::err(
                    id.as_deref(),
                    "swarm_control",
                    "token_limit is required; null explicitly disables the budget",
                );
                super::uds_busy_subagents::write_event(
                    ctx.registry,
                    ctx.client_id,
                    id.as_deref(),
                    "swarm_control",
                    &event,
                )
                .await;
                return true;
            }
            RunControlAction::UsageBudget {
                token_limit,
                strict_unknown: strict_unknown.unwrap_or(true),
            }
        }
        Action::Pause => RunControlAction::Pause {
            reason: reason.unwrap_or_else(|| "supervisor requested pause".into()),
        },
        Action::Resume => RunControlAction::Resume,
        Action::Close => RunControlAction::Close,
        Action::Extend => match deadline_seconds {
            Some(seconds) if seconds > 0 => RunControlAction::ExtendDeadline { seconds },
            _ => {
                let event = super::protocol::AgentEvent::err(
                    id.as_deref(),
                    "swarm_control",
                    "deadline_seconds is required and must be positive",
                );
                super::uds_busy_subagents::write_event(
                    ctx.registry,
                    ctx.client_id,
                    id.as_deref(),
                    "swarm_control",
                    &event,
                )
                .await;
                return true;
            }
        },
        Action::Status => RunControlAction::Status,
        Action::Wake | Action::Watch => {
            unreachable!("wake and watch are handled before supervisor control")
        }
    };
    let resuming = matches!(action, RunControlAction::Resume);
    let result = match &ctx.turn_control.swarm_control {
        Some(control) => control.apply(action).await,
        None => Err(crate::domain::error::DomainError::Tool(
            "this agent has no swarm control port".into(),
        )),
    };
    let event = match result {
        Ok(receipt) => {
            ctx.turn_control
                .observe_control_generation(receipt.generation);
            let mut data = serde_json::json!({
                "status": status_name(receipt.status), "generation": receipt.generation,
                "outcome": receipt.outcome.map(status_name), "reason": receipt.reason,
                "applied": true, "budget": receipt.budget
            });
            // #1721: the store's fan-out excludes the resumer, so wake this
            // process itself: a member suspended by a provider failure
            // re-arms on that wake.
            if resuming && let Some(warning) = self_wake(ctx, receipt.generation) {
                data["warning"] = serde_json::Value::from(warning);
            }
            if !receipt.wake_warnings.is_empty() {
                data["wake_warnings"] = serde_json::Value::from(receipt.wake_warnings);
            }
            if !receipt.resume_blockers.is_empty() {
                data["resume_blockers"] = serde_json::Value::from(receipt.resume_blockers);
            }
            super::protocol::AgentEvent::ok(id.as_deref(), "swarm_control", Some(data))
        }
        Err(error) => {
            super::protocol::AgentEvent::err(id.as_deref(), "swarm_control", error.to_string())
        }
    };
    if let Some(writer) = super::uds_ext_protocol::client_writer_tx(ctx.registry, ctx.client_id) {
        let _ = writer.send(event.to_json_line() + "\n").await;
    }
    true
}

/// Queue a wake for this process. A full command channel only carries the
/// generation when a wake message is already pending; otherwise the slot is
/// released again so later wakes are not swallowed, and the caller is warned.
fn self_wake(ctx: &ReaderDispatchCtx<'_>, generation: u64) -> Option<String> {
    match ctx.cmd_tx.try_reserve() {
        Ok(permit) => {
            if ctx.turn_control.queue_swarm_wake(generation) {
                permit.send(super::uds_multi::ClientMessage::SwarmWake { generation });
            }
            None
        }
        Err(tokio::sync::mpsc::error::TrySendError::Full(())) => {
            if ctx.turn_control.queue_swarm_wake(generation) {
                ctx.turn_control.take_swarm_wake(0);
                Some("self-wake queue full; suspended members re-arm on their next wake".into())
            } else {
                None
            }
        }
        Err(tokio::sync::mpsc::error::TrySendError::Closed(())) => {
            Some("this agent is shutting down; no self-wake".into())
        }
    }
}

fn status_name(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Setup => "setup",
        RunStatus::Running => "running",
        RunStatus::Paused => "paused",
        RunStatus::Succeeded => "succeeded",
        RunStatus::Blocked => "blocked",
        RunStatus::Failed => "failed",
        RunStatus::Cancelled => "cancelled",
        RunStatus::BudgetExhausted => "budget-exhausted",
    }
}

pub(super) async fn handle_wake(ctx: &mut super::uds::DispatchCtx<'_>, generation: u64) -> bool {
    use super::protocol::AgentEvent;
    use super::uds::emit_event_to_broadcast_or_writer;
    use crate::domain::swarm::{RunControlAction, RunStatus};

    let generation = ctx.turn_control.take_swarm_wake(generation);
    let Some(control) = ctx.turn_control.swarm_control.clone() else {
        return false;
    };
    // #1721: while suspended, learn the control generation without claiming
    // wake events; a pause/resume since the provider failure re-arms us.
    if !ctx.session.automatic_turns_allowed {
        match control.apply(RunControlAction::Status).await {
            Ok(receipt) => {
                ctx.turn_control
                    .observe_control_generation(receipt.generation);
                if ctx.session.resume_after_control_change(receipt.generation) {
                    let event = AgentEvent::ok(
                        None,
                        SWARM_WAKE,
                        Some(serde_json::json!({
                            "status": "resumed", "generation": receipt.generation,
                            "reason": "automatic turns re-armed by a swarm resume after a provider failure"
                        })),
                    );
                    emit_event_to_broadcast_or_writer(ctx, &event).await;
                }
            }
            Err(error) => {
                // Contention keeps the generation for the next wake; either
                // way the suspended member says why it stayed suspended.
                let deferred = store_failure(&error) == StoreFailure::Transient;
                if deferred {
                    ctx.turn_control.defer_swarm_wake(generation);
                }
                let event = AgentEvent::err(
                    None,
                    SWARM_WAKE,
                    format!(
                        "generation {generation} {} while suspended: {error}",
                        if deferred { "deferred" } else { "not applied" }
                    ),
                );
                emit_event_to_broadcast_or_writer(ctx, &event).await;
                return false;
            }
        }
        if !ctx.session.automatic_turns_allowed {
            return false;
        }
    }
    match control.apply(RunControlAction::Wake { generation }).await {
        Ok(receipt) => {
            ctx.turn_control
                .observe_control_generation(receipt.generation);
            ctx.session
                .observe_control_generation(Some(receipt.generation));
            // A re-armed member owes the run a turn to continue its
            // interrupted work, whether or not this wake targets it.
            let runnable = matches!(receipt.status, RunStatus::Setup | RunStatus::Running);
            let resume_turn = runnable && ctx.session.take_pending_resume_turn();
            if resume_turn {
                super::uds::handle_prompt(ctx, super::uds::PromptCommand {
                    id: None, type_name: SWARM_WAKE.into(),
                    message: "The swarm was resumed after a provider failure interrupted your turn. Continue your interrupted work: inspect summary using your last event_cursor, read and acknowledge relevant durable inbox messages, work only on actionable tasks and yield when none remain.".into(),
                    streaming_behavior: None,
                }).await;
            } else if receipt.wake_allowed {
                super::uds::handle_prompt(ctx, super::uds::PromptCommand {
                    id: None, type_name: SWARM_WAKE.into(),
                    message: "Swarm work changed. Inspect summary using your last event_cursor, then read and acknowledge relevant durable inbox messages. Work only on actionable tasks; yield when none remain.".into(),
                    streaming_behavior: None,
                }).await;
            }
        }
        Err(error) => match store_failure(&error) {
            // Contention: keep the generation for the next wake.
            StoreFailure::Transient => {
                ctx.turn_control.defer_swarm_wake(generation);
                let event = AgentEvent::err(
                    None,
                    SWARM_WAKE,
                    format!("generation {generation} deferred: {error}"),
                );
                emit_event_to_broadcast_or_writer(ctx, &event).await;
            }
            // A durable rejection stops automatic turns until a human prompt.
            StoreFailure::Durable => {
                ctx.session.suspend_automatic_turns(
                    super::uds_session::SuspensionCause::StoreRejection,
                    None,
                );
                let event = AgentEvent::err(
                    None,
                    SWARM_WAKE,
                    format!("generation {generation}: {error}"),
                );
                emit_event_to_broadcast_or_writer(ctx, &event).await;
            }
        },
    }
    false
}

/// #1721: learn the control generation early (off the accept path) so a
/// provider-failure suspension has a baseline even if the store is busy right
/// after the failure.
pub(super) fn seed_control_generation(turn_control: &super::uds_cancel::TurnControlHandle) {
    let Some(control) = turn_control.swarm_control.clone() else {
        return;
    };
    let seed = std::sync::Arc::clone(turn_control);
    tokio::spawn(async move {
        if let Ok(receipt) = control
            .apply(crate::domain::swarm::RunControlAction::Status)
            .await
        {
            seed.observe_control_generation(receipt.generation);
        }
    });
}

/// #1721: a provider-failure suspension is dated by the control generation
/// current *after* the failure, so a pause/resume that happened during the
/// failed turn cannot re-arm it; only a later resume can. Without a swarm,
/// or when the store cannot answer, the pre-turn generation stands. Runs
/// after every turn path (prompt, drained, nudged) and only touches a
/// suspension not yet dated this way, so a later idle drain cannot re-date
/// an older suspension past a resume it should honour.
pub(super) async fn date_provider_suspension(ctx: &mut super::uds::DispatchCtx<'_>) {
    use crate::domain::swarm::RunControlAction;
    if !ctx.session.needs_provider_dating() {
        return;
    }
    let Some(control) = ctx.turn_control.swarm_control.clone() else {
        return;
    };
    if let Ok(receipt) = control.apply(RunControlAction::Status).await {
        ctx.turn_control
            .observe_control_generation(receipt.generation);
        ctx.session.suspend_automatic_turns(
            super::uds_session::SuspensionCause::ProviderFailure,
            Some(receipt.generation),
        );
    }
}

/// This process's turn control: its swarm's run control when it runs under
/// a container contract, launched as its coordinator or not (#2467).
pub(super) fn process_turn_control(coordinator: bool) -> super::uds_cancel::TurnControlHandle {
    let swarm_control = crate::interface::tool_runtime::swarm_context().map(|context| {
        std::sync::Arc::new(context)
            as std::sync::Arc<dyn crate::application::swarm::ports::SwarmRunControl>
    });
    let turn_control = std::sync::Arc::new(
        super::uds_cancel::TurnControl::with_swarm_control(swarm_control)
            .launched_as_coordinator(coordinator),
    );
    seed_control_generation(&turn_control);
    turn_control
}

/// The idle boundary every drain ends at (#2467): the provider-suspension
/// dating it always ran, then, for the run's coordinator, the `swarm_state`
/// its parent wakes on.
pub(super) async fn at_idle_boundary(ctx: &mut super::uds::DispatchCtx<'_>) {
    date_provider_suspension(ctx).await;
    let event = match ctx.turn_control.launched_coordinator {
        true => coordinator_swarm_state(&ctx.turn_control).await,
        false => member_reply_ready(&ctx.turn_control),
    };
    if let Some(event) = event {
        super::uds::emit_event_to_broadcast_or_writer(ctx, &event).await;
    }
}

/// A swarm member's `reply_ready` (#2471): sent once at the idle boundary
/// after a client's instruction ran, so its coordinator, which no longer
/// wakes on every turn end, hears the reply.
fn member_reply_ready(
    turn_control: &super::uds_cancel::TurnControlHandle,
) -> Option<super::protocol::AgentEvent> {
    let prompted = std::sync::atomic::Ordering::SeqCst;
    match (
        turn_control.swarm_control.is_some(),
        turn_control.client_prompted.swap(false, prompted),
    ) {
        (true, true) => Some(super::protocol::AgentEvent::ReplyReady),
        (true, false) | (false, _) => None,
    }
}

/// The run's `swarm_state` as this member reports it (#2467): only a
/// process launched as its run's coordinator reads its board; an unreadable
/// board is reported as unknown once it has reported before, so its parent
/// is never left waiting for a state that will not come.
async fn coordinator_swarm_state(
    turn_control: &super::uds_cancel::TurnControlHandle,
) -> Option<super::protocol::AgentEvent> {
    use crate::domain::swarm::parent_wake::parent_wake;
    use std::sync::atomic::Ordering;
    let control = match turn_control.launched_coordinator {
        true => turn_control.swarm_control.clone()?,
        false => return None,
    };
    let reported = &turn_control.coordinator_reported;
    let board = control.coordinator_board().await;
    if let Err(error) = &board {
        tracing::warn!("swarm_state: coordinator board unreadable: {error}");
    }
    let wake = match board {
        Ok(Some(board)) => {
            reported.store(true, Ordering::SeqCst);
            parent_wake(Some(&board))
        }
        // After a report, every later boundary reports, so a held turn-end
        // note always has a state to settle on.
        Ok(None) | Err(_) if reported.load(Ordering::SeqCst) => parent_wake(None),
        Ok(None) | Err(_) => return None,
    };
    let prompted = turn_control.client_prompted.swap(false, Ordering::SeqCst);
    Some(super::protocol::AgentEvent::SwarmState {
        wake: wake.kind(),
        status: wake.status().map(|status| status_name(status).to_owned()),
        prompted,
    })
}

/// How a coordination-store failure is treated by automatic turns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StoreFailure {
    /// SQLite busy/locked (`database ... is locked`): the next wake retries.
    Transient,
    /// Anything else is a durable rejection.
    Durable,
}

pub(super) fn store_failure(error: &crate::domain::error::DomainError) -> StoreFailure {
    if error.to_string().to_ascii_lowercase().contains("is locked") {
        StoreFailure::Transient
    } else {
        StoreFailure::Durable
    }
}
