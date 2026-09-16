//! Contract for the `SessionSwitchRuntime` port (D7 #1976, D8 #1977): the
//! loop runtime a switch moves besides the conversation.
//! `reset_effort_to_default` restores the startup effort and makes the
//! change visible only when something changed; `reset_workflow` clears a
//! bound engine's run and `restore_workflow` replaces it with a saved run
//! the same way; and the accounting reset it inherits still zeroes usage,
//! drops the pending queue and reports the visible count.
use quecto::application::agent_loop::UsageTotals;
use quecto::application::sessions::ports::SessionSwitchRuntime;
use quecto::application::sessions::ports::session_runtime::TurnAccountingReset;
use quecto::domain::provider::EffortLevel;
use quecto::interface::cli::uds_session_switch_runtime::LoopSessionSwitchRuntime;

use super::switch_runtime_fixture::{Runtime, runtime};

fn generation(rt: &Runtime) -> u64 {
    rt.session
        .state_snapshot("cli:contract", 0, None, 0, None)
        .generation
}

#[test]
fn an_effort_override_is_reset_to_the_default_and_the_change_is_visible() {
    let mut rt = runtime("cli:contract");
    rt.agent.set_effort(EffortLevel::Low);
    let before = generation(&rt);
    LoopSessionSwitchRuntime::new(&mut rt.agent, &mut rt.session, &rt.execution, None)
        .reset_effort_to_default();
    assert_eq!(rt.agent.effort(), None, "the startup default was no effort");
    assert!(generation(&rt) > before);
    // Already at the default: nothing changes, nothing is announced.
    let before = generation(&rt);
    LoopSessionSwitchRuntime::new(&mut rt.agent, &mut rt.session, &rt.execution, None)
        .reset_effort_to_default();
    assert_eq!(generation(&rt), before);
}

#[test]
fn a_workflow_run_is_reset_and_the_change_is_visible_only_when_it_moved() {
    let mut rt = runtime("cli:contract");
    {
        let mut engine = rt.workflow.lock().unwrap();
        engine.select_template("feature", None).unwrap();
        engine.check(1).unwrap();
        assert!(engine.persisted_run().is_some());
    }
    let before = generation(&rt);
    LoopSessionSwitchRuntime::new(
        &mut rt.agent,
        &mut rt.session,
        &rt.execution,
        Some(&rt.workflow),
    )
    .reset_workflow();
    assert!(rt.workflow.lock().unwrap().persisted_run().is_none());
    assert!(generation(&rt) > before);
    // Without a bound engine the reset is a no-op.
    let before = generation(&rt);
    LoopSessionSwitchRuntime::new(&mut rt.agent, &mut rt.session, &rt.execution, None)
        .reset_workflow();
    assert_eq!(generation(&rt), before);
}

/// D8 #1977: a resume restores the saved session's run; the change is
/// visible only when the engine's snapshot moved, and without a bound
/// engine nothing happens.
#[test]
fn a_saved_workflow_run_is_restored_and_the_change_is_visible_only_when_it_moved() {
    let mut rt = runtime("cli:contract");
    let saved = {
        let mut engine = rt.workflow.lock().unwrap();
        engine.select_template("feature", None).unwrap();
        engine.check(1).unwrap();
        let run = engine.persisted_run().expect("a run to save");
        engine.reset();
        assert!(engine.persisted_run().is_none());
        run
    };
    let before = generation(&rt);
    LoopSessionSwitchRuntime::new(
        &mut rt.agent,
        &mut rt.session,
        &rt.execution,
        Some(&rt.workflow),
    )
    .restore_workflow(saved.clone());
    assert_eq!(
        rt.workflow.lock().unwrap().persisted_run(),
        Some(saved.clone())
    );
    assert!(generation(&rt) > before);
    // Restoring the run the engine already holds changes nothing visible.
    let before = generation(&rt);
    LoopSessionSwitchRuntime::new(
        &mut rt.agent,
        &mut rt.session,
        &rt.execution,
        Some(&rt.workflow),
    )
    .restore_workflow(saved.clone());
    assert_eq!(generation(&rt), before);
    // Without a bound engine the restore is a no-op.
    let before = generation(&rt);
    LoopSessionSwitchRuntime::new(&mut rt.agent, &mut rt.session, &rt.execution, None)
        .restore_workflow(saved);
    assert_eq!(generation(&rt), before);
}

#[test]
fn the_inherited_accounting_reset_still_zeroes_usage_and_reports_the_count() {
    let mut rt = runtime("cli:contract");
    rt.session
        .record_usage("cli:contract", UsageTotals::billed(10, 5, 2, 1, 7));
    rt.session.set_context_tokens(42);
    assert!(rt.session.enqueue_pending("follow-up".into()));
    LoopSessionSwitchRuntime::new(&mut rt.agent, &mut rt.session, &rt.execution, None)
        .history_replaced(3);
    assert_eq!(rt.session.usage_snapshot().tokens.total, 0);
    assert_eq!(rt.session.context_tokens(), 0);
    assert!(rt.session.drain_pending().is_empty());
    assert_eq!(rt.execution.lock().unwrap().message_count(), 3);
}
