//! Adapter of the sessions capability's `TurnAccountingReset` port (D6
//! #1975) over the dispatch loop's own per-session bookkeeping: the
//! [`AgentSession`] tracker (usage counters, context-size reading, the
//! pending steer/follow-up queue) and the execution view's message count
//! the busy `get_state` projection reports. The clear and rewind
//! transactions order the reset; this adapter performs it and decides
//! nothing.
use crate::application::sessions::ports::session_runtime::TurnAccountingReset;
use crate::interface::cli::uds_session::AgentSession;
use crate::interface::uds::presenters::uds_execution_state::ExecutionStateHandle;

pub struct LoopTurnAccounting<'a> {
    session: &'a mut AgentSession,
    execution_state: &'a ExecutionStateHandle,
}

impl<'a> LoopTurnAccounting<'a> {
    pub fn new(session: &'a mut AgentSession, execution_state: &'a ExecutionStateHandle) -> Self {
        Self {
            session,
            execution_state,
        }
    }
}

impl TurnAccountingReset for LoopTurnAccounting<'_> {
    fn history_replaced(&mut self, visible_message_count: usize) {
        if let Ok(mut state) = self.execution_state.lock() {
            state.set_message_count(visible_message_count);
        }
        self.session.clear_usage();
        self.session.reset_workflow_emissions();
        // A replaced history is a new turn-outcome incarnation (#2708).
        if let Some(runtime) = self.session.parent_outcome() {
            runtime.history_replaced();
        }
        // Discard old-history prompts, recording cancellation for queued controls.
        self.session.discard_pending();
    }
}
