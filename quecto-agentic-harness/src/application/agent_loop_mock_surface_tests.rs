use crate::application::agent_loop::tests::{MockProvider, MockRegistry, MockTool, text_response};
use crate::domain::provider::{ChatRequest, LlmProvider, StreamEvent};
use crate::domain::tool::{Tool, ToolRegistry};
use std::sync::Arc;

#[test]
fn mock_tool_debug_trait_surface_includes_name() {
    let tool = MockTool::new("bash", "ok");
    assert!(format!("{tool:?}").contains("bash"));
}

#[tokio::test]
async fn mock_registry_and_tool_default_trait_surface_methods_are_inert() {
    let tool = Arc::new(MockTool::new("bash", "ok"));
    tool.set_session_key("session".into());
    assert_eq!(tool.execute("{}").await.unwrap().content, "ok");

    let mut registry = MockRegistry::new();
    registry.set_session_key("session");
    registry.register_extension(tool.clone());
    registry.unregister_extension("bash");
    assert!(registry.extension_names().is_empty());
    registry.register(tool);
    assert_eq!(registry.tool_count(), 1);
    assert_eq!(registry.execute("bash", "{}").await.unwrap().content, "ok");
}

#[tokio::test]
async fn mock_provider_default_as_any_and_incremental_surface() {
    let provider = MockProvider::new(vec![text_response("done")]);
    assert!(provider.as_any().is::<()>());
    let messages = [];
    let tools = [];
    let request = ChatRequest {
        messages: &messages,
        tools: &tools,
        model: "test-model",
        max_tokens: 9,
        temperature: 0.0,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    let mut rx = provider.chat_stream_incremental(request).await;
    assert!(matches!(rx.recv().await, Some(StreamEvent::Done(_))));
}
