use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use super::*;
use crate::application::subagents::use_cases::lifecycle_fakes::Phase;
use crate::application::subagents::use_cases::teardown_fakes::*;
use crate::domain::subagent_teardown::LineageSnapshot;

fn request(reason: ShutdownReason) -> TerminateAllDelegatedAgentsRequest {
    TerminateAllDelegatedAgentsRequest { reason }
}

struct Rig {
    lifecycle: Arc<FakeLifecycle>,
    routing: Arc<FakeRouting>,
    spawner: Arc<FakeSpawner>,
    fleet: FakeFleet,
}

fn rig() -> Rig {
    let lifecycle = FakeLifecycle::new(root_tree());
    let routing = FakeRouting::new();
    let spawner = FakeSpawner::new();
    let fleet = fake_fleet(lifecycle.clone(), routing.clone(), spawner.clone());
    Rig {
        lifecycle,
        routing,
        spawner,
        fleet,
    }
}

async fn bounded<F: std::future::Future>(future: F) -> F::Output {
    tokio::time::timeout(Duration::from_secs(10), future)
        .await
        .expect("bounded await timed out")
}

fn results(outcome: &FleetTeardownOutcome) -> Vec<(&str, FleetChildResult)> {
    outcome
        .settled
        .iter()
        .map(|settled| (settled.child.uuid.as_str(), settled.result))
        .collect()
}

#[tokio::test]
async fn every_direct_child_is_claimed_asked_concluded_and_compensated_then_tombstones_pruned() {
    let rig = rig();
    let outcome = bounded(
        rig.fleet
            .fleet
            .execute(request(ShutdownReason::OperatorRequest)),
    )
    .await
    .unwrap();
    assert_eq!(outcome.reason, ShutdownReason::OperatorRequest);
    assert!(!outcome.joined);
    assert!(outcome.is_settled());
    assert_eq!(
        results(&outcome),
        [
            ("A", FleetChildResult::Graceful),
            ("D", FleetChildResult::Graceful)
        ]
    );
    // Only direct children are addressed; B and C belong to A's own shutdown.
    assert_eq!(
        rig.routing.calls(),
        [
            RoutingCall::Shutdown(identity("A", 1), ShutdownReason::OperatorRequest),
            RoutingCall::Shutdown(identity("D", 1), ShutdownReason::OperatorRequest),
        ]
    );
    let attempts: Vec<_> = rig
        .fleet
        .termination
        .calls()
        .into_iter()
        .map(|(_, attempt, budget)| (attempt, budget))
        .collect();
    assert_eq!(
        attempts,
        [
            (ProtocolAttempt::Acknowledged, ConclusionBudget::Standard),
            (ProtocolAttempt::Acknowledged, ConclusionBudget::Standard)
        ]
    );
    assert_eq!(
        rig.fleet.compensation.calls(),
        [
            (identity("A", 1), TerminationCause::FleetTeardown),
            (identity("D", 1), TerminationCause::FleetTeardown)
        ]
    );
    // Per child: claim before the edge, terminal claim before compensation.
    let trace = rig.fleet.registry.trace();
    for child in ["A", "D"] {
        let at = |event: &str| {
            trace
                .iter()
                .position(|line| line == &format!("{event} {child}"))
                .unwrap_or_else(|| panic!("{event} {child} missing from {trace:?}"))
        };
        assert!(at("claim-stopping") < at("claim-terminal"));
        assert!(at("claim-terminal") < at("compensated"));
        assert!(at("compensated") < at("pruned"));
    }
    // The compensated tombstones were pruned: the roster holds no row.
    let mut pruned: Vec<_> = outcome.pruned.iter().map(|u| u.as_str()).collect();
    pruned.sort_unstable();
    assert_eq!(pruned, ["A", "D"]);
    assert!(rig.fleet.registry.rows.lock().unwrap().is_empty());
    assert_eq!(outcome.removed_count(), 2);
    assert!(!rig.fleet.fleet.in_flight());
    assert_eq!(rig.spawner.spawned.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn an_empty_fleet_settles_at_once_with_nothing_asked() {
    let lifecycle = FakeLifecycle::new(LineageSnapshot {
        owner: AgentUuid::new("root"),
        records: Vec::new(),
    });
    let routing = FakeRouting::new();
    let fleet = fake_fleet(lifecycle, routing.clone(), FakeSpawner::new());
    let outcome = bounded(fleet.fleet.execute(request(ShutdownReason::ParentShutdown)))
        .await
        .unwrap();
    assert!(outcome.settled.is_empty() && outcome.unsettled.is_empty());
    assert!(outcome.pruned.is_empty());
    assert!(routing.calls().is_empty());
}

#[tokio::test]
async fn concurrent_triggers_join_one_run_and_no_child_is_asked_twice() {
    let rig = rig();
    *rig.routing.hold_child.lock().unwrap() = Some(AgentUuid::new("A"));
    let first = tokio::spawn({
        let fleet = rig.fleet.fleet.clone();
        async move {
            fleet
                .execute(request(ShutdownReason::OperatorRequest))
                .await
        }
    });
    while rig.routing.calls().len() < 2 {
        tokio::task::yield_now().await;
    }
    assert!(rig.fleet.fleet.in_flight());
    let second = tokio::spawn({
        let fleet = rig.fleet.fleet.clone();
        async move {
            fleet
                .execute(request(ShutdownReason::TerminationSignal))
                .await
        }
    });
    tokio::task::yield_now().await;
    rig.routing.gate.notify_one();
    let first = bounded(first).await.unwrap().unwrap();
    let second = bounded(second).await.unwrap().unwrap();
    assert!(!first.joined);
    assert!(second.joined, "the second trigger joined the first run");
    // The joiner observes the first run's reason and outcome.
    assert_eq!(second.reason, ShutdownReason::OperatorRequest);
    assert_eq!(first.settled, second.settled);
    assert_eq!(rig.routing.calls().len(), 2, "A and D asked once each");
    assert_eq!(rig.fleet.compensation.calls().len(), 2);
    assert_eq!(rig.spawner.spawned.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_child_registered_while_the_run_is_in_flight_is_settled_by_a_further_pass() {
    let rig = rig();
    *rig.routing.hold_child.lock().unwrap() = Some(AgentUuid::new("A"));
    let run = tokio::spawn({
        let fleet = rig.fleet.fleet.clone();
        async move {
            fleet
                .execute(request(ShutdownReason::OperatorRequest))
                .await
        }
    });
    while rig.routing.calls().len() < 2 {
        tokio::task::yield_now().await;
    }
    // A late registration: the row exists and the lineage reports it.
    rig.fleet.registry.rows.lock().unwrap().insert(
        "E".into(),
        crate::application::subagents::use_cases::lifecycle_fakes::Row {
            identity: identity("E", 3),
            display: "E".into(),
            phase: Phase::Live,
            holds_process: false,
            delegated: true,
        },
    );
    rig.lifecycle.add_record(record("E", 3, "root"));
    rig.routing.gate.notify_one();
    let outcome = bounded(run).await.unwrap().unwrap();
    assert_eq!(
        results(&outcome),
        [
            ("A", FleetChildResult::Graceful),
            ("D", FleetChildResult::Graceful),
            ("E", FleetChildResult::Graceful)
        ]
    );
    assert_eq!(rig.routing.calls().len(), 3);
}

#[tokio::test]
async fn a_slow_child_bounds_the_run_without_blocking_its_siblings() {
    let rig = rig();
    rig.fleet.termination.hold.lock().unwrap().push("A".into());
    let run = tokio::spawn({
        let fleet = rig.fleet.fleet.clone();
        async move {
            fleet
                .execute(request(ShutdownReason::OperatorRequest))
                .await
        }
    });
    // D settles while A is still concluding.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while rig.fleet.registry.phase("D") != Phase::Compensated {
        assert!(std::time::Instant::now() < deadline, "D never settled");
        tokio::task::yield_now().await;
    }
    assert!(!run.is_finished(), "the run waits for A");
    assert_eq!(
        rig.fleet.registry.phase("A"),
        Phase::Stopping(TerminationCause::FleetTeardown)
    );
    rig.fleet.termination.gate.notify_one();
    let outcome = bounded(run).await.unwrap().unwrap();
    assert!(outcome.is_settled());
    assert_eq!(outcome.settled.len(), 2);
}

#[tokio::test]
async fn a_child_that_survives_its_fallback_is_reported_unsettled_with_its_claim_lifted() {
    let rig = rig();
    rig.fleet.termination.conclude_child_with(
        "A",
        TerminationConclusion::StillRunning("no exit within budget".into()),
    );
    let outcome = bounded(
        rig.fleet
            .fleet
            .execute(request(ShutdownReason::ParentShutdown)),
    )
    .await
    .unwrap();
    assert!(!outcome.is_settled());
    assert_eq!(results(&outcome), [("D", FleetChildResult::Graceful)]);
    assert_eq!(
        outcome.unsettled,
        [(AgentUuid::new("A"), "no exit within budget".to_owned())]
    );
    // A keeps its row, live again, so a later trigger may try again; D is
    // gone. Nothing was compensated for A.
    assert_eq!(rig.fleet.registry.phase("A"), Phase::Live);
    assert!(rig.fleet.registry.rows.lock().unwrap().get("D").is_none());
    assert_eq!(
        rig.fleet.compensation.calls(),
        [(identity("D", 1), TerminationCause::FleetTeardown)]
    );
    assert!(
        rig.fleet
            .registry
            .trace()
            .contains(&"release-stopping A".to_owned())
    );
}

#[tokio::test]
async fn fallback_and_already_exited_conclusions_are_reported_as_such() {
    let rig = rig();
    rig.fleet
        .termination
        .conclude_child_with("A", TerminationConclusion::ExitedAfterFallback);
    rig.fleet
        .termination
        .conclude_child_with("D", TerminationConclusion::AlreadyExited);
    let outcome = bounded(
        rig.fleet
            .fleet
            .execute(request(ShutdownReason::OperatorRequest)),
    )
    .await
    .unwrap();
    assert_eq!(
        results(&outcome),
        [
            ("A", FleetChildResult::Fallback),
            ("D", FleetChildResult::AlreadyExited)
        ]
    );
}

#[tokio::test]
async fn an_unowned_child_is_compensated_unobserved_when_its_exit_never_arrives() {
    let rig = rig();
    // A is unreachable, D acknowledges; neither is owned by this harness
    // and nothing else observes their exits within the bound.
    rig.routing
        .unreachable
        .lock()
        .unwrap()
        .push(AgentUuid::new("A"));
    *rig.fleet.termination.conclusion.lock().unwrap() = TerminationConclusion::NoRetainedHandle;
    *rig.fleet.registry.time_out_waits.lock().unwrap() = true;
    let outcome = bounded(
        rig.fleet
            .fleet
            .execute(request(ShutdownReason::OperatorRequest)),
    )
    .await
    .unwrap();
    assert!(outcome.is_settled());
    assert_eq!(
        results(&outcome),
        [
            ("A", FleetChildResult::Unobserved),
            ("D", FleetChildResult::Unobserved)
        ]
    );
    // Both were compensated by this run (environment finalization ran).
    assert_eq!(rig.fleet.compensation.calls().len(), 2);
    assert!(
        rig.fleet
            .registry
            .trace()
            .contains(&"await-compensated D".to_owned())
    );
    assert!(
        !rig.fleet
            .registry
            .trace()
            .contains(&"await-compensated A".to_owned())
    );
}

#[tokio::test]
async fn an_unowned_child_whose_reaper_already_claimed_the_end_is_already_exited() {
    let rig = rig();
    *rig.fleet.termination.conclusion.lock().unwrap() = TerminationConclusion::NoRetainedHandle;
    *rig.fleet.termination.reaper_compensates.lock().unwrap() = true;
    let outcome = bounded(
        rig.fleet
            .fleet
            .execute(request(ShutdownReason::OperatorRequest)),
    )
    .await
    .unwrap();
    assert_eq!(
        results(&outcome),
        [
            ("A", FleetChildResult::AlreadyExited),
            ("D", FleetChildResult::AlreadyExited)
        ]
    );
    // The reaper compensated; this run joined and compensated nothing itself.
    assert!(rig.fleet.compensation.calls().is_empty());
}

#[tokio::test]
async fn a_child_another_termination_owns_is_joined_or_reported_unsettled() {
    let rig = rig();
    // A kill already claimed A stopping.
    rig.fleet
        .registry
        .claim_stopping(&identity("A", 1), TerminationCause::SelectedTermination)
        .unwrap();
    let run = tokio::spawn({
        let fleet = rig.fleet.fleet.clone();
        async move {
            fleet
                .execute(request(ShutdownReason::OperatorRequest))
                .await
        }
    });
    // The kill finishes: the run joins its compensation.
    tokio::task::yield_now().await;
    rig.fleet.registry.set_phase("A", Phase::Compensated);
    let outcome = bounded(run).await.unwrap().unwrap();
    assert_eq!(
        results(&outcome),
        [
            ("A", FleetChildResult::Joined),
            ("D", FleetChildResult::Graceful)
        ]
    );
    // A was never asked or compensated by this run.
    assert_eq!(
        rig.routing.calls(),
        [RoutingCall::Shutdown(
            identity("D", 1),
            ShutdownReason::OperatorRequest
        )]
    );
    assert_eq!(rig.fleet.compensation.calls().len(), 1);

    // The same, but the other termination never settles within the bound.
    let rig = self::rig();
    rig.fleet
        .registry
        .claim_stopping(&identity("A", 1), TerminationCause::SelectedTermination)
        .unwrap();
    *rig.fleet.registry.time_out_waits.lock().unwrap() = true;
    let outcome = bounded(
        rig.fleet
            .fleet
            .execute(request(ShutdownReason::OperatorRequest)),
    )
    .await
    .unwrap();
    assert_eq!(outcome.unsettled.len(), 1);
    assert_eq!(outcome.unsettled[0].0, AgentUuid::new("A"));
    // The kill's claim is its own: the fleet run does not lift it.
    assert_eq!(
        rig.fleet.registry.phase("A"),
        Phase::Stopping(TerminationCause::SelectedTermination)
    );
}

#[tokio::test]
async fn dropping_the_caller_never_stops_the_detached_run() {
    let rig = rig();
    *rig.routing.hold_child.lock().unwrap() = Some(AgentUuid::new("A"));
    let caller = tokio::spawn({
        let fleet = rig.fleet.fleet.clone();
        async move {
            fleet
                .execute(request(ShutdownReason::OperatorRequest))
                .await
        }
    });
    while rig.routing.calls().len() < 2 {
        tokio::task::yield_now().await;
    }
    // The client that asked went away.
    caller.abort();
    let _ = caller.await;
    assert!(rig.fleet.fleet.in_flight(), "the run is still in flight");
    rig.routing.gate.notify_one();
    // A later trigger joins the same run and sees it complete.
    let outcome = bounded(
        rig.fleet
            .fleet
            .execute(request(ShutdownReason::ParentShutdown)),
    )
    .await
    .unwrap();
    assert!(outcome.joined);
    assert_eq!(outcome.settled.len(), 2);
    assert_eq!(rig.routing.calls().len(), 2);
}

#[tokio::test]
async fn a_spawner_that_drops_the_run_reports_interruption_and_the_next_trigger_starts_afresh() {
    let rig = rig();
    rig.spawner.drop_next.store(1, Ordering::SeqCst);
    assert_eq!(
        bounded(
            rig.fleet
                .fleet
                .execute(request(ShutdownReason::OperatorRequest))
        )
        .await,
        Err(FleetTeardownError::Interrupted)
    );
    assert!(!rig.fleet.fleet.in_flight());
    assert!(rig.routing.calls().is_empty());
    assert_eq!(rig.fleet.registry.phase("A"), Phase::Live);
    let outcome = bounded(
        rig.fleet
            .fleet
            .execute(request(ShutdownReason::OperatorRequest)),
    )
    .await
    .unwrap();
    assert!(!outcome.joined);
    assert_eq!(outcome.settled.len(), 2);
}

#[tokio::test]
async fn a_run_aborted_mid_flight_releases_its_joiners_as_interrupted() {
    let rig = rig();
    *rig.routing.hold_child.lock().unwrap() = Some(AgentUuid::new("A"));
    let joiner = tokio::spawn({
        let fleet = rig.fleet.fleet.clone();
        async move {
            fleet
                .execute(request(ShutdownReason::OperatorRequest))
                .await
        }
    });
    while rig.routing.calls().len() < 2 {
        tokio::task::yield_now().await;
    }
    rig.spawner.abort_latest().await;
    assert_eq!(
        bounded(joiner).await.unwrap(),
        Err(FleetTeardownError::Interrupted)
    );
    assert!(!rig.fleet.fleet.in_flight());
}

#[test]
fn vocabulary_is_stable() {
    assert_eq!(FleetChildResult::Graceful.to_string(), "graceful");
    assert_eq!(FleetChildResult::Fallback.as_str(), "fallback");
    assert_eq!(FleetChildResult::AlreadyExited.as_str(), "already-exited");
    assert_eq!(FleetChildResult::Unobserved.as_str(), "unobserved");
    assert_eq!(FleetChildResult::Joined.as_str(), "joined");
    assert_eq!(
        FleetTeardownError::Interrupted.to_string(),
        "fleet teardown was interrupted"
    );
    let outcome = FleetTeardownOutcome {
        reason: ShutdownReason::OperatorRequest,
        joined: false,
        settled: vec![SettledChild {
            child: identity("A", 1),
            result: FleetChildResult::Graceful,
        }],
        unsettled: vec![(AgentUuid::new("B"), "slow".into())],
        pruned: vec![AgentUuid::new("C")],
    };
    assert!(!outcome.is_settled());
    assert_eq!(outcome.removed_count(), 2);
}

#[test]
#[should_panic(expected = "the settlement bound must admit at least one child")]
fn a_zero_bound_is_refused() {
    let lifecycle = FakeLifecycle::new(root_tree());
    let routing = FakeRouting::new();
    let fleet = fake_fleet(lifecycle.clone(), routing.clone(), FakeSpawner::new());
    let _ = TerminateAllDelegatedAgents::with_bound(
        TerminateAllDelegatedAgentsPorts {
            lifecycle,
            registry: fleet.registry.clone(),
            routing,
            termination: fleet.termination.clone(),
            compensation: fleet.compensation.clone(),
            spawner: FakeSpawner::new(),
        },
        0,
    );
}
