use super::uds::DispatchCtx;

pub(super) fn response_data(ctx: &DispatchCtx<'_>) -> serde_json::Value {
    let removed = delete_all_subagents_from_registry(
        ctx.subagent_registry
            .as_ref()
            .expect("UDS dispatch always carries a subagent registry"),
        ctx.broadcast_tx.as_ref(),
    );
    serde_json::json!({ "removed": removed })
}

pub(super) fn delete_all_subagents_from_registry(
    registry: &crate::infrastructure::tools::subagent_registry::SubagentRegistry,
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
