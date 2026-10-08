//! #1044/#1045/#1046 agent-loop-level tests: creation-time conversation spill,
//! configurable pin_recent_turns / watermark marks threading,
//! window-aware effective budget, and the observable unmet-ceiling signal.
//!
//! Included as a test module from `agent_loop_pruning.rs`; uses the shared
//! mocks from `agent_loop::tests`.

use crate::application::agent_loop::tests::{MockProvider, MockRegistry, text_response};
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::application::audit::ports::AuditSink;
use crate::application::sessions::ports::ContextSpillStore;
use crate::domain::audit::AuditEvent;
use crate::domain::conversation::value_objects::message::Message;
use crate::domain::sessions::entities::session::SpillEntry;
use crate::domain::sessions::entities::session_identity::{SessionIdentity, SpillId};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

fn id(k: &str) -> SessionIdentity {
    SessionIdentity::from_persisted_key(k)
}

#[derive(Debug, Default)]
pub(super) struct MemSpillStore {
    entries: Mutex<Vec<SpillEntry>>,
}

impl ContextSpillStore for MemSpillStore {
    fn append(
        &self,
        _session_key: &SessionIdentity,
        entry: &SpillEntry,
    ) -> Pin<Box<dyn Future<Output = Result<(), crate::domain::error::DomainError>> + Send + '_>>
    {
        self.entries.lock().unwrap().push(entry.clone());
        Box::pin(async { Ok(()) })
    }

    fn recall(
        &self,
        _session_key: &SessionIdentity,
        id: &SpillId,
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
            .find(|e| e.id == id.as_str())
            .cloned();
        Box::pin(async move { Ok(found) })
    }

    fn list_entries(
        &self,
        _session_key: &SessionIdentity,
    ) -> crate::application::sessions::ports::SpillIndexList<'_> {
        let index: Vec<crate::domain::sessions::entities::session::SpillIndex> = self
            .entries
            .lock()
            .unwrap()
            .iter()
            .map(|e| crate::domain::sessions::entities::session::SpillIndex {
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
        _session_key: &SessionIdentity,
    ) -> Pin<Box<dyn Future<Output = Result<(), crate::domain::error::DomainError>> + Send + '_>>
    {
        self.entries.lock().unwrap().clear();
        Box::pin(async { Ok(()) })
    }
}

/// Audit sink capturing every emitted event for assertions.
#[derive(Debug, Default)]
pub(in crate::application::agent_loop) struct CapturingAuditSink {
    pub(in crate::application::agent_loop) events: Mutex<Vec<AuditEvent>>,
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
    responses: Vec<crate::domain::conversation::value_objects::message::LlmResponse>,
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
        retention: Some(crate::composition::retention::context_retention_over(
            spill_store,
        )),
        session_key: "test-session".to_string(),
        max_context_tokens,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log,
        pin_recent_turns: 2,
        context_marks: Default::default(),
        model_context_window: None,
        tool_profile_context:
            crate::domain::tool_policy::value_objects::tool::ToolProfileContext::Parent,
    })
}

// --- #1046 AC1: conversation messages spill at creation, not at drop time ---

#[tokio::test]
async fn assistant_and_user_messages_are_spilled_at_creation() {
    let store = Arc::new(MemSpillStore::default());
    // Huge budget: no drop-time pressure — creation-time spilling only.
    let mut loop_ = agent(
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
    let mut loop_ = agent(
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

// --- #1044 AC1: unmet ceiling is observable in the audit trail ---

#[tokio::test]
async fn unmet_ceiling_is_reflected_in_the_context_pruned_audit_event() {
    let store = Arc::new(MemSpillStore::default());
    let sink = Arc::new(CapturingAuditSink::default());
    // Budget of 5 tokens; the in-flight prompt alone (never droppable) blows
    // it, so the ceiling cannot be met by any amount of demotion.
    let mut loop_ = agent(
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

    // Known window smaller than the config value → the window, less the
    // 1024-token reply reserve (#2405), wins.
    let known = base().with_model_context_window(Some(100_000));
    assert_eq!(
        known.effective_max_context_tokens(),
        100_000 - 1_024,
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

#[tokio::test]
async fn set_model_rederives_the_context_window_budget() {
    // A runtime model switch must carry the new model's window so the pruning
    // budget never goes stale (PR #1048 review): small-window → large-window
    // stops over-pruning, and large → small re-clamps before overflow.
    let store = Arc::new(MemSpillStore::default());
    let mut agent = agent(vec![], store, 300_000, None).with_model_context_window(Some(32_768));
    // #2405: the window less the 1024-token reply reserve.
    assert_eq!(agent.effective_max_context_tokens(), 32_768 - 1_024);

    crate::application::catalogue::ports::ModelRuntime::apply_model(
        &mut agent,
        "big/model".into(),
        crate::application::catalogue::dto::ModelLimits {
            max_output_tokens: None,
            context_window: Some(1_000_000),
            prompt_limit: Default::default(),
            image_input: Default::default(),
        },
    );
    assert_eq!(
        agent.effective_max_context_tokens(),
        300_000,
        "switching to a large-window model must lift the stale 32k clamp"
    );

    crate::application::catalogue::ports::ModelRuntime::apply_model(
        &mut agent,
        "small/model".into(),
        crate::application::catalogue::dto::ModelLimits {
            max_output_tokens: None,
            context_window: Some(32_768),
            prompt_limit: Default::default(),
            image_input: Default::default(),
        },
    );
    assert_eq!(
        agent.effective_max_context_tokens(),
        32_768 - 1_024,
        "switching to a small-window model must re-clamp the budget"
    );

    crate::application::catalogue::ports::ModelRuntime::apply_model(
        &mut agent,
        "unknown/model".into(),
        crate::application::catalogue::dto::ModelLimits {
            max_output_tokens: None,
            context_window: None,
            prompt_limit: Default::default(),
            image_input: Default::default(),
        },
    );
    assert_eq!(
        agent.effective_max_context_tokens(),
        300_000,
        "an unknown window must fall back to the configured budget"
    );
}

#[tokio::test]
async fn reported_max_context_tokens_matches_the_enforced_budget() {
    // Stats/snapshot consumers read max_context_tokens(); it must report the
    // same window-aware value pruning enforces (PR #1048 review).
    let store = Arc::new(MemSpillStore::default());
    let agent = agent(vec![], store, 300_000, None).with_model_context_window(Some(32_768));
    assert_eq!(
        agent.max_context_tokens(),
        agent.effective_max_context_tokens(),
        "the reported budget must never diverge from the enforced one"
    );
    assert_eq!(agent.max_context_tokens(), 32_768 - 1_024);
}

// --- #1044 AC1: a met ceiling records no over-budget prune ---

/// The over-budget signal is observed through the deterministic
/// `ContextPruned { budget_unmet }` audit event rather than a captured
/// `tracing::warn!` — the warn and the audit event are emitted from the same
/// `over_budget` condition, and a captured warn depends on the process-global
/// tracing interest cache, which a parallel sibling test can poison to "never"
/// (races even a `rebuild_interest_cache`), making the assertion flaky (#1053).
/// The unmet case is pinned by
/// `unmet_ceiling_is_reflected_in_the_context_pruned_audit_event`.
#[tokio::test]
async fn met_ceiling_records_no_over_budget_prune() {
    let store = Arc::new(MemSpillStore::default());
    let sink = Arc::new(CapturingAuditSink::default());
    let mut loop_ = agent(
        vec![text_response("done")],
        store,
        190_000,
        Some(sink.clone() as Arc<dyn AuditSink>),
    );
    let mut messages = vec![Message::user("a comfortable prompt")];
    loop_.run_loop(&mut messages).await.unwrap();

    let events = sink.events.lock().unwrap();
    let over_budget = events.iter().any(|e| {
        matches!(
            e,
            AuditEvent::ContextPruned {
                budget_unmet: true,
                ..
            }
        )
    });
    assert!(
        !over_budget,
        "a met budget must not record a ContextPruned event with budget_unmet=true; \
         events seen: {:?}",
        *events
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
    let mut loose =
        agent(vec![text_response("done")], store.clone(), 200_000, None).with_pin_recent_turns(0);
    let mut untouched = history();
    loose.run_loop(&mut untouched).await.unwrap();
    assert!(
        untouched.iter().any(|m| m.content == big),
        "control: without a window clamp nothing should be demoted"
    );

    // Same config, but the model's known window is 100 tokens: the effective
    // budget must reach the ceiling and demote the old message.
    let mut clamped = agent(vec![text_response("done")], store, 200_000, None)
        .with_pin_recent_turns(0)
        .with_model_context_window(Some(100));
    let mut pruned = history();
    clamped.run_loop(&mut pruned).await.unwrap();
    assert!(
        !pruned.iter().any(|m| m.content == big),
        "the window-derived budget must drive pruning, not the larger config"
    );
}

// --- #2214: a prune that only stubbed says so in the audit trail ---
// #2414: the ladder runs only when no cut fits: here the brief, which every
// cut keeps, is alone over the ceiling.

#[tokio::test]
async fn a_prune_that_stubbed_a_message_records_it_in_the_context_pruned_event() {
    let store = Arc::new(MemSpillStore::default());
    let sink = Arc::new(CapturingAuditSink::default());
    let big = "x".repeat(2000); // ~500 tokens
    let brief = crate::domain::conversation::services::turn_origin::prompt(big);
    let brief_id = brief.id();
    let mut messages = vec![brief, Message::user("new prompt")];
    let mut loop_ = agent(
        vec![text_response("done")],
        store,
        200_000,
        Some(sink.clone() as Arc<dyn AuditSink>),
    )
    .with_pin_recent_turns(0)
    .with_model_context_window(Some(140));
    loop_.run_loop(&mut messages).await.unwrap();

    assert!(
        messages
            .iter()
            .any(|m| m.id() == brief_id && m.content.contains("recall(")),
        "positive control: the brief must be stubbed, not dropped"
    );
    let events = sink.events.lock().unwrap();
    let stubbed: Vec<(usize, usize)> = events
        .iter()
        .filter_map(|e| match e {
            AuditEvent::ContextPruned {
                ladder_stubbed,
                messages_dropped,
                ..
            } => Some((*ladder_stubbed, *messages_dropped)),
            _ => None,
        })
        .collect();
    assert_eq!(
        stubbed.first(),
        Some(&(1, 0)),
        "the first prune stubbed one message and dropped none: {stubbed:?}"
    );
}

// --- #1046 fold of #1043: exactly one spill writer, no duplicate entries ---

#[tokio::test]
async fn ladder_dropped_message_is_never_spilled_a_second_time() {
    let store = Arc::new(MemSpillStore::default());
    let big = "x".repeat(2000); // ~500 tokens
    // Budget so tight the ladder must stub AND drop the old assistant turn.
    let mut loop_ =
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

// --- PR #1048 follow-up: context knobs are AgentLoopConfig constructor fields ---

#[test]
fn agent_loop_config_carries_context_knobs_as_constructor_fields() {
    // pin_recent_turns, the watermark marks and model_context_window must
    // be constructor fields on AgentLoopConfig — same altitude as
    // max_context_tokens — so a construction site cannot omit them (no post-construction builder
    // patching required for correctness).
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: Arc::new(MockProvider::new(vec![text_response("hi")])),
        tool_registry: Box::new(MockRegistry::new()),
        model: "test-model".into(),
        max_tokens: 1024,
        temperature: 0.0,
        retention: None,
        session_key: "s".into(),
        max_context_tokens: 100_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 5,
        context_marks: crate::domain::conversation::services::watermark::Watermark::new(
            40_000, 12_000,
        )
        .unwrap(),
        model_context_window: Some(48_000),
        tool_profile_context:
            crate::domain::tool_policy::value_objects::tool::ToolProfileContext::Parent,
    });
    assert_eq!(
        agent.context_knob_snapshot(),
        (
            5,
            crate::domain::conversation::services::watermark::Watermark::new(40_000, 12_000)
                .unwrap()
        )
    );
    assert_eq!(agent.model_context_window, Some(48_000));
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
        images: Vec::new(),
    };
    store.append(&id("s"), &entry).await.unwrap();
    assert_eq!(
        store
            .recall(&id("s"), &SpillId::new("id1"))
            .await
            .unwrap()
            .unwrap()
            .content,
        "out"
    );
    store.clear(&id("s")).await.unwrap();
    assert!(
        store
            .recall(&id("s"), &SpillId::new("id1"))
            .await
            .unwrap()
            .is_none()
    );
}

/// #2160: the context estimate includes the tool definitions every request
/// carries.
#[tokio::test]
async fn the_context_estimate_includes_the_tool_definitions() {
    let (bare, _) = crate::application::agent_loop::tests::make_agent(vec![], vec![]);
    let (tooled, _) = crate::application::agent_loop::tests::make_agent(
        vec![],
        vec![("lookup", "ok"), ("fetch", "ok")],
    );
    let conversation = || vec![Message::user("hi")];
    let without = bare
        .apply_context_pruning(&mut conversation(), 1, false)
        .await;
    let with = tooled
        .apply_context_pruning(&mut conversation(), 1, false)
        .await;
    let tools = tooled
        .current_tool_definitions()
        .iter()
        .map(crate::domain::tool_policy::value_objects::tool::ToolDefinition::estimated_tokens)
        .sum::<usize>();
    assert!(tools > 0);
    assert_eq!(with, without + tools);
}

/// #2160: the tool definitions every request carries count against the
/// budget: a conversation that fits without them loses its oldest turn
/// with them.
#[tokio::test]
async fn the_tool_definitions_count_against_the_budget() {
    use crate::application::agent_loop::tests::{
        MockProvider, MockRegistry, MockTool, test_config,
    };
    let conversation = || {
        let turn = |n: u32| {
            let mut message = Message::assistant("x".repeat(2_000), vec![]);
            message.turn = Some(n);
            message
        };
        vec![turn(1), turn(2), turn(3), turn(4), Message::user("now")]
    };
    let messages_total =
        crate::application::context_pruning::estimate_total_tokens(&conversation());
    let agent_with = |tools: &[&str]| {
        let mut registry = MockRegistry::new();
        for name in tools {
            registry.register(std::sync::Arc::new(MockTool::new(name, "ok")));
        }
        let tool_tokens = registry
            .cached_definitions
            .iter()
            .map(crate::domain::tool_policy::value_objects::tool::ToolDefinition::estimated_tokens)
            .sum::<usize>();
        let provider = std::sync::Arc::new(MockProvider::new(vec![]));
        let agent = AgentLoopImpl::new(AgentLoopConfig {
            max_context_tokens: messages_total + 5,
            ..test_config(provider, Box::new(registry))
        });
        (agent, tool_tokens)
    };
    let (bare, _) = agent_with(&[]);
    let mut kept = conversation();
    bare.apply_context_pruning(&mut kept, 1, false).await;
    assert!(kept.iter().any(|m| m.turn == Some(1)), "everything fits");

    let (tooled, tool_tokens) = agent_with(&["lookup", "fetch", "search", "inspect"]);
    assert!(tool_tokens > 5, "{tool_tokens}");
    let mut pruned = conversation();
    tooled.apply_context_pruning(&mut pruned, 1, false).await;
    assert!(
        !pruned.iter().any(|m| m.turn == Some(1)),
        "the oldest turn makes room for the tool definitions"
    );
}

/// #2182 review: tool definitions larger than the budget leave the messages
/// a quarter of it, never nothing.
#[tokio::test]
async fn oversized_tool_definitions_leave_the_messages_a_quarter() {
    use crate::application::agent_loop::tests::{
        MockProvider, MockRegistry, MockTool, test_config,
    };
    let mut registry = MockRegistry::new();
    for i in 0..40 {
        registry.register(std::sync::Arc::new(MockTool::new(
            &format!("tool_{i}"),
            "ok",
        )));
    }
    let tool_tokens: usize = registry
        .cached_definitions
        .iter()
        .map(crate::domain::tool_policy::value_objects::tool::ToolDefinition::estimated_tokens)
        .sum();
    let provider = std::sync::Arc::new(MockProvider::new(vec![]));
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        max_context_tokens: tool_tokens / 2,
        ..test_config(provider, Box::new(registry))
    });
    let turn = |n: u32| {
        let mut message = Message::assistant("ok", vec![]);
        message.turn = Some(n);
        message
    };
    let mut messages = vec![turn(1), turn(2), turn(3), turn(4), Message::user("hi")];
    agent.apply_context_pruning(&mut messages, 1, false).await;
    assert!(
        messages.iter().any(|m| m.turn == Some(1)),
        "short history fits the quarter left to the messages"
    );
}
