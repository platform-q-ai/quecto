use super::cov_tests::Fixture;
use super::*;
use crate::domain::{message::Message, session::Session};

/// Unit test of `persist_current_session`'s CONSUMER branch only: the dirty
/// flag is hand-set here, so this cannot detect a deleted producer. The
/// end-to-end dirty-propagation coverage (producer side) lives in the #1072
/// tests below — they are the mandatory guard, not this one.
#[tokio::test]
async fn persist_replays_full_history_when_prefix_flagged_dirty() {
    let mut fx = Fixture::new();
    fx.messages = vec![Message::user("old-a"), Message::assistant("old-b", vec![])];
    fx.store
        .save(&Session {
            key: fx.session_key.clone(),
            messages: fx.messages.clone(),
            workflow_run: None,
        })
        .await
        .unwrap();
    fx.last_persisted_message_index = fx.messages.len();

    fx.messages = vec![
        Message::user("pruned-a"),
        Message::assistant("new-c", vec![]),
    ];
    let mut ctx = fx.ctx();
    ctx.durable_prefix_dirty = true;
    persist_current_session(&mut ctx).await.unwrap();

    let resumed = fx.store.load(&fx.session_key).await.unwrap().unwrap();
    let contents: Vec<_> = resumed
        .messages
        .iter()
        .map(|message| message.content.as_str())
        .collect();
    assert_eq!(contents, ["pruned-a", "new-c"]);
}

// ─── #1072: dirty propagation through the production prompt paths ───────────
//
// These tests drive `dispatch_command(Prompt …)` end-to-end and assert on the
// DURABLE OUTCOME: after a turn whose pruning mutated the pre-existing
// history, the persisted session must exactly match the live conversation.
// They fail whenever the dirty-prefix signal is lost anywhere between the
// ladder and `persist_current_session` — in-place stub demotion invisible to
// an id snapshot, an Error/Cancelled outcome dropping the flag, or
// `drain_and_run_pending` discarding the drained run's outcome.

use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::domain::error::DomainError;
use crate::domain::message::LlmResponse;
use crate::domain::provider::{ChatRequest, LlmProvider};
use crate::interface::cli::uds_cancel::fire_cancel;
use std::future::Future;
use std::pin::Pin;

/// Provider that always fails with a terminal (server-class) error.
#[derive(Debug)]
struct FailingProvider;

impl LlmProvider for FailingProvider {
    fn name(&self) -> &str {
        "failing"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn chat(
        &self,
        _request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        Box::pin(async {
            Err(DomainError::Provider(
                "provider error (500): boom".to_string(),
            ))
        })
    }
}

/// Provider that hangs until the run is cancelled.
#[derive(Debug)]
struct HangingProvider;

impl LlmProvider for HangingProvider {
    fn name(&self) -> &str {
        "hanging"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn chat(
        &self,
        _request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        Box::pin(async {
            tokio::time::sleep(std::time::Duration::from_secs(600)).await;
            Err(DomainError::Provider("unreachable".to_string()))
        })
    }
}

fn empty_request<'a>() -> ChatRequest<'a> {
    ChatRequest {
        messages: &[],
        tools: &[],
        model: "stub",
        max_tokens: 1,
        temperature: 0.0,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    }
}

#[tokio::test]
async fn masked_pruning_providers_use_trait_default_stream_surface() {
    let failing = FailingProvider;
    assert!(failing.as_any().downcast_ref::<FailingProvider>().is_some());
    assert!(
        failing
            .chat_stream(empty_request())
            .await
            .unwrap_err()
            .to_string()
            .contains("boom")
    );
    let mut rx = failing.chat_stream_incremental(empty_request()).await;
    assert!(
        matches!(rx.recv().await, Some(crate::domain::provider::StreamEvent::Error(e)) if e.contains("boom"))
    );

    let hanging = HangingProvider;
    assert!(hanging.as_any().downcast_ref::<HangingProvider>().is_some());
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(20),
            hanging.chat_stream(empty_request())
        )
        .await
        .is_err(),
        "hanging provider should still be sleeping"
    );
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(20),
            hanging.chat_stream_incremental(empty_request())
        )
        .await
        .is_err(),
        "default incremental awaits the hanging chat_stream before returning"
    );
}

fn budgeted_agent(
    provider: std::sync::Arc<dyn LlmProvider>,
    max_context_tokens: usize,
) -> AgentLoopImpl {
    AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(crate::infrastructure::tools::registry::ToolRegistryImpl::new()),
        model: "stub".into(),
        max_tokens: 100,
        temperature: 0.0,
        spill_store: None,
        session_key: "cli:test".into(),
        context_collapse_after_tool_calls: u32::MAX,
        max_context_tokens,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
    })
}

/// A spilled assistant message from an earlier prompt (`spill_id` set so
/// ladder rung 1 may stub it in place without any spill store).
fn spilled_history_message(turn: u32, content: &str) -> Message {
    let mut msg = Message::assistant(content, vec![]);
    msg.turn = Some(turn);
    msg.spill_id = Some(format!("turn{turn}:msg:assistant"));
    msg
}

/// History whose turns 1–2 exceed a 300-token budget while turns 3–4 are
/// small and tail-pinned: the ladder stubs 1–2 IN PLACE and drops nothing.
fn stub_demotable_history(big_chars: usize) -> Vec<Message> {
    let big = "z".repeat(big_chars);
    vec![
        spilled_history_message(1, &big),
        spilled_history_message(2, &big),
        spilled_history_message(3, "small earlier reply"),
        spilled_history_message(4, "another small earlier reply"),
    ]
}

async fn persist_baseline(fx: &mut Fixture) {
    fx.store
        .save(&Session {
            key: fx.session_key.clone(),
            messages: fx.messages.clone(),
            workflow_run: None,
        })
        .await
        .unwrap();
    fx.last_persisted_message_index = fx.messages.len();
}

async fn assert_durable_matches_live(fx: &Fixture) {
    let resumed = fx.store.load(&fx.session_key).await.unwrap().unwrap();
    let durable: Vec<&str> = resumed
        .messages
        .iter()
        .map(|message| message.content.as_str())
        .collect();
    let live: Vec<&str> = fx
        .messages
        .iter()
        .map(|message| message.content.as_str())
        .collect();
    assert_eq!(
        durable, live,
        "the durable session must exactly match the live (pruned) history — \
         a stale durable prefix would resurrect demoted content on resume"
    );
}

fn prompt(message: &str) -> AgentCommand {
    AgentCommand::Prompt {
        id: None,
        message: message.into(),
        streaming_behavior: None,
    }
}

/// RED (#1072 addendum finding 6): a successful turn whose only demotion is
/// IN-PLACE stub collapse (message ids unchanged) must still reconcile
/// persistence.
#[tokio::test]
async fn success_with_stub_only_demotion_reconciles_persistence() {
    let mut fx = Fixture::new();
    fx.agent = budgeted_agent(crate::interface::test_support::make_stub_provider(), 300);
    fx.messages = stub_demotable_history(2400);
    persist_baseline(&mut fx).await;

    {
        let mut ctx = fx.ctx();
        assert!(!dispatch_command(prompt("hi"), &mut ctx).await);
    }

    // Positive control: the turn stub-demoted turn 1 in place, dropped nothing.
    assert!(
        fx.messages[0].is_collapsed && fx.messages[0].content.contains("recall("),
        "scenario setup: turn 1 must be stubbed in place, got: {}",
        fx.messages[0].content
    );
    for spill_id in ["turn1:msg:assistant", "turn2:msg:assistant"] {
        assert!(
            fx.messages
                .iter()
                .any(|m| m.is_collapsed && m.content.contains(spill_id)),
            "scenario setup: the {spill_id} stub must survive in place"
        );
    }
    for small in ["small earlier reply", "another small earlier reply"] {
        assert!(
            fx.messages.iter().any(|m| m.content == small),
            "scenario setup: {small:?} must survive untouched (stub-only)"
        );
    }
    assert_durable_matches_live(&fx).await;
}

/// RED (#1072 re-review finding 2): the dirty signal must survive a turn
/// that ends in a provider ERROR — pruning already mutated history before
/// the failure, and persistence still runs afterwards.
#[tokio::test]
async fn error_outcome_still_reconciles_persistence_after_demotion() {
    let mut fx = Fixture::new();
    fx.agent = budgeted_agent(std::sync::Arc::new(FailingProvider), 300);
    fx.messages = stub_demotable_history(2400);
    persist_baseline(&mut fx).await;

    {
        let mut ctx = fx.ctx();
        assert!(!dispatch_command(prompt("hi"), &mut ctx).await);
    }

    assert!(
        fx.messages[0].is_collapsed,
        "scenario setup: pruning must have stub-demoted turn 1 before the \
         provider error"
    );
    assert_durable_matches_live(&fx).await;
}

/// RED (#1072 re-review finding 2 / addendum finding 7): the dirty signal
/// must survive a CANCELLED turn. Pruning runs before the provider call; the
/// cancel rolls back only the prompt, and persistence still runs afterwards
/// with the stubbed history at exactly the old watermark length (masked).
#[tokio::test]
async fn cancelled_outcome_still_reconciles_persistence_after_demotion() {
    let mut fx = Fixture::new();
    fx.agent = budgeted_agent(std::sync::Arc::new(HangingProvider), 300);
    fx.messages = stub_demotable_history(2400);
    persist_baseline(&mut fx).await;

    let cancel = fx.cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        fire_cancel(&cancel);
    });
    {
        let mut ctx = fx.ctx();
        assert!(!dispatch_command(prompt("hi"), &mut ctx).await);
    }

    assert!(
        fx.messages[0].is_collapsed,
        "scenario setup: pruning must have stub-demoted turn 1 before the hang"
    );
    assert_eq!(
        fx.messages.len(),
        4,
        "scenario setup: rollback leaves exactly the four pre-run messages — \
         the masked shape (length equals the old watermark)"
    );
    assert_durable_matches_live(&fx).await;
}

/// A big assistant message from an earlier prompt WITHOUT a `spill_id`:
/// ladder rung 1 cannot stub it, so rung 2 physically DROPS it — the shape
/// where the cancelled prompt's pre-turn index goes stale.
fn droppable_history_message(turn: u32, big_chars: usize) -> Message {
    let mut msg = Message::assistant("z".repeat(big_chars), vec![]);
    msg.turn = Some(turn);
    msg
}

/// #1072 review (coverage finding 1): cancel AFTER pruning physically removed
/// earlier entries, through the production dispatch path. The prompt's
/// pre-turn index is stale; a revert to `messages.truncate(user_msg_idx)`
/// becomes a no-op (index > len) and silently retains — and persists — the
/// cancelled prompt. Only id-based rollback removes it.
#[tokio::test]
async fn cancellation_after_physical_drops_removes_the_cancelled_prompt() {
    let mut fx = Fixture::new();
    fx.agent = budgeted_agent(std::sync::Arc::new(HangingProvider), 700);
    // 8 droppable oversized turns: rung 2 removes most of them on the first
    // prune, before the provider hang — a pure physical shrink.
    fx.messages = (1..=8)
        .map(|t| droppable_history_message(t, 2400))
        .collect();
    persist_baseline(&mut fx).await;

    let cancel = fx.cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        fire_cancel(&cancel);
    });
    {
        let mut ctx = fx.ctx();
        assert!(!dispatch_command(prompt("hi"), &mut ctx).await);
    }

    assert!(
        fx.messages.len() < 8,
        "scenario setup: pruning must have physically dropped pre-run \
         entries before the hang (got {} messages)",
        fx.messages.len()
    );
    assert!(
        !fx.messages.iter().any(|m| m.content == "hi"),
        "the cancelled prompt must be rolled back at its LOGICAL boundary — \
         a stale positional truncate retains it after physical drops"
    );
    assert!(
        fx.messages
            .iter()
            .all(|m| m.content.starts_with('z') && m.turn.is_some()),
        "only pre-existing survivors may remain after rollback"
    );
    assert_durable_matches_live(&fx).await;
}

/// RED (#1072 re-review finding 1): `drain_and_run_pending` must propagate
/// the drained run's dirty flag. The initial prompt fits the budget; the
/// queued (drained) follow-up pushes the total over it, so demotion happens
/// ONLY inside the drained run — whose PromptOutcome the pre-fix code
/// discards.
#[tokio::test]
async fn drained_pending_run_propagates_prefix_dirty_to_persistence() {
    let original_first = "z".repeat(1200);

    // Control (#1072 review, falsifiability finding 4): an identical fixture
    // WITHOUT the queued follow-up must not demote anything — proving the
    // demotion asserted below happens in the DRAINED run, not the initial
    // prompt run (whose dirty flag is propagated by a different code path).
    {
        let mut control = Fixture::new();
        control.agent = budgeted_agent(crate::interface::test_support::make_stub_provider(), 800);
        control.messages = stub_demotable_history(1200);
        persist_baseline(&mut control).await;
        {
            let mut ctx = control.ctx();
            assert!(!dispatch_command(prompt("hi"), &mut ctx).await);
        }
        assert!(
            control
                .messages
                .first()
                .is_some_and(|m| m.content == original_first),
            "control: the initial prompt run alone must NOT demote the \
             pre-run history — the budget only overflows with the drained \
             follow-up. If this fails, the scenario no longer isolates \
             drain_and_run_pending and its budgets need re-tuning."
        );
    }

    let mut fx = Fixture::new();
    // Budget 800: the 4-message history (~615 tokens) plus a tiny prompt fits;
    // the moderate drained follow-up (~450 tokens) pushes the total over the
    // budget, so rung 1 stubs turn 1 IN PLACE during the drained run only —
    // the final length stays at/above the old watermark (the masked shape).
    fx.agent = budgeted_agent(crate::interface::test_support::make_stub_provider(), 800);
    fx.messages = stub_demotable_history(1200);
    persist_baseline(&mut fx).await;
    fx.session.enqueue_pending("f".repeat(1800));

    {
        let mut ctx = fx.ctx();
        assert!(!dispatch_command(prompt("hi"), &mut ctx).await);
    }

    assert!(
        fx.messages
            .first()
            .is_some_and(|m| m.content != original_first),
        "scenario setup: the drained run must have demoted (stubbed or \
         dropped) the pre-run history"
    );
    assert_durable_matches_live(&fx).await;
}

// ─── dispatch_command routing ────────────────────────────────────────────────
