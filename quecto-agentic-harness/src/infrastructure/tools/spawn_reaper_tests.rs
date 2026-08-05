use super::*;
use crate::infrastructure::tools::subagent_registry::new_exit_signal_channel;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

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

    spawn_reaper_task(child, registry.clone(), "gone".into(), exit_tx, None);

    exit_rx.changed().await.unwrap();
    let signal = exit_rx.borrow().clone().expect("exit signal published");
    assert_eq!(signal.exit_code, Some(0));
    assert!(registry.lock().unwrap().is_empty());
}
