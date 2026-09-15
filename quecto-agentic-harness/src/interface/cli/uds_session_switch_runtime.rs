//! Adapter of the sessions capability's session-switch runtime ports (D7
//! #1976) over the dispatch loop's own runtime: the turn accounting the
//! clear/rewind adapter already performs, the raw-key holders the new
//! identity is propagated to (the [`AgentSession`] tracker, the agent loop
//! and its session-aware tools), and the session-scoped settings a switch
//! resets or restores (D8 #1977) — the effort override (#1067) and the
//! workflow engine's run — each bumping the tracker's visible generation
//! only when something changed. The transactions order these; this adapter
//! performs them and decides nothing.
use super::uds_execution_state::ExecutionStateHandle;
use super::uds_session::AgentSession;
use super::uds_turn_accounting::LoopTurnAccounting;
use crate::application::agent_loop::AgentLoopImpl;
use crate::application::sessions::ports::session_runtime::TurnAccountingReset;
use crate::application::sessions::ports::{SessionKeyPropagation, SessionSwitchRuntime};
use crate::domain::session_identity::SessionIdentity;
use crate::interface::shared::WorkflowStateHandle;

pub struct LoopSessionSwitchRuntime<'a> {
    agent: &'a mut AgentLoopImpl,
    session: &'a mut AgentSession,
    execution_state: &'a ExecutionStateHandle,
    workflow_state: Option<&'a WorkflowStateHandle>,
}

impl<'a> LoopSessionSwitchRuntime<'a> {
    pub fn new(
        agent: &'a mut AgentLoopImpl,
        session: &'a mut AgentSession,
        execution_state: &'a ExecutionStateHandle,
        workflow_state: Option<&'a WorkflowStateHandle>,
    ) -> Self {
        Self {
            agent,
            session,
            execution_state,
            workflow_state,
        }
    }
}

impl TurnAccountingReset for LoopSessionSwitchRuntime<'_> {
    fn history_replaced(&mut self, visible_message_count: usize) {
        LoopTurnAccounting::new(self.session, self.execution_state)
            .history_replaced(visible_message_count);
    }
}

impl SessionKeyPropagation for LoopSessionSwitchRuntime<'_> {
    fn session_key_changed(&mut self, identity: &SessionIdentity) {
        self.session
            .set_session_key(identity.runtime_key().to_string());
        self.agent.set_session_key(identity.clone());
    }
}

impl SessionSwitchRuntime for LoopSessionSwitchRuntime<'_> {
    fn reset_effort_to_default(&mut self) {
        let before = self.agent.effort();
        self.agent.reset_effort_to_default();
        if self.agent.effort() != before {
            self.session.bump_visible_generation();
        }
    }

    fn reset_workflow(&mut self) {
        apply_workflow_run(self.workflow_state, self.session, None);
    }

    fn restore_workflow(&mut self, run: crate::domain::workflow::WorkflowRunPersisted) {
        apply_workflow_run(self.workflow_state, self.session, Some(run));
    }
}

/// Replace the bound workflow engine's run — restore `run`, or reset the
/// engine when there is none — and bump the tracker's visible generation
/// when the engine's snapshot changed.
fn apply_workflow_run(
    workflow_state: Option<&WorkflowStateHandle>,
    session: &mut AgentSession,
    run: Option<crate::domain::workflow::WorkflowRunPersisted>,
) {
    if let Some(workflow) = workflow_state
        && let Ok(mut engine) = workflow.lock()
    {
        let before = serde_json::to_value(engine.snapshot(true)).ok();
        if let Some(run) = run {
            engine.restore_run(run);
        } else {
            engine.reset();
        }
        let after = serde_json::to_value(engine.snapshot(true)).ok();
        if before != after {
            session.bump_visible_generation();
        }
    }
}
