use super::tests::{MockProvider, MockRegistry, MockTool};
use super::*;
use crate::domain::agent::AgentLoop;
use std::sync::Arc;

#[test]
fn test_swap_registry_replaces_tool_registry() {
    let provider = Arc::new(MockProvider::new(vec![]));
    let mut reg_a = MockRegistry::new();
    reg_a.register(Arc::new(MockTool::new("tool_a", "ok")));
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: provider.clone(),
        tool_registry: Box::new(reg_a),
        model: "m".into(),
        max_tokens: 100,
        temperature: 0.0,
        spill_store: None,
        session_key: String::new(),
        context_collapse_after_tool_calls: u32::MAX,
        max_context_tokens: 100_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        system_prompt_provider: None,
        audit_log: None,
    });
    assert_eq!(agent.info().tool_count, 1);
    assert_eq!(agent.tool_registry.definitions()[0].name.as_ref(), "tool_a");

    let mut reg_b = MockRegistry::new();
    reg_b.register(Arc::new(MockTool::new("tool_b", "ok")));
    reg_b.register(Arc::new(MockTool::new("tool_c", "ok")));
    agent.swap_registry(Box::new(reg_b));
    assert_eq!(agent.info().tool_count, 2);
    let names: Vec<&str> = agent
        .tool_registry
        .definitions()
        .iter()
        .map(|d| d.name.as_ref())
        .collect();
    assert!(names.contains(&"tool_b"));
    assert!(names.contains(&"tool_c"));
    assert!(!names.contains(&"tool_a"));
}

#[test]
fn test_swap_registry_info_reflects_new_count() {
    let provider = Arc::new(MockProvider::new(vec![]));
    let reg = MockRegistry::new();
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(reg),
        model: "m".into(),
        max_tokens: 100,
        temperature: 0.0,
        spill_store: None,
        session_key: String::new(),
        context_collapse_after_tool_calls: u32::MAX,
        max_context_tokens: 100_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        system_prompt_provider: None,
        audit_log: None,
    });
    assert_eq!(agent.info().tool_count, 0);

    let mut new_reg = MockRegistry::new();
    new_reg.register(Arc::new(MockTool::new("alpha", "ok")));
    new_reg.register(Arc::new(MockTool::new("beta", "ok")));
    new_reg.register(Arc::new(MockTool::new("gamma", "ok")));
    agent.swap_registry(Box::new(new_reg));
    assert_eq!(agent.info().tool_count, 3);
}

// ─── Pure helper coverage: provider-error classification + builders ───────────

use crate::application::agent_usage::UsageTotals;
use crate::domain::error::DomainError;
use crate::domain::message::{LlmResponse, Message, ToolCall};

#[test]
fn is_context_limit_error_matches_known_phrases() {
    use super::is_context_or_output_limit_error as f;
    assert!(f("maximum context length exceeded for this model token"));
    assert!(f("the context length is too long, too many tokens"));
    assert!(f("context window exceeded: too many tokens"));
    assert!(f("requested 5000 tokens but limit reached"));
    assert!(f("max_tokens is too large for this context"));
    assert!(f("max output tokens exceeds context"));
}

#[test]
fn is_context_limit_error_rejects_unrelated() {
    use super::is_context_or_output_limit_error as f;
    assert!(!f("authentication failed"));
    assert!(!f("rate limited, try again"));
    assert!(!f(""));
    // Contains "token" but no context/limit phrase → not a limit error.
    assert!(!f("invalid token provided"));
}

#[test]
fn enhance_provider_error_appends_limit_hint() {
    let err = DomainError::Provider("maximum context length exceeded: too many tokens".into());
    match super::enhance_provider_error(err) {
        DomainError::Provider(m) => assert!(m.contains("Context/output limit")),
        other => panic!("expected Provider, got {other:?}"),
    }
}

#[test]
fn enhance_provider_error_does_not_double_append() {
    let err = DomainError::Provider("Context/output limit: too many tokens in context".into());
    match super::enhance_provider_error(err) {
        DomainError::Provider(m) => {
            assert_eq!(m.matches("Context/output limit").count(), 1);
        }
        other => panic!("expected Provider, got {other:?}"),
    }
}

#[test]
fn enhance_provider_error_passes_through_non_limit() {
    // An Unknown-class provider error (no recognised status/keyword) carries no
    // class guidance and must pass through verbatim.
    let err = DomainError::Provider("something unexpected went wrong".into());
    match super::enhance_provider_error(err) {
        DomainError::Provider(m) => assert_eq!(m, "something unexpected went wrong"),
        other => panic!("expected Provider, got {other:?}"),
    }
}

#[test]
fn enhance_provider_error_leaves_non_provider_unchanged() {
    let err = DomainError::Tool("boom".into());
    match super::enhance_provider_error(err) {
        DomainError::Tool(m) => assert_eq!(m, "boom"),
        other => panic!("expected Tool, got {other:?}"),
    }
}

#[test]
fn finalize_text_response_builds_result_and_appends_message() {
    let mut messages = vec![Message::user("hi")];
    let resp = LlmResponse {
        content: Some("answer".into()),
        tool_calls: vec![],
        usage: None,
        stop_reason: None,
        thinking_blocks: vec![],
    };
    let pre_response_tokens = context_pruning::estimate_total_tokens(&messages);
    let result = AgentLoopImpl::finalize_text_response(
        &mut messages,
        resp,
        3,
        UsageTotals::default(),
        pre_response_tokens,
    );
    assert_eq!(result.response, "answer");
    assert_eq!(result.tool_iterations, 3);
    assert!(!result.iteration_limit_reached);
    assert_eq!(messages.len(), 2);
}

#[test]
fn finalize_text_response_defaults_empty_content() {
    let mut messages: Vec<Message> = vec![];
    let resp = LlmResponse {
        content: None,
        tool_calls: vec![],
        usage: None,
        stop_reason: None,
        thinking_blocks: vec![],
    };
    let pre_response_tokens = context_pruning::estimate_total_tokens(&messages);
    let result = AgentLoopImpl::finalize_text_response(
        &mut messages,
        resp,
        0,
        UsageTotals::default(),
        pre_response_tokens,
    );
    assert!(result.response.is_empty());
    assert_eq!(messages.len(), 1);
}

#[test]
fn build_tool_message_populates_metadata() {
    let (agent, _p) = super::tests::make_agent(vec![], vec![]);
    let tc = ToolCall {
        id: "c1".into(),
        name: "bash".into(),
        arguments: r#"{"cmd":"ls"}"#.into(),
    };
    let msg = agent.build_tool_message(ToolMessageArgs {
        tc: &tc,
        content: "output".into(),
        image_blocks: vec![],
        spill_id: "turn1:bash:0".into(),
        is_error: false,
    });
    assert_eq!(msg.tool_name.as_deref(), Some("bash"));
    assert_eq!(msg.spill_id.as_deref(), Some("turn1:bash:0"));
    assert_eq!(msg.content, "output");
    assert!(!msg.is_error);
    assert!(msg.input_preview.is_some());
}

#[test]
fn build_chat_request_omits_session_id_when_empty() {
    let (agent, _p) = super::tests::make_agent(vec![], vec![]);
    let messages: Vec<Message> = vec![];
    let req = agent.build_chat_request(&messages, agent.tool_definitions());
    assert!(req.session_id.is_none());
    assert_eq!(req.model, "test-model");
    assert_eq!(req.max_tokens, 1024);
}

#[test]
fn model_getter_and_setter_roundtrip() {
    let (mut agent, _p) = super::tests::make_agent(vec![], vec![]);
    assert_eq!(agent.model(), "test-model");
    agent.set_model("claude-haiku-4-5".into(), None);
    assert_eq!(agent.model(), "claude-haiku-4-5");
    assert_eq!(agent.max_context_tokens(), 190_000);
}
