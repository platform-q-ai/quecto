use super::tests::{MockProvider, MockRegistry, MockTool, text_response, tool_call_response};
use super::*;
use crate::domain::agent::AgentProgressEvent;
use crate::domain::message::{LlmResponse, Message, Role, ToolCall};
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
        context_collapse_after_turns: u32::MAX,
        max_context_tokens: 190_000,
        progress_callback,
        streaming: false,
        effort: None,
        system_prompt_provider,
        audit_log: None,
    }
}

#[tokio::test]
async fn tool_result_preview_is_only_built_when_progress_is_observed() {
    let provider = Arc::new(MockProvider::new(vec![
        tool_call_response("read", r#"{"path":"notes.txt"}"#),
        text_response("done"),
    ]));
    let mut registry = MockRegistry::new();
    registry.register(Arc::new(MockTool::new("read", "€")));
    let agent = AgentLoopImpl::new(agent_config(provider, registry, None, None));
    let mut messages = vec![Message::user("read")];

    agent_loop_preview::reset_built_preview_count_for_tests();
    agent.run_loop(&mut messages).await.unwrap();

    assert_eq!(agent_loop_preview::built_preview_count_for_tests(), 0);
}

#[tokio::test]
async fn tool_result_preview_is_built_once_for_observed_tool_finish() {
    let provider = Arc::new(MockProvider::new(vec![
        tool_call_response("read", r#"{"path":"notes.txt"}"#),
        text_response("done"),
    ]));
    let mut registry = MockRegistry::new();
    registry.register(Arc::new(MockTool::new("read", "€")));
    let events = Arc::new(Mutex::new(Vec::new()));
    let events_for_callback = Arc::clone(&events);
    let callback: crate::domain::agent::ProgressCallback = Arc::new(move |event| {
        events_for_callback.lock().unwrap().push(event);
    });
    let agent = AgentLoopImpl::new(agent_config(provider, registry, Some(callback), None));
    let mut messages = vec![Message::user("read")];

    agent_loop_preview::reset_built_preview_count_for_tests();
    agent.run_loop(&mut messages).await.unwrap();

    assert_eq!(agent_loop_preview::built_preview_count_for_tests(), 1);
    assert!(events.lock().unwrap().iter().any(|event| {
        matches!(event, AgentProgressEvent::ToolFinished { result_content, .. } if result_content == "€")
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
                    id: "call_read".to_string(),
                    name: "read".to_string(),
                    arguments: first_arguments.clone(),
                },
                ToolCall {
                    id: "call_write".to_string(),
                    name: "write".to_string(),
                    arguments: second_arguments.clone(),
                },
            ],
            usage: None,
            stop_reason: None,
            thinking_blocks: vec![],
        },
        text_response("done"),
    ]));
    let mut registry = MockRegistry::new();
    registry.register(Arc::new(MockTool::new("read", "notes")));
    registry.register(Arc::new(MockTool::new("write", "ok")));
    let events = Arc::new(Mutex::new(Vec::new()));
    let events_for_callback = Arc::clone(&events);
    let callback: crate::domain::agent::ProgressCallback = Arc::new(move |event| {
        events_for_callback.lock().unwrap().push(event);
    });
    let agent = AgentLoopImpl::new(agent_config(provider, registry, Some(callback), None));
    let mut messages = vec![Message::user("copy")];

    agent.run_loop(&mut messages).await.unwrap();

    assert_eq!(messages[1].role, Role::Assistant);
    assert_eq!(messages[1].tool_calls[0].arguments, first_arguments);
    assert_eq!(messages[1].tool_calls[1].arguments, second_arguments);
    assert_eq!(messages[2].role, Role::Tool);
    assert_eq!(messages[2].tool_call_id.as_deref(), Some("call_read"));
    assert_eq!(messages[3].role, Role::Tool);
    assert_eq!(messages[3].tool_call_id.as_deref(), Some("call_write"));
    assert_eq!(messages[4].role, Role::Assistant);
    let completed_messages = events
        .lock()
        .unwrap()
        .iter()
        .find_map(|event| match event {
            AgentProgressEvent::TurnCompleted { messages } => Some(messages.clone()),
            _ => None,
        })
        .expect("turn-completed event emitted");
    assert_eq!(completed_messages.len(), 3);
    assert_eq!(completed_messages[0].role, Role::Assistant);
    assert_eq!(
        completed_messages[0].tool_calls[0].arguments,
        first_arguments
    );
    assert_eq!(
        completed_messages[0].tool_calls[1].arguments,
        second_arguments
    );
    assert_eq!(completed_messages[1].role, Role::Tool);
    assert_eq!(
        completed_messages[1].tool_call_id.as_deref(),
        Some("call_read")
    );
    assert_eq!(completed_messages[2].role, Role::Tool);
    assert_eq!(
        completed_messages[2].tool_call_id.as_deref(),
        Some("call_write")
    );
}
