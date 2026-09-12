use super::*;
use crate::infrastructure::processes::owned_child_supervisor::ProcessGroup;
use crate::infrastructure::tools::subagent_registry::new_exit_signal_channel;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

async fn adopt(supervisor: &Arc<OwnedChildSupervisor>, program: &str) -> ChildHandleId {
    supervisor
        .spawn(
            tokio::process::Command::new(program),
            ProcessGroup::Inherited,
        )
        .await
        .unwrap()
        .handle
}

/// A reaped child's registry clones can never signal it: the supervisor
/// refuses without a retained handle, and no reported lease exists for a
/// locally launched child.
#[tokio::test]
async fn removed_entry_cannot_signal_after_its_reaper_finishes() {
    use crate::infrastructure::tools::subagent_registry::{SubagentEntry, new_registry};
    use crate::infrastructure::tools::{process_tree::SIGNAL_LOG, subagent_cascade};
    let registry = new_registry();
    let (exit_tx, mut exit_rx) = new_exit_signal_channel();
    let supervisor = Arc::new(OwnedChildSupervisor::new());
    let handle = adopt(&supervisor, "true").await;
    let mut entry = SubagentEntry::new("/tmp/owned.sock".into(), 4242);
    entry.owned_child = Some(handle);
    entry.owned_child_supervisor = Some(supervisor.clone());
    registry.lock().unwrap().insert("owned".into(), entry);
    spawn_reaper_task(
        handle,
        supervisor.clone(),
        registry.clone(),
        "owned".into(),
        ReaperContext {
            exit_tx,
            broadcast_tx: None,
            swarm_context: None,
        },
    );
    // Explicit cleanup may retain this clone while an asynchronous cleanup runs.
    let removed = subagent_cascade::cascade_remove(&registry, "owned");
    tokio::time::timeout(std::time::Duration::from_secs(5), exit_rx.changed())
        .await
        .unwrap()
        .unwrap();
    SIGNAL_LOG.with(|log| *log.borrow_mut() = Some(Vec::new()));
    assert!(!subagent_cascade::terminate_removed_entry(&removed[0].1));
    // A shutdown drain can likewise retain a clone after registry removal.
    registry
        .lock()
        .unwrap()
        .insert("drained".into(), removed[0].1.clone());
    super::super::spawn_registry::shutdown_all(&registry);
    let signals = SIGNAL_LOG.with(|log| log.borrow_mut().take().unwrap());
    assert!(
        signals.is_empty(),
        "stale cleanup dispatched after reap: {signals:?}"
    );
    assert!(supervisor.signals_sent(handle).is_empty());
    assert!(!supervisor.retains(handle));
}

#[test]
fn exit_signal_from_exit_maps_code_signal_and_unobservable() {
    let missing = exit_signal_from_exit(None);
    assert_eq!(missing.exit_code, None);
    assert_eq!(missing.signal, None);
    let unobservable = exit_signal_from_exit(Some(ChildExit::Unobservable("wait failed".into())));
    assert_eq!(unobservable.exit_code, None);
    assert_eq!(unobservable.signal, None);
    let clean = exit_signal_from_exit(Some(ChildExit::Code(0)));
    assert_eq!(clean.exit_code, Some(0));
    assert_eq!(clean.signal, None);
    let signalled = exit_signal_from_exit(Some(ChildExit::Signal(15)));
    assert_eq!(signalled.exit_code, None);
    assert_eq!(signalled.signal, Some(15));
}

#[tokio::test]
async fn reaper_task_forwards_exit_signal_for_untracked_child() {
    let registry: SubagentRegistry = Arc::new(Mutex::new(HashMap::new()));
    let (exit_tx, mut exit_rx) = new_exit_signal_channel();
    let supervisor = Arc::new(OwnedChildSupervisor::new());
    let handle = adopt(&supervisor, "true").await;

    spawn_reaper_task(
        handle,
        supervisor,
        registry.clone(),
        "gone".into(),
        ReaperContext {
            exit_tx,
            broadcast_tx: None,
            swarm_context: None,
        },
    );

    exit_rx.changed().await.unwrap();
    let signal = exit_rx.borrow().clone().expect("exit signal published");
    assert_eq!(signal.exit_code, Some(0));
    assert!(registry.lock().unwrap().is_empty());
}

/// The reaper's exit signal comes from the supervisor, never from a
/// second wait on the process.
#[tokio::test]
async fn reaper_reports_the_supervisors_exit_status() {
    let registry: SubagentRegistry = Arc::new(Mutex::new(HashMap::new()));
    let (exit_tx, mut exit_rx) = new_exit_signal_channel();
    let supervisor = Arc::new(OwnedChildSupervisor::new());
    let mut command = tokio::process::Command::new("sh");
    command.arg("-c").arg("exit 3");
    let handle = supervisor
        .spawn(command, ProcessGroup::Inherited)
        .await
        .unwrap()
        .handle;
    spawn_reaper_task(
        handle,
        supervisor.clone(),
        registry,
        "code3".into(),
        ReaperContext {
            exit_tx,
            broadcast_tx: None,
            swarm_context: None,
        },
    );
    exit_rx.changed().await.unwrap();
    assert_eq!(exit_rx.borrow().clone().unwrap().exit_code, Some(3));
    // Once the reaper has published the exit and run its cleanup it retires
    // the slot: the handle is no longer retained, and never signalled.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while supervisor.knows(handle) && std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(!supervisor.knows(handle), "the reaper retires the slot");
    assert!(supervisor.signals_sent(handle).is_empty());
}
