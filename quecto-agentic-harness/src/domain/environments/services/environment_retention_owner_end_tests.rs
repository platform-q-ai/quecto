//! #2206: the owner's word ends a plain container child's environment for
//! good — its retained cleanup runs and its record is forgotten — while
//! every swarm rule of #1924/#2070 stands untouched.

use crate::domain::environments::services::environment_retention::*;
use crate::domain::swarm::RunStatus;

fn swarm(status: RunStatus, outcome: Option<RunStatus>) -> HostedSwarmRun {
    HostedSwarmRun {
        id: "run-7".to_string(),
        status,
        outcome,
        coordinator: "member-7".to_string(),
        deadline: 4_102_444_800.0,
    }
}

fn placeholder() -> HostedSwarmRun {
    HostedSwarmRun {
        deadline: 0.0,
        ..swarm(RunStatus::Setup, None)
    }
}

/// Every mode, so a new one has to be placed on purpose.
const ALL_MODES: [MemberFinalizeMode; 6] = [
    MemberFinalizeMode::Exit,
    MemberFinalizeMode::ParentKill,
    MemberFinalizeMode::OwnerEnd,
    MemberFinalizeMode::OwnerTeardown,
    MemberFinalizeMode::LaunchRollback,
    MemberFinalizeMode::LaunchRollbackOwned,
];

/// Every swarm observation that is not a plain container.
fn swarm_observations() -> Vec<SwarmRunObservation> {
    let mut observed: Vec<SwarmRunObservation> = [
        swarm(RunStatus::Running, None),
        swarm(RunStatus::Paused, None),
        swarm(RunStatus::Paused, Some(RunStatus::Failed)),
        swarm(RunStatus::Cancelled, None),
        swarm(RunStatus::Succeeded, None),
        swarm(RunStatus::Failed, None),
    ]
    .into_iter()
    .map(SwarmRunObservation::Run)
    .collect();
    observed.push(SwarmRunObservation::Unreadable("locked".to_string()));
    observed
}

#[test]
fn only_an_owner_end_is_the_owner_ending_one_member() {
    for mode in ALL_MODES {
        let expected = matches!(mode, MemberFinalizeMode::OwnerEnd);
        assert_eq!(mode.owner_ended_member(), expected, "{mode:?}");
    }
}

#[test]
fn an_owner_end_is_inspectable_like_a_supervisor_kill() {
    let mode = MemberFinalizeMode::OwnerEnd;
    assert!(
        mode.inspectable_end(),
        "a swarm it ends is judged as a kill"
    );
    assert!(!mode.launch_rollback());
}

#[test]
fn a_plain_container_is_no_store_or_the_bootstrap_placeholder() {
    assert!(SwarmRunObservation::NoStore.plain_container());
    assert!(SwarmRunObservation::Run(placeholder()).plain_container());
    for observed in swarm_observations() {
        assert!(!observed.plain_container(), "{observed:?}");
    }
}

#[test]
fn the_owner_ending_a_plain_child_ends_its_box_for_good() {
    let mode = MemberFinalizeMode::OwnerEnd;
    for observed in [
        SwarmRunObservation::NoStore,
        SwarmRunObservation::Run(placeholder()),
    ] {
        assert!(ends_plain_environment(mode, &observed), "{observed:?}");
        assert!(!retains_environment(mode, &observed), "{observed:?}");
    }
}

#[test]
fn nothing_but_the_owner_ending_the_child_ends_a_plain_box_for_good() {
    // A child's own exit (a crash looks the same), a harness shutdown (it
    // may be a crash), the owner's teardown of everything (#2070: the
    // retained kill, no store read), an environment kill and a rollback
    // keep their own contracts.
    for mode in [
        MemberFinalizeMode::Exit,
        MemberFinalizeMode::ParentKill,
        MemberFinalizeMode::OwnerTeardown,
        MemberFinalizeMode::LaunchRollback,
        MemberFinalizeMode::LaunchRollbackOwned,
    ] {
        for observed in [
            SwarmRunObservation::NoStore,
            SwarmRunObservation::Run(placeholder()),
        ] {
            assert!(!ends_plain_environment(mode, &observed), "{mode:?}");
        }
    }
}

#[test]
fn a_swarm_or_an_unreadable_store_is_never_ended_for_good() {
    for mode in ALL_MODES {
        for observed in swarm_observations() {
            assert!(
                !ends_plain_environment(mode, &observed),
                "{mode:?} on {observed:?}"
            );
        }
    }
}

#[test]
fn an_owner_end_keeps_a_swarm_exactly_as_a_supervisor_kill_does() {
    for observed in swarm_observations() {
        assert_eq!(
            retains_environment(MemberFinalizeMode::OwnerEnd, &observed),
            retains_environment(MemberFinalizeMode::ParentKill, &observed),
            "{observed:?}"
        );
        assert_eq!(
            retention_reason(MemberFinalizeMode::OwnerEnd, &observed),
            retention_reason(MemberFinalizeMode::ParentKill, &observed),
            "{observed:?}"
        );
    }
    assert!(retains_environment(
        MemberFinalizeMode::OwnerEnd,
        &SwarmRunObservation::Run(swarm(RunStatus::Running, None))
    ));
    assert!(retains_environment(
        MemberFinalizeMode::OwnerEnd,
        &SwarmRunObservation::Unreadable("locked".to_string())
    ));
}

#[test]
fn an_unverified_no_store_is_never_plain_and_kept_only_on_the_owners_end() {
    let observed = SwarmRunObservation::NoStoreUnverified;
    assert!(!observed.plain_container());
    for mode in ALL_MODES {
        assert!(!ends_plain_environment(mode, &observed), "{mode:?}");
        assert_eq!(
            retains_environment(mode, &observed),
            mode == MemberFinalizeMode::OwnerEnd,
            "{mode:?}: kept on the owner's end, the retained kill otherwise"
        );
    }
    let reason = retention_reason(MemberFinalizeMode::OwnerEnd, &observed);
    assert!(reason.contains("cannot be checked"), "{reason}");
    assert!(reason.ends_with("kill_container to remove"), "{reason}");
}
