//! #1072: mid-run pruning vs positional watermarks — agent-loop level.
//!
//! Pins the per-run appended-message ledger and the durable-prefix dirty
//! latch against every demotion shape the #1046 ladder can produce mid-run:
//! in-place stub collapse (message identity unchanged), physical drops that
//! shrink history below its pre-turn length, demotion of the run's OWN
//! earlier messages, and the #931 malformed-feedback append. Also pins the
//! append-only contract of the `appended_from` interval.

use super::tests::{MockProvider, MockRegistry, MockTool, text_response, tool_call_response};
use super::{AgentLoopConfig, AgentLoopImpl};
use crate::domain::error::DomainError;
use crate::domain::message::{Message, Role};
use crate::domain::session::{ContextSpillStore, SpillEntry};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

#[derive(Debug, Default)]
struct MemSpillStore {
    entries: Mutex<Vec<SpillEntry>>,
}

impl ContextSpillStore for MemSpillStore {
    fn append(
        &self,
        _session_key: &str,
        entry: &SpillEntry,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        self.entries.lock().unwrap().push(entry.clone());
        Box::pin(async { Ok(()) })
    }

    fn recall(
        &self,
        _session_key: &str,
        id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<SpillEntry>, DomainError>> + Send + '_>> {
        let found = self
            .entries
            .lock()
            .unwrap()
            .iter()
            .find(|e| e.id == id)
            .cloned();
        Box::pin(async move { Ok(found) })
    }

    fn list_entries(&self, _session_key: &str) -> crate::domain::session::SpillIndexList<'_> {
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
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        self.entries.lock().unwrap().clear();
        Box::pin(async { Ok(()) })
    }
}

fn agent_with(
    provider: MockProvider,
    registry: MockRegistry,
    spill_store: Option<Arc<dyn ContextSpillStore>>,
    max_context_tokens: usize,
) -> AgentLoopImpl {
    AgentLoopImpl::new(AgentLoopConfig {
        provider: Arc::new(provider),
        tool_registry: Box::new(registry),
        model: "test-model".to_string(),
        max_tokens: 1024,
        temperature: 0.0,
        spill_store,
        session_key: "test-1072".to_string(),
        context_collapse_after_tool_calls: u32::MAX,
        max_context_tokens,
        progress_callback: None,
        streaming: false,
        effort: None,
        system_prompt_provider: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
    })
}

/// A spilled assistant message from an earlier prompt: `turn` stamped and
/// `spill_id` set, so ladder rung 1 may stub it in place.
fn spilled_history_message(turn: u32, content: &str) -> Message {
    let mut msg = Message::assistant(content, vec![]);
    msg.turn = Some(turn);
    msg.spill_id = Some(format!("turn{turn}:msg:assistant"));
    msg
}

fn big_content() -> String {
    // ~2400 ASCII chars ≈ 600 estimated tokens.
    "lorem ipsum dolor sit amet ".repeat(90)
}

// ─── (a) dirty latch: in-place stub demotion must latch dirty ────────────────

/// RED for #1072 finding 6: rung-1 demotion mutates messages IN PLACE — the
/// UUID sequence of the durable prefix is unchanged, so an id-snapshot
/// comparison misses it. The dirty signal must come from the ladder outcome
/// (`collapsed_to_stubs > 0 || dropped > 0`) instead.
#[tokio::test]
async fn in_place_stub_demotion_latches_durable_prefix_dirty() {
    let big = big_content();
    let mut messages = vec![
        spilled_history_message(1, &big),
        spilled_history_message(2, &big),
        spilled_history_message(3, "a small earlier reply"),
        spilled_history_message(4, "another small earlier reply"),
        Message::user("hi"),
    ];
    // Budget forces rung 1 to stub turns 1–2; turns 3–4 are tail-pinned and
    // the post-stub total fits, so NOTHING is dropped: identity is unchanged.
    let agent = agent_with(
        MockProvider::new(vec![text_response("ok")]),
        MockRegistry::new(),
        None,
        300,
    );
    let result = agent.run_loop(&mut messages).await.unwrap();

    // Positive control: the demotion really was in-place stubbing, no drops.
    assert!(
        messages[0].is_collapsed && messages[0].content.contains("recall("),
        "scenario setup: turn 1 must be stub-demoted in place, got: {}",
        messages[0].content
    );
    assert!(
        messages.iter().filter(|m| m.turn == Some(2)).count() == 1,
        "scenario setup: no message may be dropped in this stub-only scenario"
    );

    assert!(
        result.durable_prefix_dirty,
        "in-place stub demotion changes the durable prefix content (same ids!) \
         and MUST latch durable_prefix_dirty, or persistence appends a delta \
         against a durable prefix still holding the full pre-stub content"
    );
}

// ─── (b) live failure shape: shrink below pre-turn length ────────────────────

/// The live #1072 panic shape: pre_turn_len > post_turn_len. The run must
/// complete, report exactly its own appended messages, and latch dirty.
#[tokio::test]
async fn mid_run_shrink_below_pre_turn_length_reports_exact_ledger_and_dirty() {
    let big = big_content();
    let mut messages: Vec<Message> = (1..=8).map(|t| spilled_history_message(t, &big)).collect();
    messages.push(Message::user("go"));
    let pre_turn_len = messages.len();

    let mut registry = MockRegistry::new();
    registry.register(Arc::new(MockTool::new("bulk", "tool-output-payload")));
    let agent = agent_with(
        MockProvider::new(vec![
            tool_call_response("bulk", "{}"),
            text_response("final answer"),
        ]),
        registry,
        Some(Arc::new(MemSpillStore::default())),
        700,
    );
    let result = agent.run_loop(&mut messages).await.unwrap();

    assert!(
        messages.len() < pre_turn_len,
        "scenario setup: pruning must shrink history below its pre-turn \
         length ({} -> {})",
        pre_turn_len,
        messages.len()
    );

    let appended = &result.appended_messages;
    let roles: Vec<Role> = appended.iter().map(|m| m.role.clone()).collect();
    assert_eq!(
        roles,
        vec![Role::Assistant, Role::Tool, Role::Assistant],
        "the ledger must carry exactly the run's own messages, got {roles:?}"
    );
    assert_eq!(appended[0].tool_calls.len(), 1);
    assert_eq!(appended[1].content, "tool-output-payload");
    assert_eq!(appended[2].content, "final answer");
    assert!(
        result.durable_prefix_dirty,
        "physical drops must latch durable_prefix_dirty"
    );
}

// ─── (c) ledger content: the run's OWN messages demoted mid-run ─────────────

/// A long multi-tool run near the ceiling can have its OWN earlier messages
/// demoted or dropped by a later iteration's prune. The appended ledger must
/// still carry those messages as appended — never their stubbed survivors,
/// never omit them.
#[tokio::test]
async fn runs_own_demoted_messages_still_appear_in_full_in_the_ledger() {
    let big_output = "y".repeat(4000); // ~1000 tokens per tool result
    let mut registry = MockRegistry::new();
    registry.register(Arc::new(MockTool::new("bulk", &big_output)));
    let agent = agent_with(
        MockProvider::new(vec![
            tool_call_response("bulk", "{}"),
            tool_call_response("bulk", "{}"),
            text_response("done"),
        ]),
        registry,
        Some(Arc::new(MemSpillStore::default())),
        500,
    )
    .with_pin_recent_turns(1);
    let mut messages = vec![Message::user("go")];
    let result = agent.run_loop(&mut messages).await.unwrap();

    // Positive control: the run's own turn-1 tool result must no longer be
    // present in full in live context (stub-demoted or dropped mid-run).
    let live_full = messages
        .iter()
        .filter(|m| m.role == Role::Tool && m.content == big_output)
        .count();
    assert!(
        live_full <= 1,
        "scenario setup: a later prune must demote the run's own earlier \
         tool result; both are still in full"
    );

    let appended = &result.appended_messages;
    let roles: Vec<Role> = appended.iter().map(|m| m.role.clone()).collect();
    assert_eq!(
        roles,
        vec![
            Role::Assistant,
            Role::Tool,
            Role::Assistant,
            Role::Tool,
            Role::Assistant
        ],
        "the ledger must carry every message the run appended, got {roles:?}"
    );
    let ledger_full = appended
        .iter()
        .filter(|m| m.role == Role::Tool && m.content == big_output)
        .count();
    assert_eq!(
        ledger_full, 2,
        "both tool results must appear in the ledger with their as-appended \
         content even though one was demoted mid-run"
    );
}

// ─── (e) malformed feedback must extend the ledger ───────────────────────────

/// RED for #1072 re-review finding 4: `append_malformed_feedback` pushes a
/// user message into the conversation mid-run, but the ledger never records
/// it, so AgentEnd silently omits a message the run appended.
#[tokio::test]
async fn malformed_feedback_message_is_included_in_the_appended_ledger() {
    let mut registry = MockRegistry::new();
    registry.register(Arc::new(MockTool::new("echo", "echoed")));
    let agent = agent_with(
        MockProvider::new_results(vec![
            Ok(tool_call_response("echo", "{}")),
            Err(DomainError::Provider(
                "provider error (400): invalid_request_error: tool_use input is malformed"
                    .to_string(),
            )),
            Ok(text_response("recovered")),
        ]),
        registry,
        None,
        190_000,
    );
    let mut messages = vec![Message::user("go")];
    let result = agent.run_loop(&mut messages).await.unwrap();

    // Positive control: the feedback user message is in the conversation.
    assert!(
        messages
            .iter()
            .any(|m| m.role == Role::User
                && m.content.contains("rejected by the provider as malformed")),
        "scenario setup: the #931 feedback message must have been appended"
    );

    let roles: Vec<Role> = result
        .appended_messages
        .iter()
        .map(|m| m.role.clone())
        .collect();
    assert_eq!(
        roles,
        vec![Role::Assistant, Role::Tool, Role::User, Role::Assistant],
        "the malformed-request feedback user message was appended by this run \
         and must appear in the ledger, got {roles:?}"
    );
    assert!(
        result.appended_messages[2]
            .content
            .contains("rejected by the provider as malformed")
    );
}

// ─── (d) the run ledger is recorded at append time, not via a slice ─────────

/// #1072 (option d-2): the positional `appended_from` slice was migrated to
/// the ledger — `execute_tool_calls_for_response` records each message it
/// appends directly in the run ledger at the moment of the append. This test
/// pins that contract: even under a budget that would prune the oversized
/// prefix if pruning ever ran here, the ledger carries exactly the appended
/// assistant tool-call message and tool result, independent of any positional
/// arithmetic over `messages`.
#[tokio::test]
async fn tool_execution_records_its_appends_in_the_run_ledger_under_budget_pressure() {
    let big = big_content();
    let mut registry = MockRegistry::new();
    registry.register(Arc::new(MockTool::new("bulk", "tool-output-payload")));
    // Budget 100: any pruning pass inside tool execution WOULD demote/drop
    // the oversized prefix below.
    let agent = agent_with(
        MockProvider::new(vec![]),
        registry,
        Some(Arc::new(MemSpillStore::default())),
        100,
    );
    let mut messages: Vec<Message> = (1..=4).map(|t| spilled_history_message(t, &big)).collect();
    messages.push(Message::user("go"));
    let prefix: Vec<(Role, String)> = messages
        .iter()
        .map(|m| (m.role.clone(), m.content.clone()))
        .collect();

    let mut ledger = Vec::new();
    agent
        .execute_tool_calls_for_response(
            &mut messages,
            1,
            tool_call_response("bulk", "{}"),
            &mut ledger,
        )
        .await;

    let ledger_shape: Vec<(Role, String)> = ledger
        .iter()
        .map(|m| (m.role.clone(), m.content.clone()))
        .collect();
    assert_eq!(
        ledger_shape,
        vec![
            (Role::Assistant, String::new()),
            (Role::Tool, "tool-output-payload".to_string()),
        ],
        "the ledger must record exactly the assistant tool-call message and \
         the tool result, at append time"
    );
    assert_eq!(ledger[0].tool_calls.len(), 1);
    // The conversation still received the same two appends after the prefix.
    let post_prefix: Vec<(Role, String)> = messages
        .iter()
        .take(prefix.len())
        .map(|m| (m.role.clone(), m.content.clone()))
        .collect();
    assert_eq!(
        post_prefix, prefix,
        "tool execution itself must not mutate the pre-existing prefix"
    );
    assert_eq!(messages.len(), prefix.len() + 2);
}

// ─── clean-side boundary: no demotion means NOT dirty ───────────────────────

/// #1072 review (coverage finding 2): the latch must stay CLEAN for a run
/// that triggers no demotion. A latch hardcoded to true (or latched on the
/// mere execution of a prune pass) would silently force a full durable
/// rewrite on every turn, defeating the `save_clean_delta` fast path.
#[tokio::test]
async fn under_budget_run_leaves_the_durable_prefix_clean() {
    let mut messages = vec![
        spilled_history_message(1, "a small earlier reply"),
        spilled_history_message(2, "another small earlier reply"),
        Message::user("hi"),
    ];
    let pre_run = messages.clone();
    let agent = agent_with(
        MockProvider::new(vec![text_response("ok")]),
        MockRegistry::new(),
        None,
        190_000,
    );
    let result = agent.run_loop(&mut messages).await.unwrap();

    for (original, live) in pre_run.iter().zip(messages.iter()) {
        assert_eq!(
            original.content, live.content,
            "scenario setup: an under-budget run must not touch history"
        );
    }
    assert!(
        !result.durable_prefix_dirty,
        "a run with no demotion must report the durable prefix CLEAN so \
         persistence keeps the append-only fast path"
    );
    assert!(
        !agent.take_durable_prefix_dirty(),
        "the agent-level latch must also stay clean for an untouched run"
    );
}
