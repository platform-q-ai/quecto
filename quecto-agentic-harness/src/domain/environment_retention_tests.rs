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

#[test]
fn retention_policy_holds_for_every_created_run_on_exit_and_parent_kill() {
    let run = SwarmRunObservation::Run;
    let unreadable = SwarmRunObservation::Unreadable("locked".to_string());
    let placeholder = HostedSwarmRun {
        status: RunStatus::Setup,
        deadline: 0.0,
        ..running_swarm()
    };
    for mode in [MemberFinalizeMode::Exit, MemberFinalizeMode::ParentKill] {
        for status in [
            RunStatus::Running,
            RunStatus::Paused,
            RunStatus::Succeeded,
            RunStatus::Blocked,
            RunStatus::Failed,
            RunStatus::Cancelled,
            RunStatus::BudgetExhausted,
        ] {
            assert!(
                retains_environment(mode, &run(with_status(status, None))),
                "{mode:?} on a created {status:?} run retains"
            );
        }
        assert!(
            !retains_environment(mode, &run(placeholder.clone())),
            "{mode:?}: the bootstrap placeholder is no swarm"
        );
        assert!(!retains_environment(mode, &SwarmRunObservation::NoStore));
        assert!(
            retains_environment(mode, &unreadable),
            "{mode:?}: a store that exists but cannot be read is never proof of no run"
        );
    }
    for mode in [
        MemberFinalizeMode::LaunchRollback,
        MemberFinalizeMode::LaunchRollbackOwned,
    ] {
        assert!(
            !retains_environment(mode, &run(running_swarm())),
            "{mode:?} has nothing to inspect and keeps its cleanup"
        );
        assert!(!retains_environment(mode, &unreadable));
    }
}

#[test]
fn preserving_stop_policy_requires_affirmative_created_success() {
    let run = SwarmRunObservation::Run;
    for mode in [MemberFinalizeMode::Exit, MemberFinalizeMode::ParentKill] {
        assert!(stops_runtime_preserving_workspace(
            mode,
            &run(with_status(RunStatus::Succeeded, None))
        ));
        for active_or_recoverable in [
            running_swarm(),
            with_status(RunStatus::Paused, None),
            with_status(RunStatus::Paused, Some(RunStatus::Succeeded)),
            with_status(RunStatus::Paused, Some(RunStatus::Blocked)),
            with_status(RunStatus::Failed, None),
            with_status(RunStatus::Cancelled, None),
            with_status(RunStatus::BudgetExhausted, None),
        ] {
            assert!(!stops_runtime_preserving_workspace(
                mode,
                &run(active_or_recoverable)
            ));
        }
        let placeholder = HostedSwarmRun {
            status: RunStatus::Succeeded,
            deadline: 0.0,
            ..running_swarm()
        };
        assert!(!stops_runtime_preserving_workspace(mode, &run(placeholder)));
        assert!(!stops_runtime_preserving_workspace(
            mode,
            &SwarmRunObservation::Unreadable("locked".into())
        ));
    }
    for rollback in [
        MemberFinalizeMode::LaunchRollback,
        MemberFinalizeMode::LaunchRollbackOwned,
    ] {
        assert!(!stops_runtime_preserving_workspace(
            rollback,
            &run(with_status(RunStatus::Succeeded, None))
        ));
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
