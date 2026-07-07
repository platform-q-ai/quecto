//! #1044/#1045/#1046 agent-loop-level tests: creation-time conversation spill,
//! configurable pin_recent_turns / context_collapse_after_messages threading,
//! window-aware effective budget, and the observable unmet-ceiling signal.
//!
//! Included as a test module from `agent_loop_pruning.rs`; uses the shared
//! mocks from `agent_loop::tests`.

use crate::application::agent_loop::tests::{MockProvider, MockRegistry, text_response};
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::domain::audit::{AuditEvent, AuditSink};
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
    ) -> Pin<Box<dyn Future<Output = Result<(), crate::domain::error::DomainError>> + Send + '_>>
    {
        self.entries.lock().unwrap().clear();
        Box::pin(async { Ok(()) })
    }
}

/// Audit sink capturing every emitted event for assertions.
#[derive(Debug, Default)]
struct CapturingAuditSink {
    events: Mutex<Vec<AuditEvent>>,
}

impl AuditSink for CapturingAuditSink {
    fn emit(
        &self,
        _turn: u32,
        event: AuditEvent,
    ) -> Pin<Box<dyn Future<Output = Result<(), crate::domain::error::DomainError>> + Send + '_>>
    {
        self.events.lock().unwrap().push(event);
        Box::pin(async { Ok(()) })
    }
}

fn agent(
    responses: Vec<crate::domain::message::LlmResponse>,
    spill_store: Arc<MemSpillStore>,
    max_context_tokens: usize,
    audit_log: Option<Arc<dyn AuditSink>>,
) -> AgentLoopImpl {
    AgentLoopImpl::new(AgentLoopConfig {
        provider: Arc::new(MockProvider::new(responses)),
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
        audit_log,
    })
}

// --- #1046 AC1: conversation messages spill at creation, not at drop time ---

#[tokio::test]
async fn assistant_and_user_messages_are_spilled_at_creation() {
    let store = Arc::new(MemSpillStore::default());
    // Huge budget: no drop-time pressure — creation-time spilling only.
    let loop_ = agent(
        vec![text_response("the reply")],
        store.clone(),
        190_000,
        None,
    );
    let mut messages = vec![Message::user("the question")];
    loop_.run_loop(&mut messages).await.unwrap();

    let entries = store.entries.lock().unwrap();
    let assistant = entries
        .iter()
        .find(|e| e.id == "turn1:msg:assistant")
        .expect("the assistant reply must be spilled at creation under turn1:msg:assistant");
    assert_eq!(assistant.content, "the reply");
    assert_eq!(assistant.tool, "assistant");
    assert!(
        entries
            .iter()
            .any(|e| e.tool == "user" && e.content == "the question"),
        "the user prompt must be spilled at creation too; got ids {:?}",
        entries.iter().map(|e| &e.id).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn creation_spill_ids_never_collide_across_prompts() {
    let store = Arc::new(MemSpillStore::default());
    let loop_ = agent(
        vec![text_response("reply A"), text_response("reply B")],
        store.clone(),
        190_000,
        None,
    );
    // Two prompts in one session: turn numbering restarts each run_loop.
    let mut messages = vec![Message::user("prompt A")];
    loop_.run_loop(&mut messages).await.unwrap();
    messages.push(Message::user("prompt B"));
    loop_.run_loop(&mut messages).await.unwrap();

    let entries = store.entries.lock().unwrap();
    let ids: Vec<&str> = entries
        .iter()
        .filter(|e| e.tool == "assistant")
        .map(|e| e.id.as_str())
        .collect();
    assert_eq!(
        ids,
        vec!["turn1:msg:assistant", "turn1:msg:assistant:2"],
        "colliding base ids must be de-duplicated with a :n suffix"
    );
    let second = entries
        .iter()
        .find(|e| e.id == "turn1:msg:assistant:2")
        .unwrap();
    assert_eq!(second.content, "reply B");
}

// --- #1046 AC2/AC5: context_collapse_after_messages threads into the loop ---

#[tokio::test]
async fn message_collapse_knob_threads_through_the_loop() {
    let store = Arc::new(MemSpillStore::default());
    let loop_ = agent(vec![text_response("done")], store.clone(), 190_000, None)
        .with_context_collapse_after_messages(1);
    let mut old_a = Message::assistant("an old answer with plenty of words in it", vec![]);
    old_a.turn = Some(1);
    old_a.spill_id = Some("turn1:msg:assistant".to_string());
    let mut old_b = Message::assistant("another old answer with plenty of words", vec![]);
    old_b.turn = Some(2);
    old_b.spill_id = Some("turn2:msg:assistant".to_string());
    let mut messages = vec![old_a, old_b, Message::user("new prompt")];
    loop_.run_loop(&mut messages).await.unwrap();

    let oldest = messages
        .iter()
        .find(|m| m.role == Role::Assistant && m.turn == Some(1))
        .expect("the oldest assistant message must remain in context as a stub");
    assert!(
        oldest.is_collapsed && oldest.content.contains("recall("),
        "with context_collapse_after_messages=1 the oldest conversation \
         message must be collapsed to a recall stub, got: {}",
        oldest.content
    );
}

// --- #1045: pin_recent_turns is configurable and threads into pruning ---

#[tokio::test]
async fn non_default_pin_recent_turns_changes_pinning() {
    let big = "x".repeat(2000); // ~500 tokens each
    let history = || -> Vec<Message> {
        let mut v: Vec<Message> = (1..=4u32)
            .map(|t| {
                let mut m = Message::assistant(&big, vec![]);
                m.turn = Some(t);
                m
            })
            .collect();
        v.push(Message::user("new prompt"));
        v
    };

    // Default pinning (2): turn 2 is outside the tail and must be pruned away
    // from full context under a 100-token budget.
    let store = Arc::new(MemSpillStore::default());
    let default_loop = agent(vec![text_response("done")], store, 100, None);
    let mut default_messages = history();
    default_loop.run_loop(&mut default_messages).await.unwrap();
    assert!(
        !default_messages
            .iter()
            .any(|m| m.turn == Some(2) && m.content == big),
        "positive control: with the default pin of 2, turn 2 must not survive in full"
    );

    // Non-default pinning (3): turn 2 is inside the tail and must survive in
    // full despite the same impossible budget.
    let store = Arc::new(MemSpillStore::default());
    let wide_loop = agent(vec![text_response("done")], store, 100, None).with_pin_recent_turns(3);
    let mut wide_messages = history();
    wide_loop.run_loop(&mut wide_messages).await.unwrap();
    assert!(
        wide_messages
            .iter()
            .any(|m| m.turn == Some(2) && m.content == big),
        "with pin_recent_turns=3, turn 2 is tail-pinned and must survive in full"
    );
}

// --- #1044 AC1: unmet ceiling is observable in the audit trail ---

#[tokio::test]
async fn unmet_ceiling_is_reflected_in_the_context_pruned_audit_event() {
    let store = Arc::new(MemSpillStore::default());
    let sink = Arc::new(CapturingAuditSink::default());
    // Budget of 5 tokens; the in-flight prompt alone (never droppable) blows
    // it, so the ceiling cannot be met by any amount of demotion.
    let loop_ = agent(
        vec![text_response("done")],
        store,
        5,
        Some(sink.clone() as Arc<dyn AuditSink>),
    );
    let mut messages = vec![Message::user("y".repeat(600))];
    loop_.run_loop(&mut messages).await.unwrap();

    let events = sink.events.lock().unwrap();
    let pruned: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            AuditEvent::ContextPruned { budget_unmet, .. } => Some(*budget_unmet),
            _ => None,
        })
        .collect();
    assert!(
        pruned.iter().any(|unmet| *unmet),
        "an unmeetable ceiling must emit a ContextPruned audit event with \
         budget_unmet=true; ContextPruned events seen: {pruned:?}"
    );
}

// --- #1044 AC2: window-aware effective context budget ---

#[tokio::test]
async fn effective_budget_derives_from_known_model_window() {
    let store = Arc::new(MemSpillStore::default());
    let base = || agent(vec![], store.clone(), 200_000, None);

    // Known window smaller than the config value → the window wins.
    let known = base().with_model_context_window(Some(100_000));
    assert_eq!(
        known.effective_max_context_tokens(),
        100_000,
        "a known smaller model window must clamp the effective budget"
    );

    // Config override: config smaller than a huge window → config wins.
    let overridden = base().with_model_context_window(Some(1_000_000));
    assert_eq!(
        overridden.effective_max_context_tokens(),
        200_000,
        "max_context_tokens stays the override when below the model window"
    );

    // Unknown model → fall back to the configured value.
    let unknown = base().with_model_context_window(None);
    assert_eq!(
        unknown.effective_max_context_tokens(),
        200_000,
        "unknown models fall back to the configured budget"
    );
}

// --- #1044 AC1: the unmet-ceiling warning itself is pinned, not just the flag ---

/// Minimal subscriber capturing the targets of WARN-level events.
struct WarnCapture {
    warn_targets: Arc<Mutex<Vec<String>>>,
}

impl tracing::Subscriber for WarnCapture {
    fn enabled(&self, _metadata: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _span: &tracing::span::Id, _values: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _span: &tracing::span::Id, _follows: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        if *event.metadata().level() == tracing::Level::WARN {
            self.warn_targets
                .lock()
                .unwrap()
                .push(event.metadata().target().to_string());
        }
    }
    fn enter(&self, _span: &tracing::span::Id) {}
    fn exit(&self, _span: &tracing::span::Id) {}
}

/// Run `messages` through a loop with the given budget, capturing WARN targets.
fn warn_targets_for_budget(messages: &mut Vec<Message>, max_context_tokens: usize) -> Vec<String> {
    let warn_targets = Arc::new(Mutex::new(Vec::new()));
    let subscriber = WarnCapture {
        warn_targets: warn_targets.clone(),
    };
    tracing::subscriber::with_default(subscriber, || {
        let store = Arc::new(MemSpillStore::default());
        let loop_ = agent(vec![text_response("done")], store, max_context_tokens, None);
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(loop_.run_loop(messages))
            .unwrap();
    });
    warn_targets.lock().unwrap().clone()
}

#[test]
fn unmet_ceiling_emits_a_context_prune_warning() {
    // Budget of 5 tokens; the in-flight prompt alone (never droppable) blows it.
    let mut messages = vec![Message::user("y".repeat(600))];
    let targets = warn_targets_for_budget(&mut messages, 5);
    assert!(
        targets.iter().any(|t| t == "context_prune"),
        "an unmeetable ceiling must emit tracing::warn!(target: \"context_prune\"); \
         WARN targets seen: {targets:?}"
    );
}

#[test]
fn met_ceiling_emits_no_context_prune_warning() {
    let mut messages = vec![Message::user("a comfortable prompt")];
    let targets = warn_targets_for_budget(&mut messages, 190_000);
    assert!(
        !targets.iter().any(|t| t == "context_prune"),
        "a met budget must not warn; WARN targets seen: {targets:?}"
    );
}

// --- #1044 AC2: the window-derived budget actually drives pruning ---

#[tokio::test]
async fn window_derived_budget_drives_pruning_not_the_larger_config() {
    let store = Arc::new(MemSpillStore::default());
    let big = "x".repeat(2000); // ~500 tokens
    let history = || -> Vec<Message> {
        let mut old = Message::assistant(&big, vec![]);
        old.turn = Some(1);
        vec![old, Message::user("new prompt")]
    };

    // Control: config budget alone (200k) leaves the history untouched.
    let loose =
        agent(vec![text_response("done")], store.clone(), 200_000, None).with_pin_recent_turns(0);
    let mut untouched = history();
    loose.run_loop(&mut untouched).await.unwrap();
    assert!(
        untouched.iter().any(|m| m.content == big),
        "control: without a window clamp nothing should be demoted"
    );

    // Same config, but the model's known window is 100 tokens: the effective
    // budget must reach the ceiling and demote the old message.
    let clamped = agent(vec![text_response("done")], store, 200_000, None)
        .with_pin_recent_turns(0)
        .with_model_context_window(Some(100));
    let mut pruned = history();
    clamped.run_loop(&mut pruned).await.unwrap();
    assert!(
        !pruned.iter().any(|m| m.content == big),
        "the window-derived budget must drive pruning, not the larger config"
    );
}

// --- #1046 fold of #1043: exactly one spill writer, no duplicate entries ---

#[tokio::test]
async fn ladder_dropped_message_is_never_spilled_a_second_time() {
    let store = Arc::new(MemSpillStore::default());
    let big = "x".repeat(2000); // ~500 tokens
    // Budget so tight the ladder must stub AND drop the old assistant turn.
    let loop_ =
        agent(vec![text_response("done")], store.clone(), 20, None).with_pin_recent_turns(0);
    let mut old = Message::assistant(&big, vec![]);
    old.turn = Some(1);
    let mut messages = vec![old, Message::user("q")];
    loop_.run_loop(&mut messages).await.unwrap();

    assert!(
        !messages.iter().any(|m| m.content == big),
        "positive control: the tight budget must remove the old message"
    );
    let entries = store.entries.lock().unwrap();
    let matching: Vec<&str> = entries
        .iter()
        .filter(|e| e.content == big)
        .map(|e| e.id.as_str())
        .collect();
    assert_eq!(
        matching,
        vec!["turn1:msg:assistant"],
        "creation-time spilling is the single writer: a later ladder drop \
         must not file a duplicate entry"
    );
}
