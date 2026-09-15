//! Contract for the `TurnAccountingReset` port (#1864, #1865, D6 #1975):
//! `history_replaced` zeroes what the runtime accounted for the replaced
//! transcript (usage counters, context-size reading), drops the prompts
//! queued against it, and reports the visible count the transaction
//! computed — nothing else of the runtime moves.
use std::sync::{Arc, Mutex};

use quecto::application::sessions::ports::session_runtime::TurnAccountingReset;
use quecto::interface::cli::uds_execution_state::{ExecutionState, ExecutionStateHandle};
use quecto::interface::cli::uds_session::AgentSession;
use quecto::interface::cli::uds_turn_accounting::LoopTurnAccounting;

fn accounted_session() -> AgentSession {
    let mut session = AgentSession::new("model".into(), "cli:contract".into());
    session.record_usage(10, 5, 2, 1, 7);
    session.set_context_tokens(42);
    assert!(session.enqueue_pending("follow-up".into()));
    assert!(session.prepend_pending("steer".into()));
    session
}

fn execution_state() -> ExecutionStateHandle {
    Arc::new(Mutex::new(ExecutionState::default()))
}

#[test]
fn a_history_replacement_resets_usage_context_and_pending_and_reports_the_count() {
    let mut session = accounted_session();
    let execution = execution_state();
    let model = session.model().to_string();
    let generation_before = session.state_snapshot(0, None, 0, None).generation;

    LoopTurnAccounting::new(&mut session, &execution).history_replaced(3);

    let usage = session.usage_snapshot();
    assert_eq!(usage.tokens.total, 0);
    assert_eq!(usage.tokens.input, 0);
    assert_eq!(usage.tokens.cache_read, 0);
    assert_eq!(usage.cost_micro_usd, 0);
    assert_eq!(session.context_tokens(), 0);
    let snapshot = session.state_snapshot(0, None, 0, None);
    assert_eq!(snapshot.pending_message_count, 0);
    assert!(session.drain_pending().is_empty());
    assert_eq!(execution.lock().unwrap().message_count(), 3);
    // Nothing else of the tracker moves: model, key, streaming, generation.
    assert_eq!(session.model(), model);
    assert_eq!(snapshot.session_key, "cli:contract");
    assert!(!snapshot.is_streaming);
    assert_eq!(snapshot.generation, generation_before);
}

#[test]
fn a_reset_on_a_fresh_runtime_is_idempotent_and_still_reports_the_count() {
    let mut session = AgentSession::new("model".into(), "cli:contract".into());
    let execution = execution_state();
    let mut adapter = LoopTurnAccounting::new(&mut session, &execution);
    adapter.history_replaced(0);
    adapter.history_replaced(5);
    assert_eq!(execution.lock().unwrap().message_count(), 5);
    assert_eq!(session.usage_snapshot().tokens.total, 0);
    assert_eq!(
        session
            .state_snapshot(0, None, 0, None)
            .pending_message_count,
        0
    );
}
