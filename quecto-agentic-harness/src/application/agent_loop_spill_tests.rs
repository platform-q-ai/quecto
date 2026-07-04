/// Spill-to-disk tests for the agent loop.
///
/// Split from `agent_loop_tests.rs` to keep files within the 750-line limit.
/// Uses shared mock infrastructure from `super::tests`.
use super::tests::{MockProvider, MockRegistry, MockTool, text_response, tool_call_response};
use super::*;
use crate::domain::message::{Message, Role};
use crate::domain::session::{ContextSpillStore, SpillEntry};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

/// Mock spill store that records appended entries.
#[derive(Debug, Default)]
struct MockSpillStore {
    entries: Mutex<Vec<SpillEntry>>,
}

impl ContextSpillStore for MockSpillStore {
    fn append(
        &self,
        _session_key: &str,
        entry: &SpillEntry,
    ) -> Pin<Box<dyn Future<Output = Result<(), crate::domain::error::DomainError>> + Send + '_>>
    {
        self.entries.lock().unwrap().push(entry.clone());
        Box::pin(async { Ok(()) })
    }

    fn recall(
        &self,
        _session_key: &str,
        _id: &str,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<Option<SpillEntry>, crate::domain::error::DomainError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async { Ok(None) })
    }

    fn list_entries(
        &self,
        _session_key: &str,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        Arc<Vec<crate::domain::session::SpillIndex>>,
                        crate::domain::error::DomainError,
                    >,
                > + Send
                + '_,
        >,
    > {
        Box::pin(async { Ok(Arc::new(vec![])) })
    }

    fn clear(
        &self,
        _session_key: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), crate::domain::error::DomainError>> + Send + '_>>
    {
        Box::pin(async { Ok(()) })
    }
}

#[tokio::test]
async fn test_spill_preserves_message_content_after_spill() {
    let spill_store = Arc::new(MockSpillStore::default());
    let provider = Arc::new(MockProvider::new(vec![
        tool_call_response("bash", r#"{"command":"echo hi"}"#),
        text_response("done"),
    ]));
    let mut registry = MockRegistry::new();
    registry.register(Arc::new(MockTool::new("bash", "big output here")));

    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(registry),
        model: "test-model".to_string(),
        max_tokens: 1024,
        temperature: 0.7,
        spill_store: Some(spill_store.clone()),
        session_key: "test-session".to_string(),
        context_collapse_after_tool_calls: u32::MAX,
        max_context_tokens: 190_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        system_prompt_provider: None,
        audit_log: None,
    });

    let mut messages = vec![Message::user("run it")];
    agent.run_loop(&mut messages).await.unwrap();

    let tool_msg = messages.iter().find(|m| m.role == Role::Tool).unwrap();
    assert_eq!(
        tool_msg.content, "big output here",
        "tool message content must be preserved after spill"
    );

    let entries = spill_store.entries.lock().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].content, "big output here");
}
