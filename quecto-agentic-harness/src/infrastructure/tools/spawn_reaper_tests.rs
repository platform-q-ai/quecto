use super::*;
use crate::infrastructure::tools::subagent_registry::new_exit_signal_channel;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

// Dispatch is intercepted: this models numeric reassignment after reaping, not
// real kernel PID reuse. No post-reap signal is ever sent to the OS.
#[tokio::test]
async fn removed_entry_cannot_signal_after_its_reaper_finishes() {
    use crate::infrastructure::tools::subagent_registry::{SubagentEntry, new_registry};
    use crate::infrastructure::tools::{process_tree::SIGNAL_LOG, subagent_cascade};
    let registry = new_registry();
    let (exit_tx, mut exit_rx) = new_exit_signal_channel();
    let child = tokio::process::Command::new("true").spawn().unwrap();
    let pid = child.id().unwrap();
    registry.lock().unwrap().insert(
        "owned".into(),
        SubagentEntry::new("/tmp/owned.sock".into(), pid),
    );
    let ownership = registry.lock().unwrap()["owned"].process_ownership.clone();
    spawn_reaper_task(
        child,
        registry.clone(),
        "owned".into(),
        exit_tx,
        None,
        ownership,
    );
    // Explicit cleanup may retain this clone while an asynchronous cleanup runs.
    let removed = subagent_cascade::cascade_remove(&registry, "owned");
    tokio::time::timeout(std::time::Duration::from_secs(5), exit_rx.changed())
        .await
        .unwrap()
        .unwrap();
    SIGNAL_LOG.with(|log| *log.borrow_mut() = Some(Vec::new()));
    subagent_cascade::terminate_removed_entry(&removed[0].1);
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
}

#[test]
fn exit_signal_from_status_maps_code_signal_and_error() {
    let err = exit_signal_from_status(Err(std::io::Error::other("wait failed")));
    assert_eq!(err.exit_code, None);
    assert_eq!(err.signal, None);

    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        let clean = exit_signal_from_status(Ok(std::process::ExitStatus::from_raw(0)));
        assert_eq!(clean.exit_code, Some(0));
        assert_eq!(clean.signal, None);

        // Raw wait status 15 = terminated by SIGTERM.
        let signalled = exit_signal_from_status(Ok(std::process::ExitStatus::from_raw(15)));
        assert_eq!(signalled.exit_code, None);
        assert_eq!(signalled.signal, Some(15));
    }
}

#[tokio::test]
async fn reaper_task_forwards_exit_signal_for_untracked_child() {
    let registry: SubagentRegistry = Arc::new(Mutex::new(HashMap::new()));
    let (exit_tx, mut exit_rx) = new_exit_signal_channel();
    let child = tokio::process::Command::new("true").spawn().unwrap();

    spawn_reaper_task(
        child,
        registry.clone(),
        "gone".into(),
        exit_tx,
        None,
        super::super::process_ownership::ProcessOwnership::new(),
    );

    exit_rx.changed().await.unwrap();
    let signal = exit_rx.borrow().clone().expect("exit signal published");
    assert_eq!(signal.exit_code, Some(0));
    assert!(registry.lock().unwrap().is_empty());
}
