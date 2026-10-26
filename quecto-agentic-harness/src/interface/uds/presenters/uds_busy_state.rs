//! A guarded harness's own `get_state` while its serial loop is busy (#2708):
//! the connection answers it from the snapshot, to the requester only.
use crate::interface::uds::wire::protocol::{AgentCommand, AgentEvent};

/// What a busy `get_state` is answered from, shared by the connections.
#[derive(Clone)]
pub(crate) struct BusyState {
    pub(crate) busy: crate::interface::uds::framing::uds_multi::BusyFlag,
    pub(crate) state: crate::interface::cli::uds_snapshots::StateSnapshot,
    pub(crate) runtime_store: crate::application::provider_runtime::RuntimeSnapshotStore,
    pub(crate) workflow: Option<crate::interface::cli::controllers::shared::WorkflowStateHandle>,
    pub(crate) execution: super::uds_execution_state::ExecutionStateHandle,
}

/// A busy `get_state` the connection answers: its `id` and `since`.
pub(crate) type Asked = (Option<String>, Option<u64>);

/// `line` as this harness's own `get_state` (no `agent_id`) on a guarded
/// loop that is busy; `None` leaves it to the loop.
pub(in crate::interface) fn asked(line: &str, source: Option<&BusyState>) -> Option<Asked> {
    let source = source.filter(|_| line.contains("get_state"))?;
    let command = serde_json::from_str::<AgentCommand>(line.trim()).ok()?;
    let busy = source.busy.load(std::sync::atomic::Ordering::SeqCst);
    match (command, busy) {
        (
            AgentCommand::GetState {
                id,
                since,
                agent_id: None,
            },
            true,
        ) => Some((id, since)),
        _ => None,
    }
}

impl BusyState {
    /// The answer's frame, computed now; while busy, an idle phase (a run
    /// ended before its `agent_end`, or between turns) reads `finalizing`.
    pub(crate) async fn answer(&self, (id, since): Asked) -> String {
        // Release the async snapshot lock before taking either sync mutex.
        let mut snapshot = self.state.read().await.clone();
        super::uds_admission_warnings::overlay_current(&mut snapshot, &self.runtime_store);
        use crate::interface::cli::uds_snapshots::connect_state;
        let mut live = connect_state(&snapshot, &self.workflow, &self.execution);
        let busy = self.busy.load(std::sync::atomic::Ordering::SeqCst);
        if let (true, Some(execution)) = (busy, live.execution.as_mut()) {
            match execution.phase.as_str() {
                "thinking" | "runningTool" | "finalizing" => {}
                _ => execution.phase = "finalizing".into(),
            }
        }
        let data =
            crate::interface::cli::uds_state_projection::slim_state_response_data(&live, since);
        let mut event = AgentEvent::ok(id.as_deref(), "get_state", Some(data)).to_json_line();
        if event.len() > crate::infrastructure::processes::line_cap::EVENT_LINE_JSON_BUDGET {
            let error = "response exceeds the protocol frame limit";
            event = AgentEvent::err(id.as_deref(), "get_state", error).to_json_line();
        }
        event + "\n"
    }
}

#[cfg(test)]
#[path = "uds_busy_state_tests.rs"]
mod tests;
