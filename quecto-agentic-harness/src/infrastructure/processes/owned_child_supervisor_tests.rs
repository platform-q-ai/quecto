use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::*;

fn sleeper(seconds: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("sleep");
    cmd.arg(seconds);
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());
    cmd
}

/// A `sleep` that ignores TERM, so only KILL ends it.
fn stubborn_sleeper() -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("sh");
    cmd.arg("-c").arg("trap '' TERM; sleep 30 & wait $!");
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());
    cmd
}

/// Spawn through the supervisor and return the handle and display pid.
async fn adopt(
    supervisor: &Arc<OwnedChildSupervisor>,
    command: tokio::process::Command,
    group: ProcessGroup,
) -> (ChildHandleId, DisplayPid) {
    let spawned = supervisor.spawn(command, group).await.expect("spawn");
    (spawned.handle, spawned.display_pid)
}

fn fast() -> TerminationBudget {
    TerminationBudget {
        exit_after_ack: Duration::from_millis(300),
        term_grace: Duration::from_millis(500),
        kill_grace: Duration::from_secs(2),
    }
}

async fn negative() -> ProtocolOutcome {
    ProtocolOutcome::Negative("unreachable".into())
}

#[tokio::test]
async fn term_then_wait_then_kill_only_after_a_negative_protocol_outcome() {
    let supervisor = Arc::new(OwnedChildSupervisor::new());
    let (id, DisplayPid(pid)) = adopt(&supervisor, sleeper("30"), ProcessGroup::Inherited).await;
    assert!(pid > 0);
    assert!(supervisor.retains(id));
    let seen_before_protocol = Arc::new(AtomicBool::new(false));
    let probe = {
        let supervisor = supervisor.clone();
        let seen = seen_before_protocol.clone();
        async move {
            seen.store(supervisor.signals_sent(id).is_empty(), Ordering::SeqCst);
            ProtocolOutcome::Negative("no socket".into())
        }
    };
    let outcome = supervisor.terminate(id, probe, fast()).await;
    assert!(
        seen_before_protocol.load(Ordering::SeqCst),
        "no signal may precede the protocol attempt"
    );
    assert_eq!(
        outcome,
        TerminationOutcome::ExitedAfterTerm {
            negative: "no socket".into(),
            exit: ChildExit::Signal(libc::SIGTERM),
        }
    );
    assert_eq!(supervisor.signals_sent(id), [SentSignal::Term]);
    assert_eq!(
        supervisor.fallback_authorised_by(id).as_deref(),
        Some("no socket")
    );
    assert!(!supervisor.retains(id));
    assert!(supervisor.knows(id));
}

#[tokio::test]
async fn kill_follows_term_when_the_child_ignores_it() {
    let supervisor = Arc::new(OwnedChildSupervisor::new());
    let (id, _) = adopt(&supervisor, stubborn_sleeper(), ProcessGroup::Inherited).await;
    // Give the shell a moment to install its trap.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let outcome = supervisor.terminate(id, negative(), fast()).await;
    assert!(
        matches!(
            outcome,
            TerminationOutcome::ExitedAfterKill {
                exit: ChildExit::Signal(libc::SIGKILL),
                ..
            }
        ),
        "{outcome:?}"
    );
    assert_eq!(
        supervisor.signals_sent(id),
        [SentSignal::Term, SentSignal::Kill]
    );
}

#[tokio::test]
async fn an_acknowledged_child_that_exits_is_never_signalled() {
    let supervisor = Arc::new(OwnedChildSupervisor::new());
    let (id, _) = adopt(&supervisor, sleeper("0.2"), ProcessGroup::Inherited).await;
    let outcome = supervisor
        .terminate(
            id,
            async { ProtocolOutcome::Acknowledged },
            TerminationBudget {
                exit_after_ack: Duration::from_secs(5),
                ..fast()
            },
        )
        .await;
    assert_eq!(
        outcome,
        TerminationOutcome::ExitedAfterProtocol(ChildExit::Code(0))
    );
    assert!(supervisor.signals_sent(id).is_empty());
    assert_eq!(supervisor.fallback_authorised_by(id), None);
}

#[tokio::test]
async fn an_acknowledged_child_that_lingers_is_terminated_after_the_exit_deadline() {
    let supervisor = Arc::new(OwnedChildSupervisor::new());
    let (id, _) = adopt(&supervisor, sleeper("30"), ProcessGroup::Inherited).await;
    let outcome = supervisor
        .terminate(id, async { ProtocolOutcome::Acknowledged }, fast())
        .await;
    match outcome {
        TerminationOutcome::ExitedAfterTerm { negative, exit } => {
            assert!(
                negative.contains("acknowledged but not exited"),
                "{negative}"
            );
            assert_eq!(exit, ChildExit::Signal(libc::SIGTERM));
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn already_exited_children_are_reaped_once_and_never_signalled() {
    let supervisor = Arc::new(OwnedChildSupervisor::new());
    let (id, _) = adopt(&supervisor, sleeper("0"), ProcessGroup::Inherited).await;
    let exit = supervisor.wait_exit(id).await;
    assert_eq!(exit, Some(ChildExit::Code(0)));
    assert!(!supervisor.retains(id));
    let outcome = supervisor.terminate(id, negative(), fast()).await;
    assert_eq!(
        outcome,
        TerminationOutcome::AlreadyExited(ChildExit::Code(0))
    );
    assert!(supervisor.signals_sent(id).is_empty());
    // A second wait observes the same single reap.
    assert_eq!(supervisor.wait_exit(id).await, Some(ChildExit::Code(0)));
}

#[tokio::test]
async fn a_child_exiting_during_the_protocol_attempt_is_not_signalled() {
    let supervisor = Arc::new(OwnedChildSupervisor::new());
    let (id, _) = adopt(&supervisor, sleeper("0.1"), ProcessGroup::Inherited).await;
    let protocol = {
        let supervisor = supervisor.clone();
        async move {
            supervisor.wait_exit(id).await;
            ProtocolOutcome::Negative("gone".into())
        }
    };
    let outcome = supervisor.terminate(id, protocol, fast()).await;
    assert_eq!(
        outcome,
        TerminationOutcome::AlreadyExited(ChildExit::Code(0))
    );
    assert!(supervisor.signals_sent(id).is_empty());
}

#[tokio::test]
async fn refuses_to_signal_without_a_retained_handle() {
    let supervisor = Arc::new(OwnedChildSupervisor::new());
    let ghost = ChildHandleId::probe(99);
    let outcome = supervisor.terminate(ghost, negative(), fast()).await;
    assert_eq!(outcome, TerminationOutcome::NoRetainedHandle);
    assert!(supervisor.signals_sent(ghost).is_empty());
    assert!(!supervisor.retains(ghost));
    assert_eq!(supervisor.wait_exit(ghost).await, None);
    assert!(supervisor.exit_receiver(ghost).is_none());
    // A detached request for a ghost is a no-op too.
    supervisor.request_termination(ghost, Box::pin(negative()), fast());
    assert!(supervisor.handles().is_empty());
}

#[tokio::test]
async fn cancelled_and_concurrent_terminations_signal_at_most_once_per_kind() {
    let supervisor = Arc::new(OwnedChildSupervisor::new());
    let (id, _) = adopt(&supervisor, stubborn_sleeper(), ProcessGroup::Inherited).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    // First attempt: cancelled right after TERM was sent.
    let first = {
        let supervisor = supervisor.clone();
        tokio::spawn(async move {
            supervisor
                .terminate(
                    id,
                    negative(),
                    TerminationBudget {
                        term_grace: Duration::from_secs(30),
                        ..fast()
                    },
                )
                .await
        })
    };
    while supervisor.signals_sent(id).is_empty() {
        tokio::task::yield_now().await;
    }
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    // Two concurrent second attempts: TERM is not repeated, KILL happens once.
    let a = {
        let supervisor = supervisor.clone();
        tokio::spawn(async move { supervisor.terminate(id, negative(), fast()).await })
    };
    let b = {
        let supervisor = supervisor.clone();
        tokio::spawn(async move { supervisor.terminate(id, negative(), fast()).await })
    };
    let (a, b) = (a.await.unwrap(), b.await.unwrap());
    for outcome in [&a, &b] {
        assert!(
            matches!(
                outcome,
                TerminationOutcome::ExitedAfterKill { .. } | TerminationOutcome::AlreadyExited(_)
            ),
            "{outcome:?}"
        );
    }
    assert_eq!(
        supervisor.signals_sent(id),
        [SentSignal::Term, SentSignal::Kill]
    );
    assert_eq!(
        supervisor.fallback_authorised_by(id).as_deref(),
        Some("unreachable")
    );
}

#[tokio::test]
async fn own_process_group_signals_the_whole_group() {
    let supervisor = Arc::new(OwnedChildSupervisor::new());
    let mut cmd = tokio::process::Command::new("sh");
    // The grandchild sleeps in the same (new) group; TERM to the group ends both.
    cmd.arg("-c").arg("sleep 30 & wait $!");
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());
    cmd.process_group(0);
    let (id, DisplayPid(pid)) = adopt(&supervisor, cmd, ProcessGroup::Own).await;
    let outcome = supervisor.terminate(id, negative(), fast()).await;
    assert!(
        matches!(outcome, TerminationOutcome::ExitedAfterTerm { .. }),
        "{outcome:?}"
    );
    // The group is gone: probing it fails.
    tokio::time::sleep(Duration::from_millis(200)).await;
    // SAFETY: signal 0 only probes the group this test created.
    let alive = unsafe { libc::kill(-(pid as i32), 0) } == 0;
    assert!(!alive, "process group must be gone after TERM");
}

#[tokio::test]
async fn dry_run_records_without_dispatching() {
    let supervisor = Arc::new(OwnedChildSupervisor::new());
    supervisor.set_dry_run(true);
    let (id, _) = adopt(&supervisor, sleeper("30"), ProcessGroup::Inherited).await;
    let outcome = supervisor
        .terminate(
            id,
            negative(),
            TerminationBudget {
                exit_after_ack: Duration::from_millis(10),
                term_grace: Duration::from_millis(50),
                kill_grace: Duration::from_millis(50),
            },
        )
        .await;
    assert_eq!(
        outcome,
        TerminationOutcome::StillRunning {
            negative: "unreachable".into()
        }
    );
    assert_eq!(
        supervisor.signals_sent(id),
        [SentSignal::Term, SentSignal::Kill]
    );
    assert!(supervisor.retains(id), "nothing was actually signalled");
    supervisor.set_dry_run(false);
    let outcome = supervisor.terminate(id, negative(), fast()).await;
    // Both kinds were already recorded: no real signal can be sent now, so
    // the child keeps running and the caller learns that honestly.
    assert!(matches!(outcome, TerminationOutcome::StillRunning { .. }));
    // Clean up the real sleeper directly through its group-less pid path.
    let (cleanup, _) = adopt(&supervisor, sleeper("0"), ProcessGroup::Inherited).await;
    supervisor.wait_exit(cleanup).await;
}

#[tokio::test]
async fn detached_request_runs_inside_and_outside_a_runtime() {
    let supervisor = Arc::new(OwnedChildSupervisor::new());
    let (inside, _) = adopt(&supervisor, sleeper("30"), ProcessGroup::Inherited).await;
    supervisor.request_termination(inside, Box::pin(negative()), fast());
    assert_eq!(
        supervisor.wait_exit(inside).await,
        Some(ChildExit::Signal(libc::SIGTERM))
    );
    let (outside, _) = adopt(&supervisor, sleeper("30"), ProcessGroup::Inherited).await;
    let from_thread = supervisor.clone();
    std::thread::spawn(move || {
        from_thread.request_termination(outside, Box::pin(negative()), fast());
    })
    .join()
    .unwrap();
    assert_eq!(
        supervisor.wait_exit(outside).await,
        Some(ChildExit::Signal(libc::SIGTERM))
    );
    let mut handles = supervisor.handles();
    handles.sort();
    assert_eq!(handles, [inside, outside]);
    assert_eq!(format!("{inside:?}"), "ChildHandle#1");
}

#[test]
fn exit_status_translation_and_defaults() {
    assert_eq!(TerminationBudget::default(), TerminationBudget::DEFAULT);
    let unobservable = ChildExit::from_status(Err(std::io::Error::other("waitpid failed")));
    assert_eq!(
        unobservable,
        ChildExit::Unobservable("waitpid failed".into())
    );
    use std::os::unix::process::ExitStatusExt;
    assert_eq!(
        ChildExit::from_status(Ok(std::process::ExitStatus::from_raw(3 << 8))),
        ChildExit::Code(3)
    );
    assert_eq!(
        ChildExit::from_status(Ok(std::process::ExitStatus::from_raw(9))),
        ChildExit::Signal(9)
    );
    // Refused targets: zero and negative pids never reach the kernel.
    send_signal(0, ProcessGroup::Own, SentSignal::Term);
    send_signal(u32::MAX, ProcessGroup::Inherited, SentSignal::Kill);
}

/// The `slot.reaped` guard in `dispatch`: once reaped, a direct dispatch
/// sends nothing and records nothing, whatever the flags say.
#[tokio::test]
async fn dispatch_never_signals_a_reaped_handle() {
    let supervisor = Arc::new(OwnedChildSupervisor::new());
    let (id, _) = adopt(&supervisor, sleeper("0"), ProcessGroup::Inherited).await;
    assert_eq!(supervisor.wait_exit(id).await, Some(ChildExit::Code(0)));
    supervisor.dispatch(id, SentSignal::Term, "late");
    supervisor.dispatch(id, SentSignal::Kill, "late");
    assert!(supervisor.signals_sent(id).is_empty());
    assert_eq!(supervisor.fallback_authorised_by(id), None);
    // An unknown handle is refused the same way.
    supervisor.dispatch(
        ChildHandleId::probe(u64::MAX - 1),
        SentSignal::Term,
        "ghost",
    );
    assert!(
        supervisor
            .signals_sent(ChildHandleId::probe(u64::MAX - 1))
            .is_empty()
    );
}

/// Slots are retired once their exit was observed; the table never grows
/// with the number of processes ever spawned, and a retired handle still
/// answers from its bounded record.
#[tokio::test]
async fn retired_slots_do_not_accumulate() {
    let supervisor = Arc::new(OwnedChildSupervisor::new());
    let mut last = None;
    for _ in 0..12 {
        // A child that exits only when told, so the unreaped case is not a
        // race against a fast exit.
        let mut told = tokio::process::Command::new("sh");
        told.arg("-c").arg("read _; exit 0");
        told.stdin(std::process::Stdio::piped());
        told.stdout(std::process::Stdio::null());
        told.stderr(std::process::Stdio::null());
        let spawned = supervisor
            .spawn(told, ProcessGroup::Inherited)
            .await
            .expect("spawn");
        let id = spawned.handle;
        assert!(
            !supervisor.retire(id),
            "an unreaped handle is never retired"
        );
        drop(spawned.stdin);
        supervisor.wait_exit(id).await;
        assert!(supervisor.retire(id));
        assert!(!supervisor.retire(id), "retiring twice is inert");
        assert!(!supervisor.knows(id));
        assert!(!supervisor.retains(id));
        assert!(supervisor.signals_sent(id).is_empty());
        last = Some(id);
    }
    assert_eq!(supervisor.slot_count(), 0);
    assert!(supervisor.handles().is_empty());
    // A terminated-then-retired handle keeps its signal record.
    let (id, _) = adopt(&supervisor, sleeper("30"), ProcessGroup::Inherited).await;
    supervisor.terminate(id, negative(), fast()).await;
    assert!(supervisor.retire(id));
    assert_eq!(supervisor.signals_sent(id), [SentSignal::Term]);
    assert_eq!(
        supervisor.fallback_authorised_by(id).as_deref(),
        Some("unreachable")
    );
    assert_eq!(
        supervisor.terminate(id, negative(), fast()).await,
        TerminationOutcome::NoRetainedHandle
    );
    assert_ne!(last, Some(id));
    // The process-wide supervisor is one shared instance.
    assert!(Arc::ptr_eq(
        &OwnedChildSupervisor::process_wide(),
        &OwnedChildSupervisor::process_wide()
    ));
}

/// Reviewer's throwaway (#1946 round two): the slot is retired by its
/// owner while a termination is parked in its protocol phase. The outcome
/// must reflect the recorded exit — never a fabricated "still running" —
/// and no signal is sent.
#[tokio::test]
async fn a_slot_retired_during_the_protocol_phase_reports_its_recorded_exit() {
    for (acknowledged, expected) in [
        (
            true,
            TerminationOutcome::ExitedAfterProtocol(ChildExit::Code(0)),
        ),
        (false, TerminationOutcome::AlreadyExited(ChildExit::Code(0))),
    ] {
        let supervisor = Arc::new(OwnedChildSupervisor::new());
        let (id, _) = adopt(&supervisor, sleeper("0"), ProcessGroup::Inherited).await;
        let protocol = {
            let supervisor = supervisor.clone();
            async move {
                // The reaper's path: exit observed, then the slot retired.
                supervisor.wait_exit(id).await;
                assert!(supervisor.retire(id));
                if acknowledged {
                    ProtocolOutcome::Acknowledged
                } else {
                    ProtocolOutcome::Negative("late".into())
                }
            }
        };
        let outcome = supervisor.terminate(id, protocol, fast()).await;
        assert_eq!(outcome, expected);
        assert!(supervisor.signals_sent(id).is_empty());
        assert!(!supervisor.knows(id));
    }
}

/// Retired after TERM was sent: the record keeps the classification.
#[tokio::test]
async fn a_slot_retired_after_term_keeps_its_term_classification() {
    let supervisor = Arc::new(OwnedChildSupervisor::new());
    let (id, _) = adopt(&supervisor, sleeper("30"), ProcessGroup::Inherited).await;
    let first = {
        let supervisor = supervisor.clone();
        tokio::spawn(async move {
            supervisor
                .terminate(
                    id,
                    negative(),
                    TerminationBudget {
                        term_grace: Duration::from_secs(30),
                        ..fast()
                    },
                )
                .await
        })
    };
    while supervisor.signals_sent(id).is_empty() {
        tokio::task::yield_now().await;
    }
    // A concurrent owner observes the exit and retires the slot.
    supervisor.wait_exit(id).await;
    assert!(supervisor.retire(id));
    let outcome = first.await.unwrap();
    assert_eq!(
        outcome,
        TerminationOutcome::ExitedAfterTerm {
            negative: "unreachable".into(),
            exit: ChildExit::Signal(libc::SIGTERM),
        }
    );
    // A handle whose record has left the ring reports nothing retained.
    let probe = ChildHandleId::probe(u64::MAX - 2);
    assert_eq!(
        supervisor.terminate(probe, negative(), fast()).await,
        TerminationOutcome::NoRetainedHandle
    );
}

/// Owners that do not wait for the exit themselves retire on reap.
#[tokio::test]
async fn retire_when_reaped_removes_the_slot_after_the_exit() {
    let supervisor = Arc::new(OwnedChildSupervisor::new());
    let (id, _) = adopt(&supervisor, sleeper("0.2"), ProcessGroup::Inherited).await;
    supervisor.retire_when_reaped(id);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while supervisor.knows(id) && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(!supervisor.knows(id));
    assert_eq!(supervisor.slot_count(), 0);
    // Already reaped: retires at once; unknown: no-op.
    let (id, _) = adopt(&supervisor, sleeper("0"), ProcessGroup::Inherited).await;
    supervisor.wait_exit(id).await;
    supervisor.retire_when_reaped(id);
    assert!(!supervisor.knows(id));
    supervisor.retire_when_reaped(ChildHandleId::probe(u64::MAX - 3));
    assert_eq!(supervisor.slot_count(), 0);
}
