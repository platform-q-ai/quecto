//! `AgentLoop` port implementation for [`AgentLoopImpl`].
//!
//! The domain-facing port is a thin adapter over the loop's own methods; it
//! lives beside the implementation rather than inside it so `agent_loop.rs`
//! stays within the module size gate.

use std::pin::Pin;

use crate::application::agent_loop::AgentLoopImpl;
use crate::domain::agent::{AgentInfo, AgentLoop, AgentResult};
use crate::domain::error::DomainError;
use crate::domain::message::Message;

impl AgentLoop for AgentLoopImpl {
    fn process<'a>(
        &'a mut self,
        messages: &'a mut Vec<Message>,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<AgentResult, DomainError>> + Send + 'a>>
    {
        Box::pin(self.run_loop(messages))
    }

    fn info(&self) -> AgentInfo {
        AgentInfo {
            tool_count: self.tool_catalog().tool_count(),
        }
    }
}
