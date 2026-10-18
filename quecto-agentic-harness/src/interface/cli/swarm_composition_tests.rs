//! #2342/#2414: a member's ceiling is lowered to the swarm's once it takes
//! part in a swarm; the watermark marks scale down under it.

use super::wire_member;
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::application::providers::ports::{ChatRequest, LlmProvider};
use crate::domain::conversation::value_objects::message::LlmResponse;
use crate::domain::error::DomainError;
use crate::infrastructure::config::AgentDefaults;
use crate::infrastructure::tools::swarm_bridge::Participation;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

#[derive(Debug)]
struct Silent;

impl LlmProvider for Silent {
    fn route_order(&self) -> Vec<String> {
        vec![self.name().to_string()]
    }
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
        max_context_tokens: 190_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_marks: Default::default(),
        model_context_window: None,
        tool_profile_context:
            crate::domain::tool_policy::value_objects::tool::ToolProfileContext::Parent,
    })
}

#[test]
fn a_member_takes_the_swarm_ceiling_and_a_loner_keeps_its_budget() {
    let defaults = AgentDefaults {
        swarm_max_context_tokens: 40_000,
        ..AgentDefaults::default()
    };
    let participation = Participation::shared();
    let member = wire_member(agent(), &participation, &defaults);
    assert_eq!(member.effective_max_context_tokens(), 190_000, "not yet");
    participation.set(true);
    assert_eq!(member.effective_max_context_tokens(), 40_000);
    let loner = wire_member(agent(), &Participation::none(), &defaults);
    assert_eq!(loner.effective_max_context_tokens(), 190_000);
}
