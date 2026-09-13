use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use super::*;
use crate::domain::ids::AgentUuid;
use crate::domain::subagent_teardown::LaunchGeneration;
use crate::infrastructure::tools::subagent_registry::{
    SubagentNotification, new_exit_signal_channel, new_notification_channel, new_registry,
};

fn launched(uuid: &str, display: &str, generation: u64) -> SubagentEntry {
    let mut entry = SubagentEntry::with_identity(
        AgentUuid::new(uuid),
        display.into(),
        PathBuf::from(format!("/tmp/{uuid}.sock")),
        0,
    );
    entry.launch_generation = Some(LaunchGeneration::new(generation));
    entry
}

fn reported(uuid: &str, display: &str, parent: &str, generation: u64) -> SubagentEntry {
    let mut entry =
        SubagentEntry::with_identity(AgentUuid::new(uuid), display.into(), PathBuf::new(), 4242);
    entry.reported_generation = Some(LaunchGeneration::new(generation));
    entry.parent_id = Some(parent.into());
    entry
}

fn identity(uuid: &str, generation: u64) -> DelegatedAgentIdentity {
    DelegatedAgentIdentity::new(uuid, LaunchGeneration::new(generation))
}

/// root → A → B, A → C, root → D, plus a fixture row keyed by label.
fn tree() -> SubagentRegistry {
    let registry = new_registry();
    {
        let mut entries = registry.lock().unwrap();
        entries.insert("A".into(), launched("A", "alpha", 1));
        entries.insert("B".into(), reported("B", "bravo", "A", 1));
        entries.insert("C".into(), reported("C", "charlie", "A", 1));
        entries.insert("D".into(), launched("D", "delta", 2));
        entries.insert(
            "fixture".into(),
            SubagentEntry::new(PathBuf::from("/tmp/fixture.sock"), 0),
        );
    }
    registry
}

fn agents(registry: &SubagentRegistry) -> RegistryDelegatedAgents {
    RegistryDelegatedAgents::new(registry.clone(), None, None)
        .with_compensation_wait(Duration::from_millis(200))
}

#[test]
fn resolution_accepts_uuids_and_live_labels_and_refuses_the_rest() {
    let registry = tree();
    let port = agents(&registry);
    assert_eq!(port.resolve("A"), Ok(identity("A", 1)));
    assert_eq!(port.resolve("delta"), Ok(identity("D", 2)));
    assert_eq!(port.resolve("bravo"), Ok(identity("B", 1)));
    assert_eq!(port.resolve("ghost"), Err(ResolutionError::Unknown));
    assert_eq!(port.resolve("fixture"), Err(ResolutionError::NotDelegated));
    // Two live rows sharing a label are ambiguous.
    registry
        .lock()
        .unwrap()
        .insert("A2".into(), launched("A2", "alpha", 5));
    assert_eq!(port.resolve("alpha"), Err(ResolutionError::Ambiguous));
    // A compensated row is exited, by uuid and by label.
    registry.lock().unwrap()["D"]
        .teardown
        .send_replace(TeardownPhase::Compensated);
    assert_eq!(port.resolve("D"), Err(ResolutionError::Exited));
    assert_eq!(port.resolve("delta"), Err(ResolutionError::Exited));
    // Once the row is also exited in the registry's status vocabulary the
    // label no longer resolves at all.
    registry.lock().unwrap().get_mut("D").unwrap().status = SubagentStatus::Exited;
    assert_eq!(port.resolve("delta"), Err(ResolutionError::Unknown));
}

#[test]
fn claims_walk_the_phases_exactly_once() {
    let registry = tree();
    let port = agents(&registry);
    let a = identity("A", 1);
    assert_eq!(
        port.claim_stopping(&identity("A", 9), TerminationCause::SelectedTermination),
        Err(StoppingClaimError::Unknown),
        "a stale generation names no row"
    );
    assert_eq!(
        port.claim_stopping(&a, TerminationCause::SelectedTermination),
        Ok(())
    );
    assert_eq!(
        port.claim_stopping(&a, TerminationCause::SelectedTermination),
        Err(StoppingClaimError::AlreadyStopping)
    );
    port.release_stopping(&a);
    assert_eq!(
        registry.lock().unwrap()["A"].teardown_phase(),
        TeardownPhase::Live
    );
    port.release_stopping(&a);
    assert_eq!(
        port.claim_stopping(&a, TerminationCause::SelectedTermination),
        Ok(())
    );
    assert_eq!(port.claim_terminal(&a), TerminalClaim::Claimed);
    assert_eq!(port.claim_terminal(&a), TerminalClaim::AlreadyClaimed);
    assert_eq!(
        port.claim_stopping(&a, TerminationCause::SelectedTermination),
        Err(StoppingClaimError::Exited)
    );
    port.release_stopping(&a);
    assert_eq!(
        registry.lock().unwrap()["A"].teardown_phase(),
        TeardownPhase::Compensating(TeardownIntent::SelectedTermination),
        "a release never undoes a terminal claim"
    );
    assert_eq!(
        port.claim_terminal(&identity("ghost", 1)),
        TerminalClaim::AlreadyClaimed
    );
    // A row that is exited in the registry's older vocabulary cannot be
    // claimed stopping either.
    registry.lock().unwrap().get_mut("D").unwrap().status = SubagentStatus::Exited;
    assert_eq!(
        port.claim_stopping(&identity("D", 2), TerminationCause::SelectedTermination),
        Err(StoppingClaimError::Exited)
    );
}

#[test]
fn process_is_held_only_while_the_supervisor_retains_the_handle() {
    let registry = tree();
    let port = agents(&registry);
    assert!(!port.holds_process(&identity("A", 1)));
    let supervisor = Arc::new(
        crate::infrastructure::processes::owned_child_supervisor::OwnedChildSupervisor::new(),
    );
    {
        let mut entries = registry.lock().unwrap();
        let entry = entries.get_mut("A").unwrap();
        entry.owned_child =
            Some(crate::infrastructure::processes::owned_child_supervisor::ChildHandleId::probe(7));
        entry.owned_child_supervisor = Some(supervisor);
    }
    assert!(
        !port.holds_process(&identity("A", 1)),
        "a handle the supervisor never issued is not retained"
    );
}

#[tokio::test]
async fn waiting_for_compensation_observes_the_phase_or_times_out() {
    let registry = tree();
    let port = Arc::new(agents(&registry));
    assert_eq!(
        port.await_compensated(&identity("ghost", 1)).await,
        CompensationObservation::Unknown
    );
    assert_eq!(
        port.await_compensated(&identity("A", 1)).await,
        CompensationObservation::TimedOut
    );
    let waiter = {
        let port = port.clone();
        tokio::spawn(async move { port.await_compensated(&identity("B", 1)).await })
    };
    tokio::time::sleep(Duration::from_millis(30)).await;
    registry.lock().unwrap()["B"]
        .teardown
        .send_replace(TeardownPhase::Compensated);
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), waiter)
            .await
            .unwrap()
            .unwrap(),
        CompensationObservation::Compensated
    );
    // Rows keyed by a label are found through their uuid.
    let uuid = registry.lock().unwrap()["fixture"].agent_uuid.clone();
    let fixture = DelegatedAgentIdentity::new(uuid, LaunchGeneration::new(0));
    assert_eq!(port.claim_terminal(&fixture), TerminalClaim::Claimed);
    assert_eq!(port.claim_terminal(&fixture), TerminalClaim::AlreadyClaimed);
}

#[tokio::test]
async fn compensation_removes_the_subtree_broadcasts_once_and_notifies_for_exits() {
    let registry = tree();
    let (broadcast_tx, mut broadcast_rx) = tokio::sync::broadcast::channel::<String>(8);
    let (notify_tx, mut notify_rx) = new_notification_channel();
    let (exit_tx, _exit_rx) = new_exit_signal_channel();
    let (b_exit_tx, _b_exit_rx) = new_exit_signal_channel();
    let monitor = Arc::new(tokio::spawn(std::future::pending::<()>()));
    {
        let mut entries = registry.lock().unwrap();
        let a = entries.get_mut("A").unwrap();
        a.exit_signal_tx = Some(exit_tx.clone());
        a.monitor_handle = Some(monitor.clone());
        entries.get_mut("B").unwrap().exit_signal_tx = Some(b_exit_tx.clone());
    }
    let port = RegistryDelegatedAgents::new(registry.clone(), Some(broadcast_tx), Some(notify_tx));
    let a = identity("A", 1);
    assert_eq!(port.claim_terminal(&a), TerminalClaim::Claimed);
    let compensated = port
        .compensate(
            &a,
            TerminationCause::Exit(ExitObservation::ConnectionClosed),
        )
        .await;
    assert_eq!(compensated.removed[0], AgentUuid::new("A"));
    let mut rest: Vec<_> = compensated.removed[1..].to_vec();
    rest.sort();
    assert_eq!(rest, [AgentUuid::new("B"), AgentUuid::new("C")]);
    // One survivor-only broadcast: D and the fixture survive, A's subtree is gone.
    let event = broadcast_rx.try_recv().expect("one broadcast");
    assert!(event.contains("\"agentUuid\":\"D\""));
    assert!(!event.contains("\"agentUuid\":\"A\""));
    assert!(!event.contains("\"agentUuid\":\"B\""));
    assert!(broadcast_rx.try_recv().is_err(), "exactly one broadcast");
    // The passive note names the exit kind truthfully.
    let note = notify_rx.try_recv().expect("one exited note");
    assert_eq!(
        note.notification,
        SubagentNotification::Exited {
            agent_id: "alpha".into(),
            reason: Some("connection_closed".into()),
        }
    );
    assert!(notify_rx.try_recv().is_err());
    // Exit signals: the unowned target carries the observation kind; a
    // descendant carries no status of its own.
    assert_eq!(
        exit_tx.borrow().as_ref().map(|s| s.kind),
        Some(ExitSignalKind::ConnectionClosed)
    );
    assert_eq!(
        b_exit_tx.borrow().as_ref().map(|s| (s.kind, s.signal)),
        Some((ExitSignalKind::Terminated, None)),
        "no fabricated signal number for a descendant"
    );
    // Every removed row is compensated and dead; the monitor was aborted.
    {
        let entries = registry.lock().unwrap();
        for key in ["A", "B", "C"] {
            assert_eq!(entries[key].teardown_phase(), TeardownPhase::Compensated);
            assert_eq!(entries[key].status, SubagentStatus::Exited);
        }
        assert_eq!(entries["D"].teardown_phase(), TeardownPhase::Live);
    }
    for _ in 0..100 {
        if monitor.is_finished() {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(monitor.is_finished());
}

#[tokio::test]
async fn a_selected_termination_compensates_silently_and_an_unknown_row_removes_nothing() {
    let registry = tree();
    let (broadcast_tx, mut broadcast_rx) = tokio::sync::broadcast::channel::<String>(8);
    let (notify_tx, mut notify_rx) = new_notification_channel();
    let port = RegistryDelegatedAgents::new(registry.clone(), Some(broadcast_tx), Some(notify_tx));
    let compensated = port
        .compensate(&identity("D", 2), TerminationCause::SelectedTermination)
        .await;
    assert_eq!(compensated.removed, [AgentUuid::new("D")]);
    assert!(broadcast_rx.try_recv().is_ok());
    assert!(
        notify_rx.try_recv().is_err(),
        "the operator asked for it: no passive note"
    );
    let nothing = port
        .compensate(
            &identity("ghost", 1),
            TerminationCause::LaunchRollback {
                owns_environment: false,
            },
        )
        .await;
    assert!(nothing.removed.is_empty());
    assert!(broadcast_rx.try_recv().is_err());
}

#[test]
fn causes_map_to_the_cleanup_contract_and_exit_kind() {
    assert_eq!(
        finalize_mode(TerminationCause::Exit(ExitObservation::ProcessExited)),
        FinalizeMode::Exit
    );
    assert_eq!(
        finalize_mode(TerminationCause::SelectedTermination),
        FinalizeMode::ParentKill
    );
    assert_eq!(
        finalize_mode(TerminationCause::LaunchRollback {
            owns_environment: true
        }),
        FinalizeMode::LaunchRollbackOwned
    );
    assert_eq!(
        finalize_mode(TerminationCause::LaunchRollback {
            owns_environment: false
        }),
        FinalizeMode::LaunchRollback
    );
    assert_eq!(
        exit_kind(TerminationCause::Exit(ExitObservation::ProcessExited)),
        ExitSignalKind::ProcessExit
    );
    assert_eq!(
        exit_kind(TerminationCause::Exit(ExitObservation::NeverReachable)),
        ExitSignalKind::NeverReachable
    );
    assert_eq!(
        exit_kind(TerminationCause::LaunchRollback {
            owns_environment: false
        }),
        ExitSignalKind::Terminated
    );
    assert_eq!(ExitSignalKind::Terminated.to_wire_str(), "terminated");
}

/// The reaper observing the exit of a child a kill claimed stopping runs
/// the kill's compensation: no post-mortem inspect mode, no passive note.
#[tokio::test]
async fn an_exit_observed_for_a_claimed_kill_honours_the_kill_intent() {
    let registry = tree();
    let (notify_tx, mut notify_rx) = new_notification_channel();
    let port = RegistryDelegatedAgents::new(registry.clone(), None, Some(notify_tx));
    let d = identity("D", 2);
    port.claim_stopping(&d, TerminationCause::SelectedTermination)
        .unwrap();
    assert_eq!(port.claim_terminal(&d), TerminalClaim::Claimed);
    assert_eq!(
        effective_cause(
            &registry.lock().unwrap()["D"],
            TerminationCause::Exit(ExitObservation::ProcessExited)
        ),
        TerminationCause::SelectedTermination
    );
    port.compensate(&d, TerminationCause::Exit(ExitObservation::ProcessExited))
        .await;
    assert!(notify_rx.try_recv().is_err(), "a kill posts no note");
    let a = identity("A", 1);
    port.claim_stopping(
        &a,
        TerminationCause::LaunchRollback {
            owns_environment: true,
        },
    )
    .unwrap();
    assert_eq!(port.claim_terminal(&a), TerminalClaim::Claimed);
    assert_eq!(
        effective_cause(
            &registry.lock().unwrap()["A"],
            TerminationCause::Exit(ExitObservation::ConnectionClosed)
        ),
        TerminationCause::LaunchRollback {
            owns_environment: true
        }
    );
    // A row nobody claimed keeps the observed cause.
    let registry = tree();
    let port = RegistryDelegatedAgents::new(registry.clone(), None, None);
    assert_eq!(
        port.claim_terminal(&identity("A", 1)),
        TerminalClaim::Claimed
    );
    assert_eq!(
        effective_cause(
            &registry.lock().unwrap()["A"],
            TerminationCause::Exit(ExitObservation::NeverReachable)
        ),
        TerminationCause::Exit(ExitObservation::NeverReachable)
    );
}

/// A descendant that already ended is not re-listed (or re-signalled) by
/// the compensation of its ancestor: only rows moved out of live membership
/// by this compensation are reported.
#[tokio::test]
async fn compensation_reports_only_the_rows_it_moved_out_of_live_membership() {
    let registry = tree();
    let (c_exit_tx, _c_rx) = new_exit_signal_channel();
    {
        let mut entries = registry.lock().unwrap();
        let c = entries.get_mut("C").unwrap();
        c.exit_signal_tx = Some(c_exit_tx.clone());
        let next = super::super::subagent_cascade::next_roster_sequence(&entries);
        super::super::subagent_cascade::mark_entry_dead(entries.get_mut("C").unwrap(), next);
    }
    let port = RegistryDelegatedAgents::new(registry.clone(), None, None);
    let compensated = port
        .compensate(&identity("A", 1), TerminationCause::SelectedTermination)
        .await;
    let mut removed = compensated.removed.clone();
    removed.sort();
    assert_eq!(removed, [AgentUuid::new("A"), AgentUuid::new("B")]);
    assert!(
        c_exit_tx.borrow().is_none(),
        "an already-dead descendant is not signalled again"
    );
}

/// The reported-snapshot prune and the cascade both go through the ladder:
/// a row they mark dead is `Compensated`, so a kill waiting on a nested
/// target resolves at once instead of running out its bound.
#[tokio::test]
async fn prune_and_cascade_release_waiters_through_the_ladder() {
    let registry = tree();
    let port = RegistryDelegatedAgents::new(registry.clone(), None, None)
        .with_compensation_wait(Duration::from_millis(50));
    // A's next snapshot omits B: the merge prunes it.
    let line = serde_json::json!({"type": "subagent_state_changed", "subagents": [{
        "agentId": "charlie", "agentUuid": "C", "status": "idle", "pid": 4242,
        "parentId": "A", "executionBackend": "local", "launchGeneration": 1,
    }]})
    .to_string();
    super::super::subagent_monitor::forward_child_state_changed(&line, &registry, "A")
        .expect("merged");
    assert_eq!(
        registry.lock().unwrap()["B"].teardown_phase(),
        TeardownPhase::Compensated
    );
    let started = std::time::Instant::now();
    assert_eq!(
        port.await_compensated(&identity("B", 1)).await,
        CompensationObservation::Compensated
    );
    assert!(started.elapsed() < Duration::from_millis(40), "no wait");
    assert_eq!(
        registry.lock().unwrap()["C"].teardown_phase(),
        TeardownPhase::Live,
        "a listed descendant stays live"
    );
    // A cascade of A marks C dead the same way.
    super::super::subagent_cascade::cascade_remove(&registry, "A");
    assert_eq!(
        registry.lock().unwrap()["C"].teardown_phase(),
        TeardownPhase::Compensated
    );
    assert_eq!(
        port.await_compensated(&identity("C", 1)).await,
        CompensationObservation::Compensated
    );
}
