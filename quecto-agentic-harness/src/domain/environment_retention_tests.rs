//! Pure retention policy (#1924): which ends of an environment withhold its
//! final-member teardown and how the reason reads. No effect, no port.

use crate::domain::environment_retention::*;
use crate::domain::swarm::RunStatus;

fn running_swarm() -> HostedSwarmRun {
    HostedSwarmRun {
        id: "run-42".to_string(),
        status: RunStatus::Running,
        outcome: None,
        coordinator: "member-42".to_string(),
        deadline: 4_102_444_800.0,
    }
}

fn with_status(status: RunStatus, outcome: Option<RunStatus>) -> HostedSwarmRun {
    HostedSwarmRun {
        status,
        outcome,
        ..running_swarm()
    }
}

/// The outcomes only the supervisor outside the swarm can make terminal.
const CLOSED_BY_OWNER: [RunStatus; 4] = [
    RunStatus::Succeeded,
    RunStatus::Blocked,
    RunStatus::Failed,
    RunStatus::BudgetExhausted,
];

#[test]
fn a_run_that_has_not_ended_keeps_its_environment() {
    // #2070: a crash, a lost coordinator or an exited agent never ends a
    // swarm — the box stays so the run can be resumed. A run paused holding
    // an outcome has not been closed by its owner yet: still not ended.
    let unreadable = SwarmRunObservation::Unreadable("locked".to_string());
    for mode in [MemberFinalizeMode::Exit, MemberFinalizeMode::ParentKill] {
        for run in [
            with_status(RunStatus::Running, None),
            with_status(RunStatus::Paused, None),
            with_status(RunStatus::Paused, Some(RunStatus::Succeeded)),
            with_status(RunStatus::Paused, Some(RunStatus::Failed)),
            // The coordinator agent writes `cancelled` itself: an agent
            // never ends a swarm, so its box is kept like any other.
            with_status(RunStatus::Cancelled, None),
        ] {
            assert!(
                retains_environment(mode, &SwarmRunObservation::Run(run.clone())),
                "{mode:?} on a created run that is {} retains",
                run.describe()
            );
        }
        assert!(
            retains_environment(mode, &unreadable),
            "{mode:?}: a store that exists but cannot be read is never proof the run ended"
        );
    }
}

#[test]
fn a_run_its_owner_closed_gives_its_environment_up() {
    // #2070: the container lives as long as the swarm and no longer. Only the
    // supervisor can close a run into its outcome; then the final member's
    // end tears the box down.
    for mode in [MemberFinalizeMode::Exit, MemberFinalizeMode::ParentKill] {
        for status in CLOSED_BY_OWNER {
            assert!(with_status(status, None).closed_by_owner());
            assert!(
                !retains_environment(mode, &SwarmRunObservation::Run(with_status(status, None))),
                "{mode:?} on a {status:?} run must not retain"
            );
        }
    }
}

#[test]
fn an_owners_explicit_teardown_ends_every_swarm_it_owns() {
    // #2070: delete-all or a session transition is the owner saying "done" —
    // whatever the run's state, even an unreadable one.
    let mode = MemberFinalizeMode::OwnerTeardown;
    assert!(!mode.inspectable_end());
    assert!(
        !mode.launch_rollback(),
        "it runs the retained kill, not cleanup"
    );
    for run in [
        with_status(RunStatus::Running, None),
        with_status(RunStatus::Paused, Some(RunStatus::Failed)),
        with_status(RunStatus::Succeeded, None),
    ] {
        assert!(!retains_environment(mode, &SwarmRunObservation::Run(run)));
    }
    assert!(!retains_environment(
        mode,
        &SwarmRunObservation::Unreadable("locked".to_string())
    ));
}

#[test]
fn what_is_no_swarm_never_retains() {
    let placeholder = HostedSwarmRun {
        status: RunStatus::Setup,
        deadline: 0.0,
        ..running_swarm()
    };
    let unreadable = SwarmRunObservation::Unreadable("locked".to_string());
    for mode in [MemberFinalizeMode::Exit, MemberFinalizeMode::ParentKill] {
        assert!(
            !retains_environment(mode, &SwarmRunObservation::Run(placeholder.clone())),
            "{mode:?}: the bootstrap placeholder is no swarm"
        );
        assert!(!retains_environment(mode, &SwarmRunObservation::NoStore));
    }
    for mode in [
        MemberFinalizeMode::LaunchRollback,
        MemberFinalizeMode::LaunchRollbackOwned,
    ] {
        assert!(
            !retains_environment(mode, &SwarmRunObservation::Run(running_swarm())),
            "{mode:?} has nothing to inspect and keeps its cleanup"
        );
        assert!(!retains_environment(mode, &unreadable));
    }
}

#[test]
fn retention_reasons_name_how_the_run_ended() {
    let exit = MemberFinalizeMode::Exit;
    let text = |run| retention_reason(exit, &SwarmRunObservation::Run(run));
    assert!(text(with_status(RunStatus::Succeeded, None)).starts_with("run closed: succeeded;"));
    assert!(text(with_status(RunStatus::Failed, None)).starts_with("run closed: failed;"));
    assert!(text(with_status(RunStatus::Cancelled, None)).starts_with("run ended: cancelled;"));
    assert!(
        text(with_status(
            RunStatus::Paused,
            Some(RunStatus::BudgetExhausted)
        ))
        .starts_with("run ended: budget-exhausted;")
    );
    let killed = retention_reason(
        MemberFinalizeMode::ParentKill,
        &SwarmRunObservation::Run(with_status(RunStatus::Paused, Some(RunStatus::Succeeded))),
    );
    assert!(
        killed.starts_with("coordinator killed by supervisor; run paused holding succeeded;"),
        "{killed}"
    );
    let killed_running = retention_reason(
        MemberFinalizeMode::ParentKill,
        &SwarmRunObservation::Run(running_swarm()),
    );
    assert!(
        killed_running.starts_with("coordinator killed by supervisor; run running;"),
        "{killed_running}"
    );
    for reason in [
        text(running_swarm()),
        killed,
        retention_reason(exit, &SwarmRunObservation::Unreadable("x".into())),
    ] {
        assert!(reason.ends_with("kill_container to remove"), "{reason}");
    }
}

#[test]
fn loss_reasons_distinguish_a_recorded_loss_from_an_already_ended_run() {
    let lost = loss_reason(
        "member-42",
        &CoordinatorLoss {
            run: with_status(RunStatus::Paused, Some(RunStatus::Failed)),
            lost: true,
        },
    );
    assert!(
        lost.starts_with(
            "swarm coordinator 'member-42' lost its connection; run paused holding failed;"
        ),
        "{lost}"
    );
    let ended = loss_reason(
        "member-42",
        &CoordinatorLoss {
            run: with_status(RunStatus::Paused, Some(RunStatus::Succeeded)),
            lost: false,
        },
    );
    assert!(ended.starts_with("run ended: succeeded;"), "{ended}");
    let unrecorded = unrecorded_loss_reason(&running_swarm(), "store locked");
    assert!(
        unrecorded.contains("the loss could not be recorded (store locked)"),
        "{unrecorded}"
    );
    for reason in [lost, ended, unrecorded] {
        assert!(reason.ends_with("kill_container to remove"), "{reason}");
    }
}

#[test]
fn hosted_run_created_and_ended_follow_the_run_status() {
    assert!(running_swarm().created());
    assert!(
        !HostedSwarmRun {
            deadline: 0.0,
            ..running_swarm()
        }
        .created()
    );
    assert!(!running_swarm().ended());
    assert!(!with_status(RunStatus::Paused, None).ended());
    assert!(with_status(RunStatus::Paused, Some(RunStatus::Blocked)).ended());
    assert!(with_status(RunStatus::Cancelled, None).ended());
    assert_eq!(
        with_status(RunStatus::Paused, Some(RunStatus::Failed)).describe(),
        "paused holding failed"
    );
    assert_eq!(running_swarm().describe(), "running");
    assert!(MemberFinalizeMode::Exit.inspectable_end());
    assert!(MemberFinalizeMode::ParentKill.inspectable_end());
    assert!(!MemberFinalizeMode::LaunchRollback.inspectable_end());
    assert!(MemberFinalizeMode::LaunchRollbackOwned.launch_rollback());
    assert!(!MemberFinalizeMode::Exit.launch_rollback());
}

#[test]
fn only_the_owners_close_counts_as_closed() {
    assert!(!running_swarm().closed_by_owner());
    assert!(!with_status(RunStatus::Paused, Some(RunStatus::Succeeded)).closed_by_owner());
    assert!(
        !with_status(RunStatus::Cancelled, None).closed_by_owner(),
        "the coordinator cancels its own run"
    );
    assert!(with_status(RunStatus::Cancelled, None).keeps_environment());
    assert!(!with_status(RunStatus::Succeeded, None).keeps_environment());
    let placeholder = HostedSwarmRun {
        deadline: 0.0,
        ..running_swarm()
    };
    assert!(
        !placeholder.keeps_environment(),
        "no swarm was ever created"
    );
}
