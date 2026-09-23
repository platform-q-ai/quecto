//! Test-support entry points over the UDS session internals (execution
//! state, admission projection, mid-turn publish and the reader dispatch),
//! used by BDD scenarios and integration tests. Built only under `cfg(test)`
//! or the `test-support` feature.

use super::{
    protocol, uds_admission_projection, uds_cancel, uds_execution_state, uds_ext_protocol,
    uds_reader_dispatch, uds_session_handles, uds_state_projection,
};

pub fn live_execution_state_for_events(
    events: &[crate::domain::agent::AgentProgressEvent],
) -> serde_json::Value {
    let mut state = uds_execution_state::ExecutionState::default();
    state.start_run();
    for event in events {
        state.observe(event);
    }
    serde_json::json!({ "messageCount": state.message_count(), "execution": state.snapshot() })
}

/// Test-support (#1679 P4): the `get_state` projection of a process whose
/// admission activity comes from `source`, polled the way a supervisor does.
pub struct AdmissionStateProbe {
    execution: uds_execution_state::ExecutionState,
    session: protocol::SessionState,
}

impl AdmissionStateProbe {
    pub fn new(
        source: std::sync::Arc<dyn crate::application::ports::AdmissionObservation>,
    ) -> Self {
        let mut execution = uds_execution_state::ExecutionState::default();
        execution.set_admission_source(source);
        Self {
            execution,
            session: protocol::SessionState {
                admission_warnings: Vec::new(),
                model: "probe".into(),
                generation: 0,
                is_streaming: false,
                session_key: "probe".into(),
                message_count: 0,
                pending_message_count: 0,
                max_context_tokens: 0,
                effort: None,
                effort_levels: vec![],
                workflow: None,
                execution: None,
                sync: 0,
                control_receipts: vec![],
                automatic_turns_suspended: false,
                repeated_failure_notifications: Default::default(),
            },
        }
    }
    pub fn start_run(&mut self) {
        self.execution.start_run();
    }
    pub fn finish_run(&mut self) {
        self.execution.finish_run();
    }
    /// The slim `get_state` response data for an optional `since` cursor.
    pub fn poll(&mut self, since: Option<u64>) -> serde_json::Value {
        self.session.generation = self.execution.observe_visible_revisions(0, 0);
        self.session.execution = Some(self.execution.snapshot());
        uds_state_projection::slim_state_response_data(&self.session, since)
    }
}

/// Test-support (#1679 P4): the production `admission_state_changed` hook.
pub fn admission_broadcast_hook(
    broadcast_tx: tokio::sync::broadcast::Sender<String>,
) -> crate::infrastructure::admission::ActivityHook {
    uds_admission_projection::admission_event_hook(broadcast_tx, None)
}

pub fn completed_live_execution_state(
    events: &[crate::domain::agent::AgentProgressEvent],
) -> serde_json::Value {
    let mut state = uds_execution_state::ExecutionState::default();
    state.start_run();
    for event in events {
        state.observe(event);
    }
    state.finish_run();
    serde_json::json!({ "messageCount": state.message_count(), "execution": state.snapshot() })
}
/// Test-support: run a sequence of progress events through the mid-turn
/// publish path (`publish_turn_progress`) against a fresh conversation
/// snapshot, returning every event line emitted to the sink. BDD scenarios use
/// this to pin that mid-turn `TurnCompleted` events emit `ledger_advanced`
/// hints (the child-progress-freeze fix, 2026-07-29).
pub async fn ledger_hint_lines_for_turn_events(
    events: &[crate::domain::agent::AgentProgressEvent],
    session: &crate::application::sessions::active_session::ActiveSessionHandle,
) -> Vec<serde_json::Value> {
    let mut buf: Vec<u8> = Vec::new();
    let mut sink = uds_cancel::EventSink::writer(&mut buf);
    for event in events {
        uds_cancel::publish_turn_progress(event, Some(session), &mut sink).await;
    }
    String::from_utf8_lossy(&buf)
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// Test-support: run one raw command line through the FULL per-connection
/// reader dispatch (`uds_reader_dispatch::dispatch`) against a snapshot with
/// one committed message. Returns `(served_inline, response)`: `served_inline`
/// is true when the command was answered on the reader task and never queued
/// behind the dispatch loop. Covers the TUI's DIRECT child-feed path — a
/// plain `sync` with no `agent_id` on the child's own socket — which is
/// served by the child-local `uds_sync` fast path even while the child's
/// dispatch loop is occupied (PR #1307 review).
pub async fn busy_reader_dispatch(
    line: &str,
    session: &uds_session_handles::SessionReadHandles,
) -> (bool, Option<serde_json::Value>) {
    let _ = session
        .active_session
        .write()
        .await
        .publish(&[crate::domain::message::Message::user("committed")]);
    let clients = uds_ext_protocol::new_client_tool_registry();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(8);
    uds_ext_protocol::register_client_writer(&clients, 1, tx);
    // The dispatch-loop channel: anything landing here would have queued
    // behind an in-flight parent/child turn.
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel(8);
    uds_reader_dispatch::dispatch(uds_reader_dispatch::ReaderDispatchCtx {
        line: line.to_string(),
        session,
        registry: &clients,
        subagent_registry: &None,
        fleet: None,
        client_id: 1,
        cmd_tx: &cmd_tx,
        cancel_handle: &std::sync::Arc::new(std::sync::Mutex::new(uds_cancel::CancelSlot::Idle)),
        turn_control: &uds_cancel::TurnControl::default(),
    })
    .await;
    let served_inline = cmd_rx.try_recv().is_err();
    if !served_inline {
        return (false, None);
    }
    let response = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .ok()
        .flatten()
        .and_then(|l| serde_json::from_str(&l).ok());
    (true, response)
}
