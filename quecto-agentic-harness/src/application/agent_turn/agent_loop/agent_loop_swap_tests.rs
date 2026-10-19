use super::tests::{MockProvider, MockRegistry, MockTool};
use super::*;
use crate::application::agent_turn::ports::AgentLoop;
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
        retention: None,
        session_key: String::new(),
        max_context_tokens: 100_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_marks: Default::default(),
        model_context_window: None,
        tool_profile_context:
            crate::domain::tool_policy::value_objects::tool::ToolProfileContext::Parent,
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
        retention: None,
        session_key: String::new(),
        max_context_tokens: 100_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_marks: Default::default(),
        model_context_window: None,
        tool_profile_context:
            crate::domain::tool_policy::value_objects::tool::ToolProfileContext::Parent,
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

use crate::application::agent_turn::agent_loop::agent_usage::UsageTotals;
use crate::domain::conversation::value_objects::message::{LlmResponse, Message, ToolCall};

/// Minimal agent for exercising instance methods like `finalize_text_response`.
fn bare_agent() -> AgentLoopImpl {
    AgentLoopImpl::new(AgentLoopConfig {
        provider: Arc::new(MockProvider::new(vec![])),
        tool_registry: Box::new(MockRegistry::new()),
        model: "m".into(),
        max_tokens: 100,
        temperature: 0.0,
        retention: None,
        session_key: String::new(),
        max_context_tokens: 100_000,
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
    let result =
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(bare_agent().finalize_text_response(
                &mut messages,
                resp,
                super::TurnEnd {
                    iterations: 3,
                    usage: UsageTotals::default(),
                    pre_response_context_tokens: pre_response_tokens,
                    current_turn: 1,
                },
            ));
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
    let result =
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(bare_agent().finalize_text_response(
                &mut messages,
                resp,
                super::TurnEnd {
                    iterations: 0,
                    usage: UsageTotals::default(),
                    pre_response_context_tokens: pre_response_tokens,
                    current_turn: 1,
                },
            ));
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
        is_error: false,
    });
    assert_eq!(msg.tool_name.as_deref(), Some("bash"));
    assert_eq!(
        msg.spill_id, None,
        "spill_id is stamped by spill_tool_message on a successful append, never at build time"
    );
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
    crate::application::catalogue::ports::ModelRuntime::apply_model(
        &mut agent,
        "claude-haiku-4-5".into(),
        crate::application::catalogue::dto::ModelLimits {
            max_output_tokens: None,
            context_window: None,
            prompt_limit: Default::default(),
            image_input: Default::default(),
        },
    );
    assert_eq!(agent.model(), "claude-haiku-4-5");
    assert_eq!(agent.max_context_tokens(), 190_000);
}
