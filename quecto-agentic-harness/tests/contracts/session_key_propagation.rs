//! Contract for the `SessionKeyPropagation` port (D7 #1976, D10 #1979):
//! when the transaction announces a new identity, the loop's raw-key holder
//! adopts it — the agent loop and its session-aware tools — and the
//! tracker, which keeps no copy, resets its usage and bumps its visible
//! generation exactly when the key changed; nothing else of the tracker
//! moves. The `sessionKey` a presenter reports is the active session's.
use quecto::application::agent_loop::UsageTotals;
use quecto::application::sessions::ports::SessionKeyPropagation;
use quecto::domain::session_identity::SessionIdentity;
use quecto::interface::cli::uds_session_switch_runtime::LoopSessionSwitchRuntime;

use super::switch_runtime_fixture::runtime;

#[test]
fn a_new_identity_reaches_the_tracker_and_the_session_aware_tools() {
    let mut rt = runtime("cli:contract");
    rt.session
        .record_usage("cli:contract", UsageTotals::billed(10, 5, 2, 1, 7));
    let generation_before = rt
        .session
        .state_snapshot("cli:contract", 0, None, 0, None)
        .generation;
    let fresh = SessionIdentity::fresh_chat(1_700_000_000, 42);

    LoopSessionSwitchRuntime::new(
        &mut rt.agent,
        &mut rt.session,
        &rt.execution,
        Some(&rt.workflow),
    )
    .session_key_changed(&fresh);

    assert_eq!(rt.agent.session_key(), fresh.runtime_key());
    let snapshot = rt
        .session
        .state_snapshot(fresh.runtime_key(), 0, None, 0, None);
    assert_eq!(snapshot.session_key, fresh.runtime_key());
    assert!(
        snapshot.generation > generation_before,
        "a key change is visible"
    );
    assert_eq!(rt.session.usage_snapshot().tokens.total, 0);
    assert_eq!(
        rt.tool.seen.lock().unwrap().as_slice(),
        [fresh.runtime_key().to_string()]
    );
    assert_eq!(snapshot.model, "stub");
    assert!(!snapshot.is_streaming);
}

#[test]
fn the_same_identity_again_is_a_no_op_on_the_tracker() {
    let mut rt = runtime("cli:contract");
    let generation_before = rt
        .session
        .state_snapshot("cli:contract", 0, None, 0, None)
        .generation;
    let same = SessionIdentity::from_persisted_key("cli:contract");
    LoopSessionSwitchRuntime::new(
        &mut rt.agent,
        &mut rt.session,
        &rt.execution,
        Some(&rt.workflow),
    )
    .session_key_changed(&same);
    assert_eq!(
        rt.session
            .state_snapshot("cli:contract", 0, None, 0, None)
            .generation,
        generation_before
    );
    // The tools are always told: they hold no copy to compare against.
    assert_eq!(rt.tool.seen.lock().unwrap().as_slice(), ["cli:contract"]);
}
