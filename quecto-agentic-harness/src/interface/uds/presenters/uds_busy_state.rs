//! A guarded harness's own `get_state` while a turn is busy (#2708 trial).
//!
//! The dispatch loop is serial, so a `get_state` queued behind a long turn
//! waited for the turn to end, and the Pocket relay reads state on every
//! `ledger_advanced` hint under a 15 s timeout. On a guarded harness the
//! connection's reader answers it mid-turn instead, from the state snapshot
//! the loop refreshes at every turn boundary and after each `turn_outcome`,
//! overlaid with the live execution, workflow and admission warnings exactly
//! as the connect-time snapshot is; `since` is honoured as on the loop.
//! It is also answered here while commands are queued ahead of it (a
//! follow-up the loop is about to run as the next turn), so it never waits
//! behind a turn; the answer is then as of the last command the loop
//! finished, as mid-turn. Idle with nothing queued, the command goes to the
//! loop as before. An unguarded harness (no source) is unchanged: its
//! `get_state` waits for the loop.
use crate::interface::uds::wire::protocol::{AgentCommand, AgentEvent};

/// What a busy `get_state` is answered from: one per guarded loop, shared
/// by its connections.
#[derive(Clone)]
pub(crate) struct BusyState {
    pub(crate) busy: crate::interface::uds::framing::uds_multi::BusyFlag,
    pub(crate) state: crate::interface::cli::uds_snapshots::StateSnapshot,
    pub(crate) runtime_store: crate::application::provider_runtime::RuntimeSnapshotStore,
    pub(crate) workflow: Option<crate::interface::cli::controllers::shared::WorkflowStateHandle>,
    pub(crate) execution: super::uds_execution_state::ExecutionStateHandle,
}

/// Answer `line` here when it is this harness's own `get_state` (no
/// `agent_id`) on a guarded loop that is busy or has commands
/// `queued_ahead`; otherwise leave it to the loop.
pub(in crate::interface) async fn intercept(
    line: &str,
    (source, queued_ahead): (Option<&BusyState>, bool),
    clients: &crate::interface::uds::controllers::uds_ext_protocol::ClientToolRegistry,
    client_id: u64,
) -> bool {
    // Most lines are prompts: only a guarded loop's line naming the command
    // is parsed.
    let Some(source) = source.filter(|_| line.contains("get_state")) else {
        return false;
    };
    let Ok(AgentCommand::GetState {
        id,
        since,
        agent_id: None,
    }) = serde_json::from_str::<AgentCommand>(line.trim())
    else {
        return false;
    };
    let busy = source.busy.load(std::sync::atomic::Ordering::SeqCst);
    if !(busy || queued_ahead) {
        return false;
    }
    let data = source.answer(since).await;
    let event = AgentEvent::ok(id.as_deref(), "get_state", Some(data));
    crate::interface::uds::controllers::uds_busy_subagents::write_event(
        clients,
        client_id,
        id.as_deref(),
        "get_state",
        &event,
    )
    .await;
    true
}

impl BusyState {
    /// The `get_state` data a busy loop would give: the refreshed snapshot
    /// with the live overlays, the unchanged marker at a current `since`.
    async fn answer(&self, since: Option<u64>) -> serde_json::Value {
        // Release the async snapshot lock before taking either sync mutex.
        let mut snapshot = self.state.read().await.clone();
        super::uds_admission_warnings::overlay_current(&mut snapshot, &self.runtime_store);
        let mut live = crate::interface::cli::uds_snapshots::connect_state(
            &snapshot,
            &self.workflow,
            &self.execution,
        );
        live.is_streaming = true;
        crate::interface::cli::uds_state_projection::slim_state_response_data(&live, since)
    }
}

#[cfg(test)]
#[path = "uds_busy_state_tests.rs"]
mod tests;
