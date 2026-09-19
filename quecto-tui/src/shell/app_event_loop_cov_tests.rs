use super::tui_harness::TuiHarness;
use crate::components::component::Component;

async fn harness() -> TuiHarness {
    TuiHarness::new().await
}

#[tokio::test]
async fn files_autocomplete_lazy_load_request_is_spawned_once_and_applied() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.editor.set_text("open @");
    a.refresh_files_autocomplete_from_editor();
    assert!(a.workspace.files_autocomplete.take_load_request());

    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    let mut in_flight = false;
    a.start_files_autocomplete_load(&tx, &mut in_flight);
    assert!(in_flight, "first request should start a background load");
    a.start_files_autocomplete_load(&tx, &mut in_flight);
    assert!(in_flight, "second request while in-flight must be a no-op");
    drop(tx);

    let (root, files) = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .expect("file load worker should respond")
        .expect("worker should send a file list");
    assert!(a.apply_files_autocomplete_load(root, files));
    assert!(
        in_flight,
        "worker completion normally clears this flag in the event loop"
    );
    a.refresh_files_autocomplete_from_editor();
    assert!(!a.workspace.files_autocomplete.take_load_request());
}

#[tokio::test]
async fn app_workspace_file_autocomplete_uses_production_visible_capacity() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.editor.set_text("open @file");
    a.refresh_files_autocomplete_from_editor();
    assert!(a.workspace.files_autocomplete.take_load_request());

    a.workspace.files_autocomplete.apply_loaded_files(
        (0..20)
            .map(|i| format!("file-{i:02}.rs"))
            .collect::<Vec<_>>(),
    );
    a.refresh_files_autocomplete_from_editor();

    let rendered = a.workspace.files_autocomplete.render(80);
    let rows = rendered.join("\n");
    assert_eq!(
        rendered.len(),
        9,
        "8 visible file rows plus overflow indicator"
    );
    assert!(rows.contains("file-00.rs"));
    assert!(rows.contains("file-07.rs"));
    assert!(rows.contains("(1/20)"));
}

#[tokio::test]
async fn files_autocomplete_loaded_files_are_accepted_by_tab_completion() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.editor.set_text("open @fi");
    a.refresh_files_autocomplete_from_editor();
    assert!(a.workspace.files_autocomplete.take_load_request());

    a.workspace
        .files_autocomplete
        .apply_loaded_files(vec!["first.rs".into(), "src/other.rs".into()]);
    a.refresh_files_autocomplete_from_editor();
    a.handle_key(crate::shell::keys::Key::Tab);

    assert_eq!(a.editor.text(), "open @first.rs ");
}

#[tokio::test]
async fn stale_files_autocomplete_load_from_previous_workspace_is_discarded() {
    let mut h = harness().await;
    let a = h.app_mut();
    let old_root = tempfile::tempdir().expect("old workspace tempdir");
    let new_root = tempfile::tempdir().expect("new workspace tempdir");
    a.workspace.root = Some(new_root.path().to_path_buf());
    a.workspace
        .files_autocomplete
        .apply_loaded_files(vec!["new.rs".into()]);

    let applied = a.apply_files_autocomplete_load(
        old_root.path().to_path_buf(),
        vec!["stale-from-old-workspace.rs".into()],
    );

    assert!(!applied, "stale worker result must not be applied");
    a.editor.set_text("open @stale");
    a.refresh_files_autocomplete_from_editor();
    assert!(
        a.workspace.files_autocomplete.render(80).is_empty(),
        "old workspace files must not appear after switching roots"
    );
}

#[tokio::test]
async fn files_autocomplete_load_for_current_workspace_is_applied() {
    let mut h = harness().await;
    let a = h.app_mut();
    let root = tempfile::tempdir().expect("workspace tempdir");
    a.workspace.root = Some(root.path().to_path_buf());

    let applied =
        a.apply_files_autocomplete_load(root.path().to_path_buf(), vec!["current.rs".into()]);

    assert!(applied, "current workspace worker result should be applied");
    a.editor.set_text("open @cur");
    a.refresh_files_autocomplete_from_editor();
    let rendered = a.workspace.files_autocomplete.render(80).join("\n");
    assert!(rendered.contains("current.rs"));
}

// ── run() with a CLOSED master event channel (#2044 R1-2) ─────────────
// No sender is retained in production, so once the connection's feed task
// has ended `master_event_rx.recv()` answers `None` at once, forever. The
// select arm must then be DISABLED (`Some(item) = …`), leaving the loop
// parked on its other arms; an arm that accepts the `None` re-runs the loop
// immediately and burns a core.

/// Counts the wake-ups the polled future asks for, then forwards them.
struct WakeCounter {
    wakes: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    outer: std::task::Waker,
}

impl std::task::Wake for WakeCounter {
    fn wake(self: std::sync::Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &std::sync::Arc<Self>) {
        self.wakes
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.outer.wake_by_ref();
    }
}

/// Poll `future` exactly once with a wake-counting waker.
async fn poll_once_counting_wakes<F: std::future::Future>(
    future: &mut std::pin::Pin<Box<F>>,
    wakes: &std::sync::Arc<std::sync::atomic::AtomicUsize>,
) -> std::task::Poll<F::Output> {
    std::future::poll_fn(|cx| {
        let waker = std::task::Waker::from(std::sync::Arc::new(WakeCounter {
            wakes: wakes.clone(),
            outer: cx.waker().clone(),
        }));
        let mut counting = std::task::Context::from_waker(&waker);
        std::task::Poll::Ready(future.as_mut().poll(&mut counting))
    })
    .await
}

/// Virtual-time window the closed-channel loop is observed over: 1000 steps
/// of 10 ms = 10 s. A loop that spins asks to be woken on EVERY poll (tokio's
/// cooperative budget makes the spinning `recv()` yield with a self-wake),
/// so it scores one wake per step; a parked loop is woken only by its timers.
const CLOSED_CHANNEL_STEPS: usize = 1000;
const CLOSED_CHANNEL_STEP: std::time::Duration = std::time::Duration::from_millis(10);

#[tokio::test(start_paused = true)]
async fn run_stays_parked_and_live_once_the_master_event_channel_is_closed() {
    use crate::protocol::client::Event;
    use crate::shell::connection::{Connection, SourcedEvent};

    let mut h = harness().await;
    let a = h.app_mut();
    // Replacing the transport drops the harness connection and aborts its
    // feed task; with the test handle gone too, no sender is left.
    let (conn, _commands) = Connection::live_for_tests();
    a.test_attach_connection(conn, None);
    a.master_event_tx = None;
    while !a.master_event_rx.is_closed() {
        tokio::task::yield_now().await;
    }
    let other_arm = a.subagents.event_tx.clone();

    let wakes = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut run = Box::pin(a.run());
    for _ in 0..CLOSED_CHANNEL_STEPS {
        let polled = poll_once_counting_wakes(&mut run, &wakes).await;
        assert!(polled.is_pending(), "a closed channel must not end run()");
        tokio::time::advance(CLOSED_CHANNEL_STEP).await;
    }
    let parked_wakes = wakes.load(std::sync::atomic::Ordering::Relaxed);

    // Still live: another select arm keeps being served.
    other_arm
        .send(SourcedEvent::Master(Event::Token {
            token: "closed-channel-still-live".into(),
        }))
        .await
        .expect("the sub-agent fan-in stays open");
    for _ in 0..10 {
        let polled = poll_once_counting_wakes(&mut run, &wakes).await;
        assert!(polled.is_pending(), "a closed channel must not end run()");
        tokio::time::advance(CLOSED_CHANNEL_STEP).await;
    }
    drop(run);

    assert!(
        parked_wakes < CLOSED_CHANNEL_STEPS / 4,
        "run() must park on a closed master channel, not spin: {parked_wakes} wake-ups \
         in {CLOSED_CHANNEL_STEPS} polls over 10 s of virtual time"
    );
    h.capture();
    assert!(
        h.full_frame().contains("closed-channel-still-live"),
        "the loop must keep serving its other arms after the master channel closed"
    );
}
