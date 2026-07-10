use super::tests::{MockProvider, MockRegistry, MockTool, text_response, tool_call_response};
use super::*;
use crate::domain::agent::AgentProgressEvent;
use crate::domain::message::{
    LlmResponse, Message, Role, StopReason, ThinkingBlock, ToolCall,
    reset_tool_call_clone_count_for_tests, tool_call_clone_count_for_tests,
};
use std::sync::{Arc, Mutex};

fn agent_config(
    provider: Arc<MockProvider>,
    registry: MockRegistry,
    progress_callback: Option<crate::domain::agent::ProgressCallback>,
    system_prompt_provider: Option<Arc<dyn Fn() -> String + Send + Sync>>,
) -> AgentLoopConfig {
    AgentLoopConfig {
        provider,
        tool_registry: Box::new(registry),
        model: "test-model".to_string(),
        max_tokens: 1024,
        temperature: 0.7,
        spill_store: None,
        session_key: String::new(),
        context_collapse_after_tool_calls: u32::MAX,
        max_context_tokens: 190_000,
        progress_callback,
        streaming: false,
        effort: None,
        system_prompt_provider,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
    }
}

#[tokio::test]
async fn tool_result_preview_is_only_built_when_progress_is_observed() {
    let provider = Arc::new(MockProvider::new(vec![
        tool_call_response("read", r#"{"path":"notes.txt"}"#),
        text_response("done"),
    ]));
    let mut registry = MockRegistry::new();
    let content = "headless-preview-sentinel-993";
    registry.register(Arc::new(MockTool::new("read", content)));
    let agent = AgentLoopImpl::new(agent_config(provider, registry, None, None));
    let mut messages = vec![Message::user("read")];

    agent_loop_preview::reset_built_preview_count_for_tests(content);
    agent.run_loop(&mut messages).await.unwrap();

    assert_eq!(
        agent_loop_preview::built_preview_count_for_tests(content),
        0
    );
}

#[tokio::test]
async fn tool_result_preview_is_built_once_for_observed_tool_finish() {
    let provider = Arc::new(MockProvider::new(vec![
        tool_call_response("read", r#"{"path":"notes.txt"}"#),
        text_response("done"),
    ]));
    let mut registry = MockRegistry::new();
    let content = "observed-preview-sentinel-993";
    registry.register(Arc::new(MockTool::new("read", content)));
    let events = Arc::new(Mutex::new(Vec::new()));
    let events_for_callback = Arc::clone(&events);
    let callback: crate::domain::agent::ProgressCallback = Arc::new(move |event| {
        events_for_callback.lock().unwrap().push(event);
    });
    let agent = AgentLoopImpl::new(agent_config(provider, registry, Some(callback), None));
    let mut messages = vec![Message::user("read")];

    agent_loop_preview::reset_built_preview_count_for_tests(content);
    agent.run_loop(&mut messages).await.unwrap();

    assert_eq!(
        agent_loop_preview::built_preview_count_for_tests(content),
        1
    );
    assert!(events.lock().unwrap().iter().any(|event| {
        matches!(event, AgentProgressEvent::ToolFinished { result_content, .. } if result_content == content)
    }));
}

#[tokio::test]
async fn unchanged_dynamic_system_prompt_reuses_message_token_cache() {
    let provider = Arc::new(MockProvider::new(vec![
        text_response("first"),
        text_response("second"),
    ]));
    let agent = AgentLoopImpl::new(agent_config(
        Arc::clone(&provider),
        MockRegistry::new(),
        None,
        Some(Arc::new(|| "stable instructions".to_string())),
    ));
    let mut messages = vec![Message::user("first")];

    agent.run_loop(&mut messages).await.unwrap();
    messages.push(Message::user("second"));
    agent.run_loop(&mut messages).await.unwrap();

    let system_message = messages
        .iter()
        .find(|message| message.role == Role::System)
        .expect("system prompt retained");
    assert_eq!(provider.request_count(), 2);
    assert_eq!(system_message.cached_token_build_count_for_tests(), 1);
}

#[tokio::test]
async fn changed_dynamic_system_prompt_invalidates_message_token_cache() {
    let provider = Arc::new(MockProvider::new(vec![
        text_response("first"),
        text_response("second"),
    ]));
    let prompts = Arc::new(Mutex::new(
        vec![
            "first instructions".to_string(),
            "second instructions".to_string(),
        ]
        .into_iter(),
    ));
    let agent = AgentLoopImpl::new(agent_config(
        Arc::clone(&provider),
        MockRegistry::new(),
        None,
        Some(Arc::new(move || {
            prompts.lock().unwrap().next().expect("prompt available")
        })),
    ));
    let mut messages = vec![Message::user("first")];

    agent.run_loop(&mut messages).await.unwrap();
    messages.push(Message::user("second"));
    agent.run_loop(&mut messages).await.unwrap();

    let system_message = messages
        .iter()
        .find(|message| message.role == Role::System)
        .expect("system prompt retained");
    assert_eq!(provider.request_count(), 2);
    assert_eq!(system_message.content, "second instructions");
    assert_eq!(
        system_message.cached_token_build_count_for_tests(),
        2,
        "a changed dynamic prompt must invalidate and rebuild the token cache"
    );
}

#[tokio::test]
async fn tool_turn_preserves_message_order_and_tool_arguments() {
    let first_arguments = format!(
        r#"{{"path":"notes.txt","content":"{}"}}"#,
        "x".repeat(64 * 1024)
    );
    let second_arguments = format!(
        r#"{{"path":"out.txt","content":"{}"}}"#,
        "y".repeat(64 * 1024)
    );
    let provider = Arc::new(MockProvider::new(vec![
        LlmResponse {
            content: None,
            tool_calls: vec![
                ToolCall {
                    id: "call_993_read".to_string(),
                    name: "read".to_string(),
                    arguments: first_arguments.clone(),
                },
                ToolCall {
                    id: "call_993_write".to_string(),
                    name: "write".to_string(),
                    arguments: second_arguments.clone(),
                },
            ],
            usage: None,
            stop_reason: Some(StopReason::ToolUse),
            thinking_blocks: vec![ThinkingBlock::Normal {
                thinking: "use both tools".to_string(),
                signature: "sig-1".to_string(),
            }],
        },
        text_response("done"),
    ]));
    let mut registry = MockRegistry::new();
    registry.register(Arc::new(MockTool::new("read", "notes")));
    registry.register(Arc::new(MockTool::new("write", "ok")));
    let agent = AgentLoopImpl::new(agent_config(provider, registry, None, None));
    let mut messages = vec![Message::user("copy")];

    reset_tool_call_clone_count_for_tests("call_993_read");
    reset_tool_call_clone_count_for_tests("call_993_write");
    agent.run_loop(&mut messages).await.unwrap();

    assert_eq!(messages[1].role, Role::Assistant);
    assert_eq!(messages[1].tool_calls[0].arguments, first_arguments);
    assert_eq!(messages[1].tool_calls[1].arguments, second_arguments);
    assert_eq!(messages[1].stop_reason, Some(StopReason::ToolUse));
    assert_eq!(messages[1].thinking_blocks.len(), 1);
    // The run append ledger owns one clone so AgentEnd remains correct even if
    // active-context pruning removes these messages before the run completes.
    assert_eq!(tool_call_clone_count_for_tests("call_993_read"), 1);
    assert_eq!(tool_call_clone_count_for_tests("call_993_write"), 1);
    assert_eq!(messages[2].role, Role::Tool);
    assert_eq!(messages[2].tool_call_id.as_deref(), Some("call_993_read"));
    assert_eq!(messages[3].role, Role::Tool);
    assert_eq!(messages[3].tool_call_id.as_deref(), Some("call_993_write"));
    assert_eq!(messages[4].role, Role::Assistant);
}
