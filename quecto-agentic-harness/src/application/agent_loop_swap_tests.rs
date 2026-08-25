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
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
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
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
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

/// Minimal agent for exercising instance methods like `finalize_text_response`.
fn bare_agent() -> AgentLoopImpl {
    AgentLoopImpl::new(AgentLoopConfig {
        provider: Arc::new(MockProvider::new(vec![])),
        tool_registry: Box::new(MockRegistry::new()),
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
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
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
    agent.set_model("claude-haiku-4-5".into(), None, None);
    assert_eq!(agent.model(), "claude-haiku-4-5");
    assert_eq!(agent.max_context_tokens(), 190_000);
}

#[derive(Debug)]
struct DescriptorProvider {
    descriptors: Vec<crate::domain::catalogue::ModelDescriptor>,
}

impl crate::domain::provider::LlmProvider for DescriptorProvider {
    fn name(&self) -> &str {
        "descriptor"
    }
    fn model_descriptors(&self) -> Option<&[crate::domain::catalogue::ModelDescriptor]> {
        Some(&self.descriptors)
    }
    fn chat<'a>(
        &'a self,
        _request: crate::domain::provider::ChatRequest<'a>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        crate::domain::message::LlmResponse,
                        crate::domain::error::DomainError,
                    >,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async { Err(crate::domain::error::DomainError::Provider("unused".into())) })
    }
}

fn descriptor_model(
    max_tokens: u32,
    context_window: u32,
) -> crate::domain::catalogue::ModelDescriptor {
    use crate::domain::catalogue::*;
    ModelDescriptor {
        reference: ModelRef::parse("mock", "model").unwrap(),
        display_name: None,
        transport: TransportKind::OpenAiCompletions,
        auth: AuthIdentity::ApiKey,
        base_url: None,
        auth_header: true,
        allow_remote_http: true,
        configured: true,
        capabilities: ModelCapabilities {
            input: vec!["text".into()],
            context_window,
            max_tokens,
            context_window_explicit: true,
            max_tokens_explicit: true,
            reasoning: false,
            cost: ModelCost::default(),
        },
        availability: Availability::Runnable,
    }
}

#[test]
fn swap_provider_rederives_active_model_limits_from_reloaded_generation() {
    let provider = Arc::new(DescriptorProvider {
        descriptors: vec![descriptor_model(100, 1000)],
    });
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(MockRegistry::new()),
        model: "mock/model".into(),
        max_tokens: 500,
        temperature: 0.0,
        spill_store: None,
        session_key: String::new(),
        context_collapse_after_tool_calls: u32::MAX,
        max_context_tokens: 100_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: Some(1000),
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
    });

    // The provider deliberately advertises different descriptors: publication
    // must use the catalogue carried by the application-owned runtime snapshot.
    let reloaded_provider: Arc<dyn crate::domain::provider::LlmProvider> =
        Arc::new(DescriptorProvider {
            descriptors: vec![descriptor_model(999, 9999)],
        });
    agent.swap_runtime(
        crate::application::provider_runtime::CatalogueRuntimeSnapshot {
            catalogue: crate::domain::catalogue::CatalogueSnapshot::new(
                42,
                vec![descriptor_model(250, 4000)],
            ),
            provider: reloaded_provider.clone(),
        },
    );

    assert!(Arc::ptr_eq(&agent.provider, &reloaded_provider));
    assert_eq!(agent.catalogue.generation, 42);
    assert_eq!(agent.catalogue_store.current().generation, 42);
    assert_eq!(
        agent.catalogue_store.current().models()[0]
            .capabilities
            .max_tokens,
        250
    );
    assert_eq!(agent.catalogue.models()[0].capabilities.max_tokens, 250);
    assert_eq!(agent.model_max_tokens, Some(250));
    assert_eq!(agent.model_context_window, Some(4000));
}

#[test]
fn swap_provider_clears_stale_limits_when_descriptors_are_absent() {
    let provider = Arc::new(DescriptorProvider {
        descriptors: vec![descriptor_model(100, 1_000)],
    });
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(MockRegistry::new()),
        model: "mock/model".into(),
        max_tokens: 500,
        temperature: 0.0,
        spill_store: None,
        session_key: String::new(),
        context_collapse_after_tool_calls: u32::MAX,
        max_context_tokens: 100_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: Some(1_000),
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
    });

    agent.swap_provider(Arc::new(DescriptorProvider {
        descriptors: vec![descriptor_model(100, 1_000)],
    }));
    assert_eq!(agent.effective_max_context_tokens(), 1_000);

    agent.swap_provider(Arc::new(MockProvider::new(vec![])));

    assert_eq!(agent.model_max_tokens, None);
    assert_eq!(agent.model_context_window, None);
    assert_eq!(agent.effective_max_context_tokens(), 100_000);
}
