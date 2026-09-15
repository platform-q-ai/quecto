//! Adapter of the sessions capability's `TurnAccountingReset` port (D6
//! #1975) over the dispatch loop's own per-session bookkeeping: the
//! [`AgentSession`] tracker (usage counters, context-size reading, the
//! pending steer/follow-up queue) and the execution view's message count
//! the busy `get_state` projection reports. The clear and rewind
//! transactions order the reset; this adapter performs it and decides
//! nothing.
use super::uds_execution_state::ExecutionStateHandle;
use super::uds_session::AgentSession;
use crate::application::sessions::ports::session_runtime::TurnAccountingReset;

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
        // Queued prompts were admitted against the replaced history; a
        // queued control is cancelled with its receipt recorded.
        self.session.discard_pending();
    }
}
