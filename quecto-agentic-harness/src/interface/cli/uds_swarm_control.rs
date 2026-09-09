//! Typed supervisor control independent of model execution and queued prompts.
use super::uds_reader_dispatch::ReaderDispatchCtx;
use crate::domain::swarm::{RunControlAction, RunStatus};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    Pause,
    Resume,
    Status,
    Wake,
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
    if matches!(action, Action::Wake) {
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
        Action::Status => RunControlAction::Status,
        Action::Wake => unreachable!("wake handled before supervisor control"),
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
            // #1721: the resumer gets no wake from the store's notification
            // fan-out (it excludes the sender), so wake this process itself:
            // a member suspended by a provider failure re-arms on that wake.
            if resuming
                && ctx.turn_control.queue_swarm_wake(receipt.generation)
                && let Ok(permit) = ctx.cmd_tx.try_reserve()
            {
                permit.send(super::uds_multi::ClientMessage::SwarmWake {
                    generation: receipt.generation,
                });
            }
            super::protocol::AgentEvent::ok(
                id.as_deref(),
                "swarm_control",
                Some(serde_json::json!({
                    "status": status_name(receipt.status), "generation": receipt.generation,
                    "applied": true, "budget": receipt.budget
                })),
            )
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
        if let Ok(receipt) = control.apply(RunControlAction::Status).await {
            ctx.turn_control
                .observe_control_generation(receipt.generation);
            if ctx.session.resume_after_control_change(receipt.generation) {
                let event = AgentEvent::ok(
                    None,
                    "swarm_wake",
                    Some(serde_json::json!({
                        "status": "resumed", "generation": receipt.generation,
                        "reason": "automatic turns re-armed by a swarm resume after a provider failure"
                    })),
                );
                emit_event_to_broadcast_or_writer(ctx, &event).await;
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
                    id: None, type_name: "swarm_wake".into(),
                    message: "The swarm was resumed after a provider failure interrupted your turn. Continue your interrupted work: inspect summary using your last event_cursor, read and acknowledge relevant durable inbox messages, work only on actionable tasks and yield when none remain.".into(),
                    streaming_behavior: None,
                }).await;
            } else if receipt.wake_allowed {
                super::uds::handle_prompt(ctx, super::uds::PromptCommand {
                    id: None, type_name: "swarm_wake".into(),
                    message: "Swarm work changed. Inspect summary using your last event_cursor, then read and acknowledge relevant durable inbox messages. Work only on actionable tasks; yield when none remain.".into(),
                    streaming_behavior: None,
                }).await;
            }
        }
        Err(error) => match store_failure(&error) {
            // Contention: keep the generation for the next wake.
            StoreFailure::Transient => {
                ctx.turn_control.queue_swarm_wake(generation);
                let event = AgentEvent::err(
                    None,
                    "swarm_wake",
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
                    "swarm_wake",
                    format!("generation {generation}: {error}"),
                );
                emit_event_to_broadcast_or_writer(ctx, &event).await;
            }
        },
    }
    false
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
