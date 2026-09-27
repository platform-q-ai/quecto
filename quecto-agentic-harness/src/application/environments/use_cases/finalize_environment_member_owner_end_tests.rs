//! #2206: when its owner ends a plain container child on purpose — its own
//! selected kill, or a one-shot parent ending its run — the emptied
//! environment goes for good: the retained `cleanup` removes
//! the container and its state directory, and the record is forgotten, the
//! way a launch rollback discards it. Everything that may be a crash (a
//! harness shutdown, the child's own exit) keeps its old contract, and no
//! swarm's box is ever destroyed by this path.

use std::sync::{Arc, Mutex};

use super::finalize_environment_member_tests::{ScriptCall, block_on, committed_env};
use crate::application::environments::ports::{
    EnvironmentProcessCommands, HostedSwarmRunObservation, PortFuture,
};
use crate::application::environments::use_cases::FinalizeEnvironmentMember;
use crate::domain::environment_registry::{
    EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus,
};
use crate::domain::environment_retention::{
    CoordinatorLoss, HostedSwarmRun, MemberFinalizeMode, SwarmRunObservation,
};
use crate::domain::swarm::RunStatus;

/// One fake for both ports: the hosted run it reports, the scripts it
/// records, and whether the cleanup fails.
struct Port {
    observed: SwarmRunObservation,
    cleanup_error: Option<String>,
    observations: Mutex<usize>,
    losses: Mutex<usize>,
    kills: Mutex<Vec<ScriptCall>>,
    cleanups: Mutex<Vec<ScriptCall>>,
}

impl Port {
    fn observing(observed: SwarmRunObservation) -> Arc<Self> {
        Arc::new(Self {
            observed,
            cleanup_error: None,
            observations: Mutex::new(0),
            losses: Mutex::new(0),
            kills: Mutex::new(Vec::new()),
            cleanups: Mutex::new(Vec::new()),
        })
    }

    fn failing_cleanup(observed: SwarmRunObservation, error: &str) -> Arc<Self> {
        let mut port = Arc::into_inner(Self::observing(observed)).unwrap();
        port.cleanup_error = Some(error.to_string());
        Arc::new(port)
    }

    fn kills(&self) -> usize {
        self.kills.lock().unwrap().len()
    }

    fn cleanups(&self) -> Vec<ScriptCall> {
        std::mem::take(&mut *self.cleanups.lock().unwrap())
    }
}

impl HostedSwarmRunObservation for Port {
    fn observe_hosted_swarm_run<'a>(
        &'a self,
        _record: &'a EnvironmentRecord,
    ) -> PortFuture<'a, SwarmRunObservation> {
        *self.observations.lock().unwrap() += 1;
        Box::pin(async move { self.observed.clone() })
    }

    fn record_lost_coordinator<'a>(
        &'a self,
        _record: &'a EnvironmentRecord,
        hosted: &'a HostedSwarmRun,
    ) -> PortFuture<'a, Result<CoordinatorLoss, String>> {
        *self.losses.lock().unwrap() += 1;
        Box::pin(async move {
            Ok(CoordinatorLoss {
                run: hosted.clone(),
                lost: !hosted.ended(),
            })
        })
    }
}

impl EnvironmentProcessCommands for Port {
    fn run_retained_inspect<'a>(
        &'a self,
        _environment_id: &'a str,
        _argv: &'a [String],
    ) -> PortFuture<'a, Result<serde_json::Value, String>> {
        Box::pin(async { Ok(serde_json::json!({})) })
    }

    fn run_retained_kill<'a>(
        &'a self,
        environment_id: &'a str,
        argv: &'a [String],
    ) -> PortFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.kills.lock().unwrap().push(ScriptCall {
                environment_id: environment_id.to_string(),
                argv: argv.to_vec(),
            });
            Ok(())
        })
    }

    fn observe_liveness<'a>(
        &'a self,
        _record: &'a crate::domain::environment_registry::EnvironmentRecord,
    ) -> PortFuture<'a, crate::application::environments::dto::EnvironmentLiveness> {
        Box::pin(async { panic!("a final-member teardown never asks liveness") })
    }

    fn run_retained_cleanup<'a>(
        &'a self,
        environment_id: &'a str,
        argv: &'a [String],
    ) -> PortFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.cleanups.lock().unwrap().push(ScriptCall {
                environment_id: environment_id.to_string(),
                argv: argv.to_vec(),
            });
            self.cleanup_error.clone().map_or(Ok(()), Err)
        })
    }
}

fn swarm(status: RunStatus, outcome: Option<RunStatus>) -> HostedSwarmRun {
    HostedSwarmRun {
        id: "run-9".to_string(),
        status,
        outcome,
        coordinator: "member-9".to_string(),
        deadline: 4_102_444_800.0,
    }
}

fn placeholder() -> SwarmRunObservation {
    SwarmRunObservation::Run(HostedSwarmRun {
        deadline: 0.0,
        ..swarm(RunStatus::Setup, None)
    })
}

/// Every swarm that has not been closed by its owner, and a store that
/// cannot be read: the boxes this change must never destroy.
fn unclosed_swarms() -> Vec<SwarmRunObservation> {
    vec![
        SwarmRunObservation::Run(swarm(RunStatus::Running, None)),
        SwarmRunObservation::Run(swarm(RunStatus::Paused, None)),
        SwarmRunObservation::Run(swarm(RunStatus::Paused, Some(RunStatus::Failed))),
        SwarmRunObservation::Run(swarm(RunStatus::Cancelled, None)),
        SwarmRunObservation::Unreadable("store locked".to_string()),
    ]
}

fn finalize(
    port: &Arc<Port>,
    mode: MemberFinalizeMode,
    entry_plan: Option<(String, Vec<String>)>,
    registry: &EnvironmentRegistry,
    env_ref: &str,
) {
    let use_case = FinalizeEnvironmentMember::new(registry.clone(), port.clone(), port.clone());
    block_on(use_case.finalize_member(env_ref, "child-uuid", entry_plan, mode));
}

fn one_child_env() -> (EnvironmentRegistry, String) {
    let registry = EnvironmentRegistry::new();
    let env_ref = committed_env(&registry, vec!["child-uuid"]);
    (registry, env_ref)
}

#[test]
fn the_owners_end_of_a_plain_child_cleans_its_box_up_and_forgets_it() {
    {
        let mode = MemberFinalizeMode::OwnerEnd;
        for observed in [SwarmRunObservation::NoStore, placeholder()] {
            let port = Port::observing(observed.clone());
            let (registry, env_ref) = one_child_env();
            let environment_id = registry.get(&env_ref).unwrap().environment_id;

            finalize(&port, mode, None, &registry, &env_ref);

            assert_eq!(
                port.cleanups(),
                vec![ScriptCall {
                    environment_id,
                    argv: vec!["cleanup.sh".to_string()],
                }],
                "{mode:?} on {observed:?}: the retained cleanup ran once"
            );
            assert_eq!(port.kills(), 0, "{mode:?}: cleanup, never kill");
            assert!(
                registry.get(&env_ref).is_none(),
                "{mode:?} on {observed:?}: the record is forgotten"
            );
            assert_eq!(*port.losses.lock().unwrap(), 0, "no loss recorded");
        }
    }
}

#[test]
fn a_plain_child_without_its_own_cleanup_argv_uses_the_members_plan() {
    let port = Port::observing(SwarmRunObservation::NoStore);
    let registry = EnvironmentRegistry::new();
    let env_ref = committed_env(&registry, vec!["child-uuid"]);
    let mut record = registry.get(&env_ref).unwrap();
    record.retained_cleanup_argv.clear();
    registry.commit(record);
    let plan = (
        "runtime-plan".to_string(),
        vec!["plan-cleanup.sh".to_string()],
    );

    finalize(
        &port,
        MemberFinalizeMode::OwnerEnd,
        Some(plan.clone()),
        &registry,
        &env_ref,
    );

    assert_eq!(
        port.cleanups(),
        vec![ScriptCall {
            environment_id: plan.0,
            argv: plan.1,
        }]
    );
    assert!(registry.get(&env_ref).is_none());
}

#[test]
fn a_plain_child_with_no_cleanup_at_all_keeps_the_ordinary_kill() {
    let port = Port::observing(SwarmRunObservation::NoStore);
    let registry = EnvironmentRegistry::new();
    let env_ref = committed_env(&registry, vec!["child-uuid"]);
    let mut record = registry.get(&env_ref).unwrap();
    record.retained_cleanup_argv.clear();
    registry.commit(record);

    finalize(
        &port,
        MemberFinalizeMode::OwnerEnd,
        Some(("runtime-plan".to_string(), Vec::new())),
        &registry,
        &env_ref,
    );

    assert!(port.cleanups().is_empty(), "nothing names a cleanup");
    assert_eq!(port.kills(), 1, "the retained kill still ends the box");
    assert_eq!(
        registry.get(&env_ref).unwrap().status,
        EnvironmentStatus::Stopped,
        "a record is forgotten only after a cleanup"
    );
}

#[test]
fn a_failed_cleanup_keeps_the_record_cleanup_failed_and_retryable() {
    let port = Port::failing_cleanup(SwarmRunObservation::NoStore, "podman: busy");
    let (registry, env_ref) = one_child_env();

    finalize(
        &port,
        MemberFinalizeMode::OwnerEnd,
        None,
        &registry,
        &env_ref,
    );

    assert_eq!(port.cleanups().len(), 1);
    let record = registry
        .get(&env_ref)
        .expect("never forgotten while its box may remain");
    assert_eq!(record.status, EnvironmentStatus::CleanupFailed);
    assert_eq!(record.last_error.as_deref(), Some("podman: busy"));
    assert!(record.members.is_empty());
    assert!(
        registry.begin_kill(&env_ref).is_ok(),
        "container kill retries it"
    );
}

#[test]
fn a_swarm_child_the_owner_ended_keeps_its_box_like_a_supervisor_kill() {
    for observed in unclosed_swarms() {
        let port = Port::observing(observed.clone());
        let (registry, env_ref) = one_child_env();

        finalize(
            &port,
            MemberFinalizeMode::OwnerEnd,
            None,
            &registry,
            &env_ref,
        );

        assert!(port.cleanups().is_empty(), "{observed:?}: no cleanup");
        assert_eq!(port.kills(), 0, "{observed:?}: no kill");
        let record = registry.get(&env_ref).expect("record kept");
        assert_eq!(record.status, EnvironmentStatus::Retained, "{observed:?}");
        assert_eq!(*port.losses.lock().unwrap(), 0, "a kill records no loss");
        assert!(
            registry.begin_kill(&env_ref).is_ok(),
            "an explicit kill ends it"
        );
    }
}

#[test]
fn a_swarm_its_owner_closed_keeps_the_ordinary_kill_and_its_record() {
    for mode in [
        MemberFinalizeMode::OwnerEnd,
        MemberFinalizeMode::OwnerTeardown,
    ] {
        let port = Port::observing(SwarmRunObservation::Run(swarm(RunStatus::Succeeded, None)));
        let (registry, env_ref) = one_child_env();

        finalize(&port, mode, None, &registry, &env_ref);

        assert!(port.cleanups().is_empty(), "{mode:?}");
        assert_eq!(port.kills(), 1, "{mode:?}: the swarm rules run the kill");
        assert_eq!(
            registry.get(&env_ref).unwrap().status,
            EnvironmentStatus::Stopped
        );
    }
}

#[test]
fn the_owners_explicit_teardown_is_unchanged() {
    // #2070: delete-all ends every box with the retained kill, reading no
    // store; #2206 leaves it so, a plain box included.
    let mut every = unclosed_swarms();
    every.push(SwarmRunObservation::NoStore);
    for observed in every {
        let port = Port::observing(observed.clone());
        let (registry, env_ref) = one_child_env();

        finalize(
            &port,
            MemberFinalizeMode::OwnerTeardown,
            None,
            &registry,
            &env_ref,
        );

        assert_eq!(port.kills(), 1, "{observed:?}");
        assert!(port.cleanups().is_empty(), "{observed:?}");
        assert_eq!(*port.observations.lock().unwrap(), 0, "no store read");
        assert_eq!(
            registry.get(&env_ref).unwrap().status,
            EnvironmentStatus::Stopped,
            "{observed:?}: listed stopped, not forgotten"
        );
    }
}

#[test]
fn a_harness_shutdown_or_a_crashed_parents_child_is_never_ended_for_good() {
    // A harness shutdown (unannounced: maybe a crash) finalizes as a
    // ParentKill; a child that went on its own — its parent crashed, it
    // crashed — as an Exit. Neither is the owner's word: a swarm keeps its
    // box, and a plain box keeps its record (the retained kill, as before).
    for mode in [MemberFinalizeMode::ParentKill, MemberFinalizeMode::Exit] {
        for observed in unclosed_swarms() {
            let port = Port::observing(observed.clone());
            let (registry, env_ref) = one_child_env();

            finalize(&port, mode, None, &registry, &env_ref);

            assert_eq!(port.kills(), 0, "{mode:?} on {observed:?}");
            assert!(port.cleanups().is_empty(), "{mode:?} on {observed:?}");
            assert_eq!(
                registry.get(&env_ref).unwrap().status,
                EnvironmentStatus::Retained,
                "{mode:?} on {observed:?}"
            );
        }
        let port = Port::observing(SwarmRunObservation::NoStore);
        let (registry, env_ref) = one_child_env();
        finalize(&port, mode, None, &registry, &env_ref);
        assert!(port.cleanups().is_empty(), "{mode:?}: no cleanup");
        assert_eq!(port.kills(), 1, "{mode:?}");
        assert_eq!(
            registry.get(&env_ref).unwrap().status,
            EnvironmentStatus::Stopped,
            "{mode:?}: the record stays"
        );
    }
}

#[test]
fn a_member_that_does_not_empty_the_environment_ends_nothing() {
    let port = Port::observing(SwarmRunObservation::NoStore);
    let registry = EnvironmentRegistry::new();
    let env_ref = committed_env(&registry, vec!["child-uuid", "sibling-uuid"]);

    finalize(
        &port,
        MemberFinalizeMode::OwnerEnd,
        None,
        &registry,
        &env_ref,
    );

    assert!(port.cleanups().is_empty());
    assert_eq!(port.kills(), 0);
    let record = registry.get(&env_ref).unwrap();
    assert_eq!(record.status, EnvironmentStatus::Running);
    assert_eq!(record.members, vec!["sibling-uuid".to_string()]);
}
