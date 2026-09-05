use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reaper_cannot_release_child_during_signal_dispatch() {
    let mut child = tokio::process::Command::new("true").spawn().unwrap();
    let ownership = ProcessOwnership::new();
    let signal_owner = ownership.clone();
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let dispatch = std::thread::spawn(move || {
        // Simulated slow TERM/KILL dispatch, not an OS signal or PID reuse.
        signal_owner.dispatch(|| {
            entered_tx.send(()).unwrap();
            release_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
        });
    });
    entered_rx.await.unwrap();
    assert!(
        ownership.0.try_lock().is_err(),
        "dispatch must hold the reap lease"
    );
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let reaper = tokio::spawn(async move {
        started_tx.send(()).unwrap();
        ownership.wait(&mut child).await.unwrap()
    });
    started_rx.await.unwrap();
    assert!(!reaper.is_finished(), "reaper completed inside dispatch");
    release_tx.send(()).unwrap();
    dispatch.join().unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(5), reaper)
            .await
            .unwrap()
            .unwrap()
            .success()
    );
}
