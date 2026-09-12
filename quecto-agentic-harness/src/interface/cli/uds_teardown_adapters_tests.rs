use std::sync::Arc;

use super::*;
use crate::domain::subagent_teardown::LaunchGeneration;
use crate::infrastructure::tools::subagent_registry::SubagentEntry;
use crate::interface::cli::uds_cancel::{CancelSlot, TurnControl};

#[tokio::test]
async fn turn_cancellation_marks_abort_fires_the_slot_and_reports_busy() {
    let cancel_handle: CancelHandle = Arc::new(Mutex::new(CancelSlot::Idle));
    let turn_control: TurnControlHandle = Arc::new(TurnControl::default());
    let busy: BusyFlag = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let adapter = LoopTurnCancellation {
        cancel_handle: cancel_handle.clone(),
        turn_control: turn_control.clone(),
        busy: busy.clone(),
    };
    assert!(adapter.cancel_in_flight_turn().await);
    assert!(matches!(*cancel_handle.lock().unwrap(), CancelSlot::Fired));
    assert!(turn_control.is_abort_pending());
    busy.store(false, std::sync::atomic::Ordering::SeqCst);
    assert!(!adapter.cancel_in_flight_turn().await);
}

/// Once a shutdown executes, no turn may start (#1936): the cancellation
/// closes admission before firing, and arming a turn afterwards is refused
/// like a pre-fired cancel — so an idle boundary that follows the cancelled
/// turn (a drained follow-up, a subagent-note nudge) cannot start a provider
/// call the loop would have to wait out before it sees the exit signal.
#[tokio::test]
async fn shutdown_closes_turn_admission_for_every_later_turn() {
    let cancel_handle: CancelHandle = Arc::new(Mutex::new(CancelSlot::Idle));
    let turn_control: TurnControlHandle = Arc::new(TurnControl::default());
    assert!(
        crate::interface::cli::uds_cancel::arm_swarm_cancel(&cancel_handle, &turn_control)
            .await
            .is_some(),
        "before the shutdown a turn is admitted"
    );
    crate::interface::cli::uds_cancel::disarm_cancel(&cancel_handle);
    let adapter = LoopTurnCancellation {
        cancel_handle: cancel_handle.clone(),
        turn_control: turn_control.clone(),
        busy: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };
    adapter.cancel_in_flight_turn().await;
    assert!(turn_control.is_shutting_down());
    // The consumed abort flag does not reopen admission: it is sticky.
    assert!(turn_control.take_abort());
    for _ in 0..2 {
        assert!(
            crate::interface::cli::uds_cancel::arm_swarm_cancel(&cancel_handle, &turn_control)
                .await
                .is_none(),
            "no turn starts once the shutdown executes"
        );
    }
    assert!(
        matches!(*cancel_handle.lock().unwrap(), CancelSlot::Fired),
        "the refusal never touches the slot"
    );
}

#[tokio::test]
async fn deferred_persistence_records_the_reason_and_succeeds() {
    let adapter = DeferredLoopPersistence::default();
    assert_eq!(adapter.recorded_reason(), None);
    adapter
        .persist_for_shutdown(ShutdownReason::ParentConnectionLost)
        .await
        .unwrap();
    assert_eq!(
        adapter.recorded_reason(),
        Some(ShutdownReason::ParentConnectionLost)
    );
}

#[tokio::test]
async fn exit_readiness_wakes_the_loop_notify_and_records_the_readiness() {
    let notify = Arc::new(Notify::new());
    let adapter = LoopExitReadiness::new(notify.clone());
    assert_eq!(adapter.signalled(), None);
    adapter
        .signal_exit_ready(ExitReadiness::Completed(ShutdownReason::TerminationSignal))
        .await;
    tokio::time::timeout(std::time::Duration::from_secs(1), notify.notified())
        .await
        .expect("the loop is woken");
    assert_eq!(
        adapter.signalled(),
        Some(ExitReadiness::Completed(ShutdownReason::TerminationSignal))
    );
}

#[tokio::test]
async fn spawner_and_clock_run_on_the_runtime() {
    let (tx, rx) = tokio::sync::oneshot::channel();
    TokioShutdownRunSpawner.spawn_shutdown_run(Box::pin(async move {
        let _ = tx.send(());
    }));
    rx.await.unwrap();
    let clock = MonotonicShutdownClock::default();
    let first = clock.now();
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    assert!(clock.now() >= first);
}

#[test]
fn lifecycle_repository_lists_only_launched_children_as_direct() {
    let registry: SubagentRegistry = Arc::new(Mutex::new(Default::default()));
    {
        let mut entries = registry.lock().unwrap();
        let mut launched = SubagentEntry::with_identity(
            AgentUuid::new("child-b"),
            "b".into(),
            "/tmp/b.sock".into(),
            0,
        );
        launched.launch_generation = Some(LaunchGeneration::new(4));
        entries.insert("child-b".into(), launched);
        let mut launched = SubagentEntry::with_identity(
            AgentUuid::new("child-a"),
            "a".into(),
            "/tmp/a.sock".into(),
            0,
        );
        launched.launch_generation = Some(LaunchGeneration::new(2));
        entries.insert("child-a".into(), launched);
        // A merged grandchild and a restored row carry no generation.
        let mut merged = SubagentEntry::with_identity(
            AgentUuid::new("grandchild"),
            "g".into(),
            "/tmp/g.sock".into(),
            42,
        );
        merged.parent_id = Some("child-a".into());
        entries.insert("grandchild".into(), merged);
    }
    let repository = RegistryLifecycleRepository::new(Some(registry), AgentUuid::new("me"));
    assert_eq!(repository.lifecycle(), HarnessLifecycleState::Accepting);
    repository.set_lifecycle(HarnessLifecycleState::Frozen);
    assert_eq!(repository.lifecycle(), HarnessLifecycleState::Frozen);
    let lineage = repository.lineage();
    assert_eq!(lineage.owner, AgentUuid::new("me"));
    assert_eq!(
        lineage.records,
        [
            LineageRecord {
                identity: DelegatedAgentIdentity::new("child-a", LaunchGeneration::new(2)),
                parent: AgentUuid::new("me"),
            },
            LineageRecord {
                identity: DelegatedAgentIdentity::new("child-b", LaunchGeneration::new(4)),
                parent: AgentUuid::new("me"),
            },
        ]
    );
    let without = RegistryLifecycleRepository::new(None, AgentUuid::new("solo"));
    assert!(without.lineage().records.is_empty());
}

#[tokio::test]
async fn ack_writer_frames_per_wire_mode_and_flushes() {
    use tokio::io::AsyncReadExt;
    let (ours, mut peer) = tokio::net::UnixStream::pair().unwrap();
    let (_read, write) = tokio::io::split(ours);
    let writer: SharedWriter = Arc::new(tokio::sync::Mutex::new(write));
    let response = TeardownResponse::err(Some("x"), "shutdown", "nope");
    // Legacy: one newline-terminated line.
    let legacy = ConnectionAckWriter {
        writer: writer.clone(),
        mode: ConnectionWireMode::legacy(),
    };
    legacy.write_and_flush(&response).await.unwrap();
    let mut buf = vec![0u8; 256];
    let n = peer.read(&mut buf).await.unwrap();
    let text = String::from_utf8_lossy(&buf[..n]).into_owned();
    assert!(text.ends_with('\n'));
    assert!(text.contains("\"id\":\"x\""));
    // Framed: length-prefixed, no newline.
    let mode = ConnectionWireMode::default();
    mode.record(quecto_line_io::WireMode::Framed);
    let framed = ConnectionAckWriter { writer, mode };
    framed.write_and_flush(&response).await.unwrap();
    let n = peer.read(&mut buf).await.unwrap();
    assert_ne!(buf[0], b'{', "a frame starts with its length prefix");
    assert!(!buf[..n].ends_with(b"\n"));
    // A closed peer is an ack error, once the kernel reports the reset
    // (the first write after the close may still be accepted).
    drop(peer);
    let mut failed = false;
    for _ in 0..50 {
        if framed.write_and_flush(&response).await.is_err() {
            failed = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(failed, "writing to a closed peer must fail");
}
