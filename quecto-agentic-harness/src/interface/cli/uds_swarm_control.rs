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
            (Some(generation), Some(_), Err(_)) => {
                // The command channel is full: the queued wake message already
                // pending will drain this generation; coalesce, never drop it.
                ctx.turn_control.queue_swarm_wake(generation);
                super::protocol::AgentEvent::ok(
                    id.as_deref(),
                    "swarm_control",
                    Some(serde_json::json!({"status":"coalesced"})),
                )
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
    let result = match &ctx.turn_control.swarm_control {
        Some(control) => control.apply(action).await,
        None => Err(crate::domain::error::DomainError::Tool(
            "this agent has no swarm control port".into(),
        )),
    };
    let event = match result {
        Ok(receipt) => super::protocol::AgentEvent::ok(
            id.as_deref(),
            "swarm_control",
            Some(serde_json::json!({
                "status": status_name(receipt.status), "generation": receipt.generation,
                "applied": true, "budget": receipt.budget
            })),
        ),
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

    let generation = ctx.turn_control.take_swarm_wake(generation);
    if ctx.session.automatic_turns_allowed {
        if let Some(control) = ctx.turn_control.swarm_control.clone() {
            match control
                .apply(crate::domain::swarm::RunControlAction::Wake { generation })
                .await
            {
                Ok(receipt) => {
                    if receipt.wake_allowed {
                        super::uds::handle_prompt(ctx, super::uds::PromptCommand {
                                id: None, type_name: "swarm_wake".into(),
                                message: "Swarm work changed. Inspect summary using your last event_cursor, then read and acknowledge relevant durable inbox messages. Work only on actionable tasks; yield when none remain.".into(),
                                streaming_behavior: None,
                            }).await;
                    }
                }
                Err(error) => {
                    // Store contention is transient: the next wake retries.
                    // Only a durable rejection stops automatic turns.
                    if !transient_store_error(&error) {
                        ctx.session.automatic_turns_allowed = false;
                    }
                    let event = AgentEvent::err(
                        None,
                        "swarm_wake",
                        format!("generation {generation}: {error}"),
                    );
                    emit_event_to_broadcast_or_writer(ctx, &event).await;
                }
            }
        }
    }
    false
}

/// SQLite busy/locked failures from the coordination store are retried by the
/// next wake; they must not latch automatic turns off until a human prompt.
pub(super) fn transient_store_error(error: &crate::domain::error::DomainError) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    message.contains("database is locked") || message.contains("database table is locked")
}
