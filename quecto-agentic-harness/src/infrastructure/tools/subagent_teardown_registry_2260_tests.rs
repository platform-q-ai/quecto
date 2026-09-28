//! #2260: a child this harness held is announced with the exit status it
//! was reaped with, even when its end is observed before the reaper task
//! publishes that status (the monitor's EOF wins the terminal claim).
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::end_detail_tests::{REASON, UUID, crash, signal};
use super::*;
use crate::application::subagents::dto::{ObserveOwnedChildExitRequest, ObservedExit};
use crate::application::subagents::use_cases::ObserveOwnedChildExit;
use crate::domain::ids::AgentUuid;
use crate::domain::subagent_teardown::LaunchGeneration;
use crate::infrastructure::processes::owned_child_supervisor::{
    ChildHandleId, OwnedChildSupervisor, ProcessGroup,
};
use crate::infrastructure::tools::subagent_registry::{
    ExitSignalTx, NotificationRx, new_exit_signal_channel, new_notification_channel, new_registry,
};

/// How long a note built with nothing to wait for may take: generous for a
/// loaded CI host, far below any wait the note could have made.
const NO_WAIT: Duration = Duration::from_secs(1);

/// How a test's reaped child's note reads: it was killed by SIGKILL (so
/// nothing dumps core), and a panic record would not fit that end, so none
/// is left.
const KILLED: &str = "ended unexpectedly (signal 9) (connection_closed)";

/// The owned child a row names: the probe handle of a row whose supervisor
/// is not on the row (so no record can be read and only the reaper's
/// publish is left), or a real handle and the supervisor that reaped it.
enum Owned {
    Unrecorded,
    Reaped(ChildHandleId, Arc<OwnedChildSupervisor>),
}

struct Rig {
    port: Arc<RegistryDelegatedAgents>,
    notify_rx: NotificationRx,
    /// The reaper's side of the exit signal; nothing is published on it
    /// unless a test does.
    exit_tx: ExitSignalTx,
    target: DelegatedAgentIdentity,
    phases: tokio::sync::watch::Receiver<TeardownPhase>,
    monitor: Arc<tokio::task::JoinHandle<()>>,
}

fn rig(base: &std::path::Path, owned: Owned, wait: Option<Duration>) -> Rig {
    let registry = new_registry();
    let (exit_tx, _exit_rx) = new_exit_signal_channel();
    let mut entry = SubagentEntry::with_identity(
        AgentUuid::new(UUID),
        "alpha".into(),
        PathBuf::from("/tmp/a.sock"),
        // Launched by this harness as pid 7: the pid its crash record names.
        7,
    );
    entry.origin = crate::domain::child_end::ChildOrigin::Launched;
    entry.launch_generation = Some(LaunchGeneration::new(1));
    entry.exit_signal_tx = Some(exit_tx.clone());
    match owned {
        Owned::Unrecorded => entry.owned_child = Some(ChildHandleId::probe(1)),
        Owned::Reaped(handle, supervisor) => {
            entry.owned_child = Some(handle);
            entry.owned_child_supervisor = Some(supervisor);
        }
    }
    let monitor = Arc::new(tokio::spawn(std::future::pending::<()>()));
    entry.monitor_handle = Some(Arc::clone(&monitor));
    let phases = entry.teardown.subscribe();
    registry.lock().unwrap().insert(UUID.into(), entry);
    let (notify_tx, notify_rx) = new_notification_channel();
    let port = RegistryDelegatedAgents::new(
        registry,
        None,
        Some(notify_tx),
        crate::composition::environments::build_member_finalizer,
    )
    .with_ended_child(Some(
        crate::composition::subagent_lifecycle::build_ended_child_inspection(base),
    ));
    let port = match wait {
        Some(wait) => port.with_exit_status_wait(wait),
        None => port,
    };
    Rig {
        port: Arc::new(port),
        notify_rx,
        exit_tx,
        target: DelegatedAgentIdentity::new(UUID, LaunchGeneration::new(1)),
        phases,
        monitor,
    }
}

/// Claim the row's end as the monitor's EOF would, and compensate it.
async fn compensate_closed(rig: &Rig) {
    assert_eq!(rig.port.claim_terminal(&rig.target), TerminalClaim::Claimed);
    rig.port
        .compensate(
            &rig.target,
            TerminationCause::Exit(ExitObservation::ConnectionClosed),
        )
        .await;
}

fn exited_detail(rig: &mut Rig) -> Option<String> {
    let note = rig
        .notify_rx
        .try_recv()
        .expect("an exited note")
        .notification;
    match note {
        SubagentNotification::Exited { detail, .. } => detail,
        other => panic!("{other:?}"),
    }
}

/// A real child the supervisor has reaped — killed by SIGKILL, so nothing
/// dumps core — and whose reaper task has not run: nothing is published on
/// the row's exit signal and the handle is not retired.
async fn reaped_child() -> (ChildHandleId, Arc<OwnedChildSupervisor>) {
    let supervisor = Arc::new(OwnedChildSupervisor::new());
    let mut command = tokio::process::Command::new("sh");
    command.arg("-c").arg("kill -KILL $$");
    command.stdin(std::process::Stdio::null());
    command.stdout(std::process::Stdio::null());
    command.stderr(std::process::Stdio::null());
    let spawned = supervisor
        .spawn(command, ProcessGroup::Inherited)
        .await
        .expect("spawn");
    supervisor.wait_exit(spawned.handle).await.expect("reaped");
    assert!(!supervisor.retains(spawned.handle), "the child was reaped");
    (spawned.handle, supervisor)
}

/// #2260 (review round 1, F1): the supervisor records the exit in the same
/// critical section that ends its hold, so a row whose child it reaped is
/// read from the supervisor at once — no wait for the reaper's publish.
#[tokio::test]
async fn a_reaped_childs_status_is_read_from_the_supervisor_without_waiting() {
    let base = tempfile::tempdir().unwrap();
    let (handle, supervisor) = reaped_child().await;
    let mut rig = rig(base.path(), Owned::Reaped(handle, supervisor), None);
    let started = Instant::now();
    compensate_closed(&rig).await;
    assert!(
        started.elapsed() < NO_WAIT,
        "the note waited: {:?}",
        started.elapsed()
    );
    assert_eq!(exited_detail(&mut rig).as_deref(), Some(KILLED));
    assert_eq!(*rig.exit_tx.borrow(), None, "nothing was published");
}

/// #2260 (review round 1, F1): the reaper retires the handle once its own
/// work is done; the retired record still answers.
#[tokio::test]
async fn a_retired_childs_status_is_read_from_its_record_without_waiting() {
    let base = tempfile::tempdir().unwrap();
    let (handle, supervisor) = reaped_child().await;
    assert!(supervisor.retire(handle));
    let mut rig = rig(base.path(), Owned::Reaped(handle, supervisor), None);
    let started = Instant::now();
    compensate_closed(&rig).await;
    assert!(started.elapsed() < NO_WAIT, "{:?}", started.elapsed());
    assert_eq!(exited_detail(&mut rig).as_deref(), Some(KILLED));
}

/// #2260 (review round 1, F4): the real race at the use case. The
/// supervisor has reaped the child, so the row no longer holds its process
/// and the monitor's EOF is not deferred; the reaper has not published.
/// ObserveOwnedChildExit claims first, and the note still names the crash.
#[tokio::test]
async fn the_monitor_claiming_before_the_reaper_publishes_still_names_the_crash() {
    let base = tempfile::tempdir().unwrap();
    let (handle, supervisor) = reaped_child().await;
    let mut rig = rig(base.path(), Owned::Reaped(handle, supervisor), None);
    assert!(!rig.port.holds_process(&rig.target));
    let observe = ObserveOwnedChildExit::new(rig.port.clone(), rig.port.clone());
    let started = Instant::now();
    let observed = observe
        .execute(ObserveOwnedChildExitRequest {
            child: rig.target.clone(),
            observation: ExitObservation::ConnectionClosed,
        })
        .await;
    assert!(started.elapsed() < NO_WAIT, "{:?}", started.elapsed());
    assert_eq!(
        observed,
        ObservedExit::Compensated {
            removed: vec![AgentUuid::new(UUID)]
        }
    );
    assert_eq!(exited_detail(&mut rig).as_deref(), Some(KILLED));
}

/// #2260: with no supervisor record to read, a crashed owned child's note
/// waits for a reaper that publishes late (here half a second, well inside
/// the default bound).
#[tokio::test]
async fn without_a_record_the_note_waits_for_a_late_reaper() {
    let base = tempfile::tempdir().unwrap();
    crash(base.path());
    let mut rig = rig(base.path(), Owned::Unrecorded, None);
    let exit_tx = rig.exit_tx.clone();
    let reaper = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        exit_tx.send_replace(signal(6));
    });
    compensate_closed(&rig).await;
    reaper.abort();
    assert_eq!(
        exited_detail(&mut rig).as_deref(),
        Some(format!("{REASON} (connection_closed)").as_str())
    );
}

/// #2260 (review round 1, F3): the fallback wait is bounded by the
/// configured bound — not the default — and the note then says plainly that
/// no exit status was observed.
#[tokio::test]
async fn a_reaper_that_never_publishes_is_waited_for_only_up_to_the_bound() {
    let base = tempfile::tempdir().unwrap();
    let bound = Duration::from_millis(300);
    let margin = Duration::from_millis(1_500);
    assert!(bound + margin < DEFAULT_EXIT_STATUS_WAIT);
    let mut rig = rig(base.path(), Owned::Unrecorded, Some(bound));
    let started = Instant::now();
    compensate_closed(&rig).await;
    let waited = started.elapsed();
    assert!(waited >= bound, "the note did not wait: {waited:?}");
    assert!(
        waited < bound + margin,
        "the configured bound was not used: {waited:?}"
    );
    let note = rig
        .notify_rx
        .try_recv()
        .expect("an exited note")
        .notification;
    assert_eq!(
        note.to_message(),
        "Sub-agent 'alpha' ended; no exit status or crash record was observed (connection_closed)."
    );
}

/// #2260 (review round 1, F2): a note waiting for a late reaper holds back
/// neither the monitors nor the exit signals — the target's and a fallen
/// descendant's monitors are aborted, and the descendant's exit signal
/// fired, while the note still waits. The row's release stays after the
/// note, so a joiner woken on `Compensated` finds the note posted (#1953).
#[tokio::test]
async fn a_waiting_note_holds_back_neither_the_monitors_nor_the_exit_signals() {
    let base = tempfile::tempdir().unwrap();
    let bound = Duration::from_secs(3);
    let mut rig = rig(base.path(), Owned::Unrecorded, Some(bound));
    let (child_exit_tx, child_exit_rx) = new_exit_signal_channel();
    let child_monitor = Arc::new(tokio::spawn(std::future::pending::<()>()));
    {
        let mut child = SubagentEntry::with_identity(
            AgentUuid::new("child"),
            "bravo".into(),
            PathBuf::new(),
            4242,
        );
        child.reported_generation = Some(LaunchGeneration::new(1));
        child.parent_id = Some(UUID.into());
        child.exit_signal_tx = Some(child_exit_tx);
        child.monitor_handle = Some(Arc::clone(&child_monitor));
        rig.port.lock().insert("child".into(), child);
    }
    let (port, target) = (rig.port.clone(), rig.target.clone());
    assert_eq!(port.claim_terminal(&target), TerminalClaim::Claimed);
    let started = Instant::now();
    let compensation = tokio::spawn(async move {
        port.compensate(
            &target,
            TerminationCause::Exit(ExitObservation::ConnectionClosed),
        )
        .await
    });
    let aborted = tokio::time::timeout(NO_WAIT, async {
        while !(rig.monitor.is_finished() && child_monitor.is_finished()) {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert!(
        aborted.is_ok(),
        "the monitors were aborted while the note waited"
    );
    assert_eq!(
        child_exit_rx.borrow().as_ref().map(|exit| exit.kind),
        Some(ExitSignalKind::Terminated),
        "the descendant's exit signal fired while the note waited"
    );
    assert!(started.elapsed() < bound, "{:?}", started.elapsed());
    assert!(
        rig.notify_rx.try_recv().is_err(),
        "the note is still waiting for the reaper"
    );
    assert_eq!(
        *rig.phases.borrow(),
        TeardownPhase::Compensating(TeardownIntent::Exit),
        "the release follows the note"
    );
    let compensated = compensation.await.unwrap();
    assert_eq!(
        compensated.removed,
        [AgentUuid::new(UUID), AgentUuid::new("child")]
    );
    assert_eq!(*rig.phases.borrow(), TeardownPhase::Compensated);
    assert_eq!(exited_detail(&mut rig), None);
}

/// #2260 (review round 1, F7): only a child this harness held has a reaper
/// to wait for; any other row's note is built at once, as it stands.
#[tokio::test]
async fn a_row_without_an_owned_child_is_not_waited_for() {
    let base = tempfile::tempdir().unwrap();
    let mut rig = rig(base.path(), Owned::Unrecorded, None);
    {
        let mut entries = rig.port.lock();
        let entry = entries.get_mut(UUID).expect("the row");
        entry.owned_child = None;
    }
    let started = Instant::now();
    compensate_closed(&rig).await;
    assert!(
        started.elapsed() < NO_WAIT,
        "a row with no reaper waited: {:?}",
        started.elapsed()
    );
    assert_eq!(exited_detail(&mut rig), None);
}
