//! Adapter of the session-switch runtime ports (D7 #1976, D8 #1977) over
//! the dispatch loop's runtime: turn accounting, key propagation to the
//! agent loop and its session-aware tools (the [`AgentSession`] tracker
//! keeps no copy, D10 #1979), and the effort/workflow reset or restore,
//! each bumping the tracker's visible generation only when something
//! changed. The transactions order these; this adapter decides nothing.
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
    /// The change-reasoning-effort use case (#1848) that restores the
    /// startup effort as admitted for the active model on a switch.
    effort: std::sync::Arc<crate::application::catalogue::use_cases::ChangeReasoningEffort>,
}

impl<'a> LoopSessionSwitchRuntime<'a> {
    pub fn new(
        agent: &'a mut AgentLoopImpl,
        session: &'a mut AgentSession,
        execution_state: &'a ExecutionStateHandle,
        workflow_state: Option<&'a WorkflowStateHandle>,
        effort: std::sync::Arc<crate::application::catalogue::use_cases::ChangeReasoningEffort>,
    ) -> Self {
        Self {
            agent,
            session,
            execution_state,
            workflow_state,
            effort,
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
    /// A changed key clears the departed session's usage and bumps the
    /// visible generation once, as the tracker's own key change did.
    fn session_key_changed(&mut self, identity: &SessionIdentity) {
        if self.agent.session_key() != identity.runtime_key() {
            self.session.session_changed();
        }
        self.agent.set_session_key(identity.clone());
    }
}

impl SessionSwitchRuntime for LoopSessionSwitchRuntime<'_> {
    fn reset_effort_to_default(&mut self) {
        if self
            .effort
            .restore_startup_default(self.agent, self.session.model())
        {
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
