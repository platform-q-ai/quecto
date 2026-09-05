use super::protocol::AgentEvent;
use super::uds::DispatchCtx;

type SubagentRegistry = crate::infrastructure::tools::subagent_registry::SubagentRegistry;

const COMMAND: &str = "delete_all_subagents";

/// Dispatch-path response payload. Multi-client connections are answered on
/// the reader task ([`busy_response`]) before anything reaches the dispatch
/// loop, so this path now serves the single-client loop and unit tests.
pub(super) fn response_data(ctx: &DispatchCtx<'_>) -> serde_json::Value {
    response_payload(
        ctx.subagent_registry
            .as_ref()
            .expect("UDS dispatch always carries a subagent registry"),
        ctx.broadcast_tx.as_ref(),
    )
}

/// Busy-path response (reader task, #1626): the same drain + broadcast as the
/// dispatch path, wrapped as a correlated event. A harness without a
/// registry has nothing to delete and answers with a correlated error.
pub(super) fn busy_response(
    registry: Option<&SubagentRegistry>,
    broadcast_tx: &tokio::sync::broadcast::Sender<String>,
    id: Option<&str>,
) -> AgentEvent {
    match registry {
        Some(registry) => AgentEvent::ok(
            id,
            COMMAND,
            Some(response_payload(registry, Some(broadcast_tx))),
        ),
        None => AgentEvent::err(id, COMMAND, "no sub-agent registry available"),
    }
}

/// The single wire shape both paths return.
fn response_payload(
    registry: &SubagentRegistry,
    broadcast_tx: Option<&tokio::sync::broadcast::Sender<String>>,
) -> serde_json::Value {
    serde_json::json!({ "removed": delete_all_subagents_from_registry(registry, broadcast_tx) })
}

pub(super) fn delete_all_subagents_from_registry(
    registry: &SubagentRegistry,
    broadcast_tx: Option<&tokio::sync::broadcast::Sender<String>>,
) -> usize {
    let removed = crate::infrastructure::tools::spawn::shutdown_all_with_count(registry);
    // Broadcast an authoritative empty survivor set after clearing the harness
    // registry so every connected client drops stale panel entries too.
    if let Some(tx) = broadcast_tx {
        let _ = tx.send(
            crate::infrastructure::tools::subagent_cascade::build_state_changed_event(registry),
        );
    }
    removed
}

#[cfg(test)]
#[path = "uds_delete_all_subagents_tests.rs"]
mod tests;
