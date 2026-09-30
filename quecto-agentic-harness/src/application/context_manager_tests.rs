use super::*;
use crate::application::context_pruning;
use crate::application::sessions::ports::ContextSpillStore;
use crate::domain::message::Message;
use crate::domain::session::{SpillEntry, SpillIndex};
use crate::domain::session_identity::{SessionIdentity, SpillId};
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
        let index: Vec<SpillIndex> = self
            .entries
            .lock()
            .unwrap()
            .iter()
            .map(|e| SpillIndex {
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

fn manager(max_context_tokens: usize) -> ContextManager {
    ContextManager::new(config_for(max_context_tokens))
}

fn config_for(max_context_tokens: usize) -> ContextManagerConfig {
    ContextManagerConfig {
        retention: Some(crate::composition::retention::context_retention_over(
            Arc::new(MemSpillStore::default()),
        )),
        session_key: SessionIdentity::from_persisted_key("test-session"),
        context_collapse_after_tool_calls: context_pruning::COLLAPSE_DISABLED,
        max_context_tokens,
        pin_recent_turns: 2,
        context_collapse_after_messages: context_pruning::COLLAPSE_DISABLED,
        large_result_collapse: crate::domain::large_result_collapse::LargeResultCollapse::DISABLED,
        model_context_window: None,
    }
}

fn long_message(turn: u32) -> Message {
    let mut msg = Message::assistant("x".repeat(2_000), vec![]);
    msg.turn = Some(turn);
    msg.spill_id = Some(format!("turn{turn}:msg:assistant"));
    msg
}

#[tokio::test]
async fn context_manager_plan_preserves_pinned_recent_turns() {
    let manager = manager(10);
    let mut messages = vec![
        long_message(1),
        long_message(2),
        long_message(3),
        long_message(4),
        Message::user("current prompt"),
    ];

    let plan = manager
        .prepare_provider_context(&mut messages, manager.effective_max_context_tokens(), false)
        .await;

    assert!(
        messages.iter().any(|m| m.turn == Some(3)),
        "turn 3 should be pinned as one of the two most recent completed turns"
    );
    assert!(
        messages.iter().any(|m| m.turn == Some(4)),
        "turn 4 should be pinned as one of the two most recent completed turns"
    );
    assert!(
        !messages.iter().any(|m| matches!(m.turn, Some(1 | 2))),
        "older turns should be dropped through the context-manager plan"
    );
    assert!(
        plan.durable_prefix_dirty,
        "dropping older persisted messages must request durable prefix reconciliation"
    );
}

#[tokio::test]
async fn context_manager_marks_dirty_when_manifest_layout_shifts() {
    let manager = manager(190_000);
    let mut manifest = Message::system(context_pruning::build_manifest_text());
    manifest.is_pinned = true;
    manifest.is_manifest = true;
    let mut messages = vec![manifest];

    let plan = manager
        .prepare_provider_context(&mut messages, manager.effective_max_context_tokens(), true)
        .await;

    assert!(
        messages.iter().all(|m| !m.is_manifest),
        "empty spill store should remove the persisted manifest"
    );
    assert!(
        plan.durable_prefix_dirty,
        "manifest insertion/removal shifts persisted positions and must be dirty"
    );
}

#[test]
fn context_manager_reconciles_provider_truth_across_local_estimate_changes() {
    let manager = manager(190_000);

    manager.observe_provider_context_gauge(1_000, 100);

    assert_eq!(
        manager.reconcile_context_gauge(80),
        980,
        "provider truth should be carried forward by the local estimate delta"
    );
}

#[test]
fn context_manager_is_the_agent_loop_context_boundary() {
    let manager = manager(190_000);

    assert_eq!(manager.effective_max_context_tokens(), 190_000);
    assert_eq!(
        manager.context_knob_snapshot(),
        (2, context_pruning::COLLAPSE_DISABLED)
    );
    let mut msg = Message::assistant("spill me", vec![]);
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(manager.spill_conversation_message(&mut msg));
    assert!(msg.spill_id.is_some());
}

/// #2160: tokens sent with every request outside the messages (the tool
/// definitions) shrink the room the messages have.
#[tokio::test]
async fn tokens_sent_with_every_request_shrink_the_message_budget() {
    let conversation = || {
        vec![
            long_message(1),
            long_message(2),
            long_message(3),
            long_message(4),
            Message::user("now"),
        ]
    };
    let total = context_pruning::estimate_total_tokens(&conversation());
    let manager = manager(total + 10);

    let mut roomy = conversation();
    manager
        .prepare_provider_context(&mut roomy, total + 10, false)
        .await;
    assert!(roomy.iter().any(|m| m.turn == Some(1)), "everything fits");

    let mut tight = conversation();
    let budget = total + 10 - total / 2;
    let plan = manager
        .prepare_provider_context(&mut tight, budget, false)
        .await;
    assert!(
        !tight.iter().any(|m| m.turn == Some(1)),
        "the oldest turn makes room for what every request carries"
    );
    // The ladder holds the messages to the budget (the spill manifest is
    // added after it and is not part of this bound).
    let messages: Vec<Message> = tight.iter().filter(|m| !m.is_manifest).cloned().collect();
    assert!(!plan.over_budget);
    assert!(context_pruning::estimate_total_tokens(&messages) <= budget);
}

/// #2213: once the ceiling is crossed the plan prunes down to the low-water
/// mark, so the next over-ceiling append lands in the headroom and the
/// durable prefix is rewritten once, not on every turn.
#[tokio::test]
async fn two_consecutive_over_ceiling_appends_latch_the_prefix_dirty_once() {
    let manager = manager(190_000);
    let mut messages = vec![Message::user("current prompt")];
    messages.extend((1..=8).map(long_message));
    let per_turn = context_pruning::estimate_total_tokens(&[long_message(9)]);
    let budget = context_pruning::estimate_total_tokens(&messages) + per_turn / 2;

    let mut latches = 0;
    let mut sent_after_turn_9: Vec<String> = Vec::new();
    for turn in [9, 10] {
        messages.push(long_message(turn));
        let plan = manager
            .prepare_provider_context(&mut messages, budget, false)
            .await;
        assert!(!plan.over_budget);
        latches += usize::from(plan.durable_prefix_dirty);
        if turn == 9 {
            sent_after_turn_9 = messages.iter().map(|m| m.content.clone()).collect();
        }
    }
    assert_eq!(latches, 1, "one prefix rewrite per batch, not per turn");
    // Turn 10's request starts with turn 9's, byte for byte: the cache hits.
    let prefix: Vec<String> = messages[..sent_after_turn_9.len()]
        .iter()
        .map(|m| m.content.clone())
        .collect();
    assert_eq!(prefix, sent_after_turn_9);
}

/// #2212: a transcript of dense output whose estimate is under the budget but
/// whose provider-reported size is over it.
fn digit_transcript() -> Vec<Message> {
    let mut messages = vec![Message::user("current prompt")];
    for turn in 1..=6 {
        let digits: String = (turn * 1_000..turn * 1_000 + 400)
            .map(|n| format!("{n} "))
            .collect();
        let mut msg = Message::assistant(digits, vec![]);
        msg.turn = Some(turn);
        msg.spill_id = Some(format!("turn{turn}:msg:assistant"));
        messages.push(msg);
    }
    messages
}

#[tokio::test]
async fn without_provider_usage_the_ceiling_decides_on_the_heuristic() {
    let estimate = context_pruning::estimate_total_tokens(&digit_transcript());
    let manager = manager(estimate + estimate / 4);
    assert_eq!(
        manager.pruning_ceiling_in_estimate_units(),
        manager.effective_max_context_tokens(),
        "no usage yet: the budget is the heuristic's"
    );

    let mut messages = digit_transcript();
    let plan = manager
        .prepare_provider_context(
            &mut messages,
            manager.pruning_ceiling_in_estimate_units(),
            false,
        )
        .await;

    assert_eq!(
        plan.messages_collapsed + plan.ladder_stubbed + plan.messages_dropped,
        0
    );
}

#[tokio::test]
async fn with_provider_usage_the_ceiling_decides_on_calibrated_occupancy() {
    let estimate = context_pruning::estimate_total_tokens(&digit_transcript());
    let budget = estimate + estimate / 4;
    let manager = manager(budget);
    // The provider counted the digits at about twice the estimate.
    manager.observe_provider_context_gauge(estimate * 2, estimate);
    assert_eq!(manager.estimate_scale().permille(), 2_000);
    assert_eq!(manager.pruning_ceiling_in_estimate_units(), budget / 2);

    let mut messages = digit_transcript();
    let plan = manager
        .prepare_provider_context(
            &mut messages,
            manager.pruning_ceiling_in_estimate_units(),
            false,
        )
        .await;

    assert!(
        plan.ladder_stubbed > 0,
        "calibrated occupancy (2x the estimate) is over the budget"
    );
    let kept = context_pruning::estimate_total_tokens(&messages);
    assert!(
        manager.estimate_scale().calibrated(kept) <= budget,
        "the calibrated transcript fits the budget"
    );
}

#[test]
fn the_calibrated_ceiling_is_clamped_to_between_one_and_four_times_the_heuristic() {
    let manager = manager(100_000);
    manager.observe_provider_context_gauge(50_000, 100_000);
    assert_eq!(manager.pruning_ceiling_in_estimate_units(), 100_000);
    manager.observe_provider_context_gauge(10_000_000, 100_000);
    assert_eq!(manager.pruning_ceiling_in_estimate_units(), 25_000);
}

#[test]
fn a_zero_estimate_leaves_the_heuristic_in_place() {
    let manager = manager(100_000);
    manager.observe_provider_context_gauge(5_000, 0);
    assert_eq!(manager.pruning_ceiling_in_estimate_units(), 100_000);
}

#[test]
fn a_session_change_forgets_the_observed_scale() {
    let mut manager = manager(100_000);
    manager.observe_provider_context_gauge(200_000, 100_000);
    assert_eq!(manager.pruning_ceiling_in_estimate_units(), 50_000);

    manager.set_session_key(SessionIdentity::from_persisted_key("resumed-session"));

    assert_eq!(manager.pruning_ceiling_in_estimate_units(), 100_000);
    assert_eq!(
        manager.reconcile_context_gauge(80),
        80,
        "the display gauge no longer carries the old session's provider figure"
    );
}

#[test]
fn the_calibrated_ceiling_follows_the_model_window() {
    let mut manager = manager(100_000);
    manager.observe_provider_context_gauge(200_000, 100_000);
    manager.set_model_context_window(Some(40_000));
    assert_eq!(manager.pruning_ceiling_in_estimate_units(), 20_000);
    manager.forget_calibration();
    assert_eq!(manager.pruning_ceiling_in_estimate_units(), 40_000);
}

#[test]
fn a_poisoned_gauge_still_yields_the_scale() {
    let manager = manager(100_000);
    manager.observe_provider_context_gauge(200_000, 100_000);
    manager.poison_context_gauge_lock_for_test();
    assert_eq!(manager.estimate_scale().permille(), 2_000);
    manager.forget_calibration();
    assert_eq!(manager.pruning_ceiling_in_estimate_units(), 100_000);
}

/// "Real" tokens under a tokeniser at 4 chars/token for prose and 2 for
/// digit runs: the #2212 QA rates.
fn probe_real(messages: &[Message]) -> usize {
    messages
        .iter()
        .map(|m| {
            let digits = m
                .content
                .chars()
                .filter(|c| c.is_ascii_digit() || *c == ' ')
                .count();
            match digits * 10 >= m.content.len() * 9 {
                true => m.content.len().div_ceil(2),
                false => m.content.len().div_ceil(4),
            }
        })
        .sum()
}

/// Six prose turns (500 tokens each) under a pinned tail of two dense
/// turns of `numbers` four-digit numbers each (2.5 real tokens per number).
fn prose_head_dense_tail(numbers: usize) -> Vec<Message> {
    let mut messages = vec![Message::user("current prompt")];
    for turn in 1..=6 {
        messages.push(long_message(turn));
    }
    for turn in 7..=8 {
        let digits: String = (0..numbers).map(|n| format!("{n:04} ")).collect();
        let mut msg = Message::assistant(digits, vec![]);
        msg.turn = Some(turn);
        msg.spill_id = Some(format!("turn{turn}:msg:assistant"));
        messages.push(msg);
    }
    messages
}

/// Prune `messages` against 90% of their real size, with the provider's
/// count observed; returns (real size after, budget, plan).
async fn prune_at_ninety_percent(messages: &mut Vec<Message>) -> (usize, usize, ContextPlan) {
    let est = context_pruning::estimate_total_tokens(messages);
    let real = probe_real(messages);
    let budget = real * 9 / 10;
    let manager = manager(budget);
    manager.observe_provider_context_gauge(real, est);
    let ceiling = manager.pruning_ceiling_in_estimate_units();
    let plan = manager
        .prepare_provider_context(messages, ceiling, false)
        .await;
    (probe_real(messages), budget, plan)
}

/// Review probe C (#2212): dense content in the pinned tail and prose at
/// the head. A uniform ratio over-valued what stubbing the prose freed
/// (this shape ended at 82% of the budget); the per-class estimate prices
/// each message at its own rate, so the pass reaches the 75% low-water
/// mark (plus the spill manifest the pass adds after the ladder).
#[tokio::test]
async fn a_dense_tail_does_not_erode_the_low_water_mark() {
    let mut messages = prose_head_dense_tail(800);
    let (real_after, budget, plan) = prune_at_ninety_percent(&mut messages).await;
    assert!(plan.ladder_stubbed > 0);
    assert_eq!(plan.messages_dropped, 0);
    assert!(
        real_after * 100 <= budget * 76,
        "pruned to {real_after} of {budget}: above the low-water mark"
    );
    // Prose and digits at their real rates; the stubs' ids and counts carry
    // digits, which the estimate prices a little above the probe's model.
    let estimate = context_pruning::estimate_total_tokens(&messages);
    assert!(
        (real_after..=real_after + real_after / 100).contains(&estimate),
        "{estimate} vs {real_after}"
    );
}

/// The probe's own shape: the pinned dense tail alone is 74% of the
/// budget, so the stubs of every other message put the floor at about
/// 77%. The ladder deletes stubs only to meet the ceiling itself (#2213:
/// the drop rung is a last resort), so it stops there, under the ceiling.
#[tokio::test]
async fn a_pinned_tail_near_the_low_water_mark_is_the_floor() {
    let mut messages = prose_head_dense_tail(1_200);
    let (real_after, budget, plan) = prune_at_ninety_percent(&mut messages).await;
    assert_eq!(plan.ladder_stubbed, 6, "every unpinned message is a stub");
    assert_eq!(plan.messages_dropped, 0);
    assert!(!plan.over_budget);
    assert!(real_after <= budget);
    assert!(real_after * 100 <= budget * 78, "{real_after} of {budget}");
}

/// #2214: the count-based collapse (`context_collapse_after_messages`) and
/// the ceiling ladder are counted apart: a collapse under a roomy budget
/// stubs by count alone, and the ladder stubs nothing.
#[tokio::test]
async fn the_count_based_collapse_is_counted_apart_from_the_ladder() {
    let manager = ContextManager::new(ContextManagerConfig {
        retention: Some(crate::composition::retention::context_retention_over(
            Arc::new(MemSpillStore::default()),
        )),
        session_key: SessionIdentity::from_persisted_key("test-session"),
        context_collapse_after_tool_calls: context_pruning::COLLAPSE_DISABLED,
        max_context_tokens: 190_000,
        pin_recent_turns: 1,
        context_collapse_after_messages: 1,
        large_result_collapse: crate::domain::large_result_collapse::LargeResultCollapse::DISABLED,
        model_context_window: None,
    });
    let mut messages = vec![
        long_message(1),
        long_message(2),
        long_message(3),
        long_message(4),
        Message::user("current prompt"),
    ];

    let plan = manager
        .prepare_provider_context(&mut messages, manager.effective_max_context_tokens(), false)
        .await;

    assert!(plan.messages_collapsed > 0, "{plan:?}");
    assert_eq!(plan.ladder_stubbed, 0, "{plan:?}");
    assert!(plan.durable_prefix_dirty);
}

// --- #2342: superseded snapshots and a swarm member's ceiling ---

fn summary_result(turn: u32) -> Message {
    let mut msg = Message::tool(
        format!("call-{turn}"),
        format!("summary {turn} {}", "y".repeat(800)),
    );
    msg.tool_name = Some("swarm".to_string());
    msg.turn = Some(turn);
    msg.spill_id = Some(format!("turn{turn}:swarm:0"));
    msg.snapshot_key = Some("swarm.summary");
    msg
}

#[tokio::test]
async fn a_plan_supersedes_older_snapshots_and_latches_the_prefix_dirty() {
    let manager = manager(1_000_000);
    let mut messages = vec![Message::user("go"), summary_result(1), summary_result(2)];

    let plan = manager
        .prepare_provider_context(&mut messages, 1_000_000, false)
        .await;

    assert_eq!(plan.snapshots_superseded, 1);
    let result = |id: &str| {
        messages
            .iter()
            .find(|m| m.tool_call_id.as_deref() == Some(id))
            .unwrap()
    };
    assert!(result("call-1").is_collapsed && !result("call-2").is_collapsed);
    assert!(
        plan.durable_prefix_dirty,
        "an in-place rewrite is persisted"
    );
    assert!(plan.total_tokens < plan.tokens_before);
}

#[test]
fn a_ceiling_cap_lowers_the_budget_and_never_raises_it() {
    let manager = manager(200_000);
    let cap = manager.ceiling_cap();
    assert_eq!(cap.tokens(), usize::MAX, "no cap until one is imposed");
    assert_eq!(manager.effective_max_context_tokens(), 200_000);

    cap.lower_to(48_000);
    assert_eq!(manager.effective_max_context_tokens(), 48_000);
    assert_eq!(manager.pruning_ceiling_in_estimate_units(), 48_000);

    cap.lower_to(64_000);
    assert_eq!(cap.tokens(), 48_000, "a cap only ever lowers");
    assert_eq!(manager.effective_max_context_tokens(), 48_000);
}

#[test]
fn a_ceiling_cap_above_the_budget_or_the_window_changes_nothing() {
    let mut manager = manager(40_000);
    manager.ceiling_cap().lower_to(48_000);
    assert_eq!(manager.effective_max_context_tokens(), 40_000);

    manager.set_model_context_window(Some(30_000));
    assert_eq!(manager.effective_max_context_tokens(), 30_000);
}

/// #2349 review L2: once lowered, the cap never disengages: not by a model
/// switch, a new window, a new session, nor a later (higher) value.
#[test]
fn a_lowered_ceiling_never_disengages() {
    let mut manager = manager(200_000);
    manager.ceiling_cap().lower_to(48_000);

    manager.set_model_context_window(Some(1_000_000));
    manager.forget_calibration();
    manager.set_session_key(SessionIdentity::from_persisted_key("another"));
    manager.ceiling_cap().lower_to(usize::MAX);

    assert_eq!(manager.effective_max_context_tokens(), 48_000);
    assert_eq!(manager.pruning_ceiling_in_estimate_units(), 48_000);
}

/// #2349 review M2: the window budget ignores a swarm member's cap.
#[test]
fn the_window_budget_ignores_the_swarm_cap() {
    let mut manager = manager(200_000);
    manager.ceiling_cap().lower_to(40_000);
    assert_eq!(manager.window_budget_tokens(), 200_000);
    manager.set_model_context_window(Some(100_000));
    assert_eq!(manager.window_budget_tokens(), 100_000);
    assert_eq!(manager.effective_max_context_tokens(), 40_000);
}

// --- #2348: the size-aware collapse ---

#[tokio::test]
async fn a_plan_collapses_a_large_seen_result_and_latches_the_prefix_dirty() {
    use crate::domain::large_result_collapse::LargeResultCollapse;
    use crate::domain::message::ToolCall;
    let manager = ContextManager::new(ContextManagerConfig {
        large_result_collapse: LargeResultCollapse {
            over_tokens: 2_000,
            after_turns: 3,
        },
        ..config_for(1_000_000)
    });
    let mut large = Message::tool("call-1", "the quick brown fox ".repeat(2_000));
    large.tool_name = Some("bash".to_string());
    large.spill_id = Some("turn1:bash:0".to_string());
    let call = ToolCall {
        id: "call-1".to_string(),
        name: "bash".to_string(),
        arguments: "{}".to_string(),
    };
    let mut messages = vec![
        Message::user("go"),
        Message::assistant("", vec![call]),
        large,
    ];
    for n in 0..3 {
        messages.push(Message::assistant(format!("reply {n}"), vec![]));
        messages.push(Message::user(format!("next {n}")));
    }

    let plan = manager
        .prepare_provider_context(&mut messages, 1_000_000, false)
        .await;

    assert_eq!(plan.large_results_collapsed, 1);
    assert_eq!(plan.tool_results_collapsed, 0, "not the count dial's");
    let result = messages
        .iter()
        .find(|m| m.tool_call_id.as_deref() == Some("call-1"))
        .unwrap();
    assert!(result.is_collapsed, "{}", result.content);
    assert!(
        plan.durable_prefix_dirty,
        "an in-place rewrite is persisted"
    );
    assert!(plan.total_tokens < plan.tokens_before);
}
