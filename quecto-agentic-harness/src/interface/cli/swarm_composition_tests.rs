//! #2403: an agent, a swarm member alike, runs in the context mode its
//! configuration selects; a member loads the parent's configuration (the
//! same layers and `QUECTO_*` overrides), so it inherits the parent's mode
//! unless its own configuration sets one.

use super::wire_member;
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::application::providers::ports::{ChatRequest, LlmProvider};
use crate::domain::conversation::ContextMode;
use crate::domain::error::DomainError;
use crate::domain::message::LlmResponse;
use crate::infrastructure::config::AgentDefaults;
use crate::infrastructure::tools::swarm_bridge::Participation;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

#[derive(Debug)]
struct Silent;

impl LlmProvider for Silent {
    fn name(&self) -> &str {
        "silent"
    }

    fn chat(
        &self,
        _request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        Box::pin(async { Err(DomainError::Provider("silent".into())) })
    }
}

fn agent() -> AgentLoopImpl {
    AgentLoopImpl::new(AgentLoopConfig {
        provider: Arc::new(Silent),
        tool_registry: Box::new(crate::infrastructure::tools::registry::ToolRegistryImpl::new()),
        model: "stub".into(),
        max_tokens: 100,
        temperature: 0.0,
        retention: None,
        session_key: "cli:test".into(),
        context_collapse_after_tool_calls: u32::MAX,
        max_context_tokens: 190_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        large_result_collapse: crate::domain::large_result_collapse::LargeResultCollapse::DISABLED,
        model_context_window: None,
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
    })
}

#[test]
fn an_agent_and_a_member_run_in_the_configured_context_mode() {
    let mut watermark = AgentDefaults::default();
    watermark.context_mode.context_mode = Some("watermark".to_string());
    assert!(matches!(
        watermark.context_mode(),
        ContextMode::Watermark(_)
    ));
    for participation in [Participation::none(), Participation::Fixed(true)] {
        let wired = wire_member(agent(), &participation, &watermark);
        assert_eq!(wired.context_mode(), watermark.context_mode());
        let wired = wire_member(agent(), &participation, &AgentDefaults::default());
        assert_eq!(wired.context_mode(), ContextMode::Default);
    }
}
