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
        id: &str,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<Option<SpillEntry>, crate::domain::error::DomainError>>
                + Send
                + '_,
        >,
    > {
        let found = self
            .entries
            .lock()
            .unwrap()
            .iter()
            .find(|e| e.id == id)
            .cloned();
        Box::pin(async move { Ok(found) })
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
        let index: Vec<crate::domain::session::SpillIndex> = self
            .entries
            .lock()
            .unwrap()
            .iter()
            .map(|e| crate::domain::session::SpillIndex {
                id: e.id.clone(),
                tool: e.tool.clone(),
                input_preview: e.input_preview.clone(),
                tokens: e.tokens,
            })
            .collect();
        Box::pin(async move { Ok(Arc::new(index)) })
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

    // Conversation messages spill too (#1046); the tool output must be the
    // single tool-spill entry, with its content intact.
    let entries = spill_store.entries.lock().unwrap();
    let tool_entries: Vec<_> = entries.iter().filter(|e| e.tool == "bash").collect();
    assert_eq!(tool_entries.len(), 1);
    assert_eq!(tool_entries[0].content, "big output here");
}

// --- #951: budget pruner spills conversation messages, tail-pins recent turns ---

/// Agent with a tight token-budget ceiling, no tools, and a mock spill store.
fn tight_budget_agent(
    spill_store: Arc<MockSpillStore>,
    max_context_tokens: usize,
) -> AgentLoopImpl {
    let provider = Arc::new(MockProvider::new(vec![text_response("done")]));
    AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(MockRegistry::new()),
        model: "test-model".to_string(),
        max_tokens: 1024,
        temperature: 0.7,
        spill_store: Some(spill_store),
        session_key: "test-session".to_string(),
        context_collapse_after_tool_calls: u32::MAX,
        max_context_tokens,
        progress_callback: None,
        streaming: false,
        effort: None,
        system_prompt_provider: None,
        audit_log: None,
    })
}

/// An old assistant turn plus the in-flight user prompt, sized so a 200-token
/// ceiling must drop the assistant turn.
fn history_with_old_assistant_turn(big: &str) -> Vec<Message> {
    let mut old_assistant = Message::assistant(big, vec![]);
    old_assistant.turn = Some(1);
    vec![
        Message::user("old prompt"),
        old_assistant,
        Message::user("new prompt"),
    ]
}

#[tokio::test]
async fn budget_pruned_assistant_turn_is_spilled_and_recallable() {
    let spill_store = Arc::new(MockSpillStore::default());
    let agent = tight_budget_agent(spill_store.clone(), 200);
    let big = "x".repeat(2000); // ~500 tokens, over the 200-token ceiling
    let mut messages = history_with_old_assistant_turn(&big);

    agent.run_loop(&mut messages).await.unwrap();

    // End-to-end recallability (through the real recall tool) is covered by
    // the BDD scenario "Budget-dropped assistant message is recallable";
    // recalling through the mock here would only verify the mock itself.
    let entries = spill_store.entries.lock().unwrap();
    let entry = entries
        .iter()
        .find(|e| e.id == "turn1:msg:assistant")
        .expect("budget-dropped assistant turn must be spilled with id turn1:msg:assistant");
    assert_eq!(
        entry.content, big,
        "spilled content must be the full dropped assistant text"
    );
    assert_eq!(
        entry.tool, "assistant",
        "message spills must carry the role so the manifest can distinguish them"
    );
}

#[tokio::test]
async fn budget_pruned_user_message_is_spilled_with_role_id() {
    let spill_store = Arc::new(MockSpillStore::default());
    let agent = tight_budget_agent(spill_store.clone(), 200);
    let big = "x".repeat(2000); // ~500 tokens, over the 200-token ceiling
    let mut old_user = Message::user(&big);
    old_user.turn = Some(1);
    let mut messages = vec![old_user, Message::user("new prompt")];

    agent.run_loop(&mut messages).await.unwrap();

    let entries = spill_store.entries.lock().unwrap();
    let entry = entries
        .iter()
        .find(|e| e.id == "turn1:msg:user")
        .expect("budget-dropped user message must be spilled with id turn1:msg:user");
    assert_eq!(entry.content, big);
    assert_eq!(entry.tool, "user", "message spills must carry the role");
}

#[tokio::test]
async fn ceiling_message_spill_is_reflected_in_manifest_without_tool_calls() {
    let spill_store = Arc::new(MockSpillStore::default());
    let agent = tight_budget_agent(spill_store.clone(), 200);
    let big = "x".repeat(2000);
    let mut messages = history_with_old_assistant_turn(&big);

    agent.run_loop(&mut messages).await.unwrap();

    let manifest = messages
        .iter()
        .find(|m| m.is_manifest)
        .expect("a ceiling message spill on a no-tool turn must produce/refresh the manifest");
    assert!(
        manifest.content.contains("turn1:msg:assistant"),
        "manifest must reflect the message spill; got: {}",
        manifest.content
    );
}

#[tokio::test]
async fn current_user_prompt_survives_tight_budget() {
    let spill_store = Arc::new(MockSpillStore::default());
    let agent = tight_budget_agent(spill_store.clone(), 50);
    let big_prompt = "y".repeat(600); // ~150 tokens, over the 50-token ceiling
    // Earlier conversation comes first so the prompt under test is trailing
    // but NOT the first user message — a "pin the first user message only"
    // implementation must fail this test.
    let mut earlier_assistant = Message::assistant("earlier answer", vec![]);
    earlier_assistant.turn = Some(1);
    let mut messages = vec![
        Message::user("earlier prompt"),
        earlier_assistant,
        Message::user(&big_prompt),
    ];

    agent.run_loop(&mut messages).await.unwrap();

    assert!(
        messages
            .iter()
            .any(|m| m.role == Role::User && m.content == big_prompt),
        "the in-flight user prompt is pinned and must never be dropped by the ceiling"
    );
}
