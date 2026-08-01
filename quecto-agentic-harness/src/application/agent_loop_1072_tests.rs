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
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
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

    drop(result);
    assert!(
        agent.take_durable_prefix_dirty(),
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
        agent.take_durable_prefix_dirty(),
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

/// Unit contract of `execute_tool_calls_for_response`: each message it
/// appends is recorded in the run ledger AT the moment of the append, and the
/// pre-existing prefix is left untouched. NOTE (#1073 review): no pruning can
/// run inside this function (only `run_loop` prunes), so this test cannot —
/// and does not claim to — falsify ladder behavior or a positional-slice
/// regression in `run_loop`; that end-to-end coverage lives in
/// `mid_run_shrink_below_pre_turn_length_reports_exact_ledger_and_dirty`
/// above. What this pins: the ledger parameter is a live write (not dead),
/// its entries are exact as-appended clones, and tool execution never
/// mutates messages it did not append.
#[tokio::test]
async fn tool_execution_records_its_appends_in_the_run_ledger_at_append_time() {
    let big = big_content();
    let mut registry = MockRegistry::new();
    registry.register(Arc::new(MockTool::new("bulk", "tool-output-payload")));
    let agent = agent_with(
        MockProvider::new(vec![]),
        registry,
        Some(Arc::new(MemSpillStore::default())),
        190_000,
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

// ─── manifest structural changes must latch dirty (#1073 review) ────────────

/// CONFIRMED #1073 finding: `update_spill_manifest` can INSERT a manifest at
/// the front of the persisted prefix (e.g. the first prompt after `rewind_to`
/// cleared spill references), shifting every persisted index right — with no
/// collapse/stub/drop to fire the latch. Persistence would then append a
/// clean delta starting at the (shifted) watermark, duplicating the last
/// already-persisted message in the durable file. A manifest insert/removal
/// must latch dirty on its own.
#[tokio::test]
async fn manifest_insertion_into_the_persisted_prefix_latches_dirty() {
    // Small resumed history, far under budget: no collapse, no stub, no drop.
    // The messages carry no spill_id (rewind_to strips them), so the
    // creation-time spill files them and a fresh manifest is inserted at the
    // front of the conversation.
    let mut messages = vec![
        Message::user("persisted one"),
        Message::assistant("persisted two", vec![]),
        Message::user("go"),
    ];
    let agent = agent_with(
        MockProvider::new(vec![text_response("ok")]),
        MockRegistry::new(),
        Some(Arc::new(MemSpillStore::default())),
        190_000,
    );
    let _ = agent.run_loop(&mut messages).await.unwrap();

    // Positive control: a manifest really was inserted before the prefix.
    assert!(
        messages.first().is_some_and(|m| m.is_manifest),
        "scenario setup: the spill pass must insert a manifest message"
    );
    assert!(
        agent.take_durable_prefix_dirty(),
        "a manifest insert shifts every persisted index and MUST latch \
         durable_prefix_dirty, or the clean-delta fast path appends against \
         a shifted prefix and duplicates a persisted message"
    );
}

/// Clean-side companion: once the manifest already contains static guidance,
/// later spill growth changes no prefix bytes, so the latch must stay clean or
/// `save_clean_delta` would be defeated on virtually every turn.
#[tokio::test]
async fn unchanged_static_manifest_does_not_latch_dirty() {
    let store: Arc<dyn crate::domain::session::ContextSpillStore> =
        Arc::new(MemSpillStore::default());
    let agent = agent_with(
        MockProvider::new(vec![text_response("first"), text_response("second")]),
        MockRegistry::new(),
        Some(store),
        190_000,
    );
    // First run inserts the manifest (latches — drain it).
    let mut messages = vec![Message::user("go")];
    let _ = agent.run_loop(&mut messages).await.unwrap();
    assert!(agent.take_durable_prefix_dirty());
    assert!(messages.iter().any(|m| m.is_manifest));

    // Second run: manifest exists and stays byte-identical while history grows.
    messages.push(Message::user("again"));
    let _ = agent.run_loop(&mut messages).await.unwrap();
    assert!(
        !agent.take_durable_prefix_dirty(),
        "an unchanged static manifest must NOT latch dirty — latching here \
         would force a full compact rewrite on virtually every turn"
    );
}

#[tokio::test]
async fn legacy_dynamic_manifest_migration_latches_dirty() {
    let store = Arc::new(MemSpillStore::default());
    store
        .append(
            "",
            &SpillEntry {
                id: "turn1:bash:0".into(),
                tool: "bash".into(),
                input_preview: "legacy preview".into(),
                tokens: 10,
                content: "legacy content".into(),
            },
        )
        .await
        .unwrap();
    let agent = agent_with(
        MockProvider::new(vec![text_response("ok")]),
        MockRegistry::new(),
        Some(store),
        190_000,
    );
    let mut legacy =
        Message::system("[Session memory: 1 spilled entries via recall()]\nLatest: turn1:bash:0");
    legacy.is_manifest = true;
    legacy.is_pinned = true;
    let mut messages = vec![legacy, Message::user("resume")];

    let _ = agent.run_loop(&mut messages).await.unwrap();

    let manifest = messages.iter().find(|message| message.is_manifest).unwrap();
    assert!(!manifest.content.contains("turn1:bash:0"));
    assert!(manifest.content.contains("recall(\"list\")"));
    assert!(
        agent.take_durable_prefix_dirty(),
        "migrating persisted dynamic manifest bytes requires a durable rewrite"
    );
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
    drop(result);
    assert!(
        !agent.take_durable_prefix_dirty(),
        "a run with no demotion must leave the agent latch CLEAN so \
         persistence keeps the append-only fast path"
    );
}

#[tokio::test]
async fn mem_spill_store_trait_surface_recalls_and_clears() {
    let store = MemSpillStore::default();
    let entry = SpillEntry {
        id: "id1".into(),
        tool: "bash".into(),
        input_preview: "echo".into(),
        tokens: 2,
        content: "out".into(),
    };
    store.append("s", &entry).await.unwrap();
    assert_eq!(
        store.recall("s", "id1").await.unwrap().unwrap().content,
        "out"
    );
    store.clear("s").await.unwrap();
    assert!(store.recall("s", "id1").await.unwrap().is_none());
}
