//! Review of #1938: the common shutdown sweeps the fleet once more after
//! joining an operator's run, so a child registered between that run's last
//! lineage read and the freeze is still settled before the harness exits.
use std::sync::atomic::Ordering;

use super::tests::{rig, signal};
use super::*;
use crate::application::subagents::use_cases::teardown_fakes::*;
use crate::domain::ids::AgentUuid;
use crate::domain::subagent_teardown::HarnessLifecycleState;

/// Review of #1938: an operator fleet run (delete-all, a session switch)
/// already past its lineage read, a spawn admitted meanwhile while the
/// harness was still accepting, then a shutdown that joins that run. The
/// joined run never saw the new child; the shutdown sweeps the fleet once
/// more after the join so the child is claimed and compensated before the
/// harness exits.
#[tokio::test]
async fn a_shutdown_that_joins_an_operator_fleet_run_sweeps_children_registered_meanwhile() {
    let rig = rig();
    // The operator's run: past its last lineage read, parked on the prune.
    rig.fleet
        .compensation
        .hold_prune
        .store(true, Ordering::SeqCst);
    let operator = tokio::spawn({
        let fleet = rig.fleet.fleet.clone();
        async move {
            fleet
                .execute(
                    crate::application::subagents::dto::TerminateAllDelegatedAgentsRequest {
                        reason: ShutdownReason::OperatorRequest,
                        authority:
                            crate::application::subagents::dto::FleetTeardownAuthority::Harness,
                    },
                )
                .await
        }
    });
    while rig.fleet.compensation.prunes.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    // A spawn admitted while still accepting.
    rig.fleet.registry.rows.lock().unwrap().insert(
        "E".into(),
        crate::application::subagents::use_cases::lifecycle_fakes::Row {
            identity: identity("E", 3),
            display: "E".into(),
            phase: crate::application::subagents::use_cases::lifecycle_fakes::Phase::Live,
            holds_process: false,
            delegated: true,
        },
    );
    rig.lifecycle.add_record(record("E", 3, "root"));
    // The shutdown freezes and joins the operator's run.
    let token = rig.prepare.execute(signal()).unwrap().token;
    let shutdown = tokio::spawn({
        let execute = rig.execute.clone();
        async move { execute.execute(&token).await }
    });
    while rig.fleet.fleet.waiting_joiners() < 2 {
        tokio::task::yield_now().await;
    }
    // Later prunes (the sweep) run through.
    rig.fleet
        .compensation
        .hold_prune
        .store(false, Ordering::SeqCst);
    rig.fleet.compensation.prune_gate.notify_one();
    let operator = tokio::time::timeout(std::time::Duration::from_secs(10), operator)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(10), shutdown)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(operator.settled.len(), 2, "the operator's run never saw E");
    assert_eq!(
        outcome.children_shut_down,
        [
            AgentUuid::new("A"),
            AgentUuid::new("D"),
            AgentUuid::new("E")
        ]
    );
    assert!(outcome.children_failed.is_empty());
    assert!(
        rig.fleet.registry.rows.lock().unwrap().is_empty(),
        "E claimed, compensated, pruned"
    );
    assert_eq!(rig.routing.calls().len(), 3, "A, D and E asked once each");
    assert_eq!(rig.lifecycle.lifecycle(), HarnessLifecycleState::Terminated);
}
