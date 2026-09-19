use std::sync::{Arc, Mutex};

use crate::protocol::client::Event;
use crate::shell::app::tui_harness::TuiHarness;
use crate::shell::connection::{Connection, SourcedEvent};
use crate::shell::keys::Key;

#[tokio::test]
async fn ctrl_d_and_slash_exit_inputs_share_ordinary_exit_request() {
    let mut h = TuiHarness::new().await;
    h.app_mut().handle_key(Key::Ctrl('d'));
    assert!(h.should_exit(), "Ctrl-D requests ordinary exit");
    assert_eq!(
        h.pending_aborts(),
        0,
        "Ctrl-D must not uniquely abort active turns"
    );

    for cmd in ["/exit", "/quit"] {
        let mut h = TuiHarness::new().await;
        h.app_mut().handle_submit(cmd);
        assert!(h.should_exit(), "{cmd} requests ordinary exit");
        assert_eq!(h.pending_aborts(), 0, "{cmd} does not abort while exiting");
    }
}

#[tokio::test]
async fn ordinary_exit_finalizer_persists_the_session_before_terminal_cleanup() {
    let mut h = TuiHarness::new().await;
    let a = h.app_mut();
    let (conn, mut rx) = Connection::live_for_tests();
    a.test_attach_connection(conn, None);

    a.finalize_ordinary_exit().await;

    let cmd: serde_json::Value = serde_json::from_str(&rx.try_recv().unwrap()).unwrap();
    assert_eq!(cmd["type"], "persist_session");
    assert!(a.should_exit);
}

#[tokio::test]
async fn ordinary_exit_kill_policy_cleans_up_owned_child_watches() {
    let mut h = TuiHarness::new().await;
    let a = h.app_mut();
    let (conn, mut rx) = Connection::live_for_tests();
    a.test_attach_connection(
        conn,
        Some(crate::shell::child_watch::ChildWatch::for_tests(Some(77))),
    );
    a.set_ordinary_exit_kill_owned(true);

    a.finalize_ordinary_exit().await;

    assert!(rx.try_recv().is_ok(), "persist command still enqueued");
    assert!(
        a.take_child_exit_watch_with_roster().is_none(),
        "ordinary exit must drain the TUI-owned watch by default"
    );
}

#[tokio::test]
async fn ordinary_exit_detach_policy_leaves_owned_child_watches() {
    let mut h = TuiHarness::new().await;
    let a = h.app_mut();
    let (conn, mut rx) = Connection::live_for_tests();
    a.test_attach_connection(
        conn,
        Some(crate::shell::child_watch::ChildWatch::for_tests(Some(77))),
    );
    a.set_ordinary_exit_kill_owned(false);

    a.finalize_ordinary_exit().await;

    assert!(rx.try_recv().is_ok(), "persist command still enqueued");
    assert!(
        a.take_child_exit_watch_with_roster().is_some(),
        "detach-on-exit must leave the owned watch for drop/detach instead of termination"
    );
}

#[tokio::test]
async fn ordinary_exit_waits_for_persist_barrier_before_teardown() {
    let mut h = TuiHarness::new().await;
    let a = h.app_mut();
    let (conn, mut rx) = Connection::live_for_tests();
    a.test_attach_connection(conn, None);
    let event_tx = a.master_event_tx.clone().unwrap();

    tokio::spawn(async move {
        let cmd: serde_json::Value = serde_json::from_str(&rx.recv().await.unwrap()).unwrap();
        event_tx
            .send(SourcedEvent::Master(Event::Response {
                id: cmd["id"].as_str().map(str::to_string),
                command: "persist_session".to_string(),
                success: true,
                data: None,
                error: None,
            }))
            .await
            .unwrap();
    });

    a.finalize_ordinary_exit().await;
    assert!(
        a.notifications.messages().is_empty(),
        "successful barrier should not raise errors"
    );
}

#[tokio::test]
async fn ordinary_exit_reports_persist_enqueue_error_before_teardown() {
    let mut h = TuiHarness::new().await;
    let a = h.app_mut();
    let (conn, rx) = Connection::live_for_tests();
    drop(rx);
    a.test_attach_connection(conn, None);

    let finalization_errors = a.finalize_ordinary_exit().await;
    let emitted_errors = a.take_ordinary_exit_finalization_errors_for_tests();

    let msgs = a.notifications.messages().join("\n");
    assert!(
        msgs.contains("ordinary-exit persistence enqueue failed"),
        "enqueue error must be deliberate and visible: {msgs}"
    );
    assert!(
        finalization_errors
            .iter()
            .any(|msg| msg.contains("ordinary-exit persistence enqueue failed")),
        "enqueue error must be returned for post-teardown reporting: {finalization_errors:?}"
    );
    assert!(
        emitted_errors
            .iter()
            .any(|msg| msg.contains("ordinary-exit persistence enqueue failed")),
        "finalize_ordinary_exit itself must emit enqueue errors after cleanup: {emitted_errors:?}"
    );
}

#[tokio::test]
async fn ordinary_exit_reports_persist_barrier_failure_before_teardown() {
    let mut h = TuiHarness::new().await;
    let a = h.app_mut();
    let (conn, mut rx) = Connection::live_for_tests();
    a.test_attach_connection(conn, None);
    let event_tx = a.master_event_tx.clone().unwrap();

    tokio::spawn(async move {
        let cmd: serde_json::Value = serde_json::from_str(&rx.recv().await.unwrap()).unwrap();
        event_tx
            .send(SourcedEvent::Master(Event::Response {
                id: cmd["id"].as_str().map(str::to_string),
                command: "persist_session".to_string(),
                success: false,
                data: None,
                error: Some("disk full".to_string()),
            }))
            .await
            .unwrap();
    });

    let finalization_errors = a.finalize_ordinary_exit().await;
    let emitted_errors = a.take_ordinary_exit_finalization_errors_for_tests();

    let msgs = a.notifications.messages().join("\n");
    assert!(
        msgs.contains("disk full"),
        "barrier failure must be deliberate and visible: {msgs}"
    );
    assert!(
        finalization_errors
            .iter()
            .any(|msg| msg.contains("disk full")),
        "barrier failure must be returned for post-teardown reporting: {finalization_errors:?}"
    );
    assert!(
        emitted_errors.iter().any(|msg| msg.contains("disk full")),
        "finalize_ordinary_exit itself must emit failed-persist responses after cleanup: {emitted_errors:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn ordinary_exit_barrier_uses_single_overall_deadline_for_incidental_events() {
    let mut h = TuiHarness::new().await;
    let a = h.app_mut();
    let (conn, _rx) = Connection::live_for_tests();
    a.test_attach_connection(conn, None);
    let event_tx = a.master_event_tx.clone().unwrap();

    tokio::spawn(async move {
        for i in 0..4 {
            tokio::time::advance(std::time::Duration::from_millis(600)).await;
            event_tx
                .send(SourcedEvent::Master(Event::Token {
                    token: format!("incidental-{i}"),
                }))
                .await
                .unwrap();
        }
    });

    let finalization_errors = a.finalize_ordinary_exit().await;
    let emitted_errors = a.take_ordinary_exit_finalization_errors_for_tests();

    let msgs = a.notifications.messages().join("\n");
    assert!(
        msgs.contains("ordinary-exit persistence barrier timed out"),
        "{msgs}"
    );
    assert!(
        finalization_errors
            .iter()
            .any(|msg| msg.contains("ordinary-exit persistence barrier timed out")),
        "timeout must be returned for post-teardown reporting: {finalization_errors:?}"
    );
    assert!(
        emitted_errors
            .iter()
            .any(|msg| msg.contains("ordinary-exit persistence barrier timed out")),
        "finalize_ordinary_exit itself must emit timeout errors after cleanup: {emitted_errors:?}"
    );
}

#[test]
fn ordinary_exit_finalization_errors_emit_to_post_cleanup_stderr() {
    let errors = vec![
        "ordinary-exit persistence enqueue failed: channel closed".to_string(),
        "tab 0: disk full".to_string(),
        "ordinary-exit persistence barrier timed out".to_string(),
    ];
    let mut stderr = Vec::new();

    crate::shell::app::App::emit_ordinary_exit_finalization_errors_to(&errors, &mut stderr);

    let stderr = String::from_utf8(stderr).expect("stderr utf8");
    for expected in [
        "ordinary-exit persistence enqueue failed: channel closed",
        "tab 0: disk full",
        "ordinary-exit persistence barrier timed out",
    ] {
        assert!(
            stderr.contains(&format!(
                "quecto: ordinary-exit finalization error: {expected}"
            )),
            "missing post-cleanup stderr emission for {expected}: {stderr}"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn ordinary_exit_barrier_ignores_closed_sentinel_and_waits_for_the_persist_answer() {
    let mut h = TuiHarness::new().await;
    let a = h.app_mut();
    let (conn, mut rx) = Connection::live_for_tests();
    a.test_attach_connection(conn, None);
    let event_tx = a.master_event_tx.clone().unwrap();

    tokio::spawn(async move {
        let cmd: serde_json::Value = serde_json::from_str(&rx.recv().await.unwrap()).unwrap();
        event_tx.send(SourcedEvent::Closed).await.unwrap();
        tokio::time::advance(std::time::Duration::from_millis(500)).await;
        event_tx
            .send(SourcedEvent::Master(Event::Response {
                id: cmd["id"].as_str().map(str::to_string),
                command: "persist_session".to_string(),
                success: true,
                data: None,
                error: None,
            }))
            .await
            .unwrap();
    });

    let witness = Arc::new(Mutex::new(Vec::new()));
    let finalize = a.finalize_ordinary_exit();
    tokio::pin!(finalize);

    tokio::select! {
        _ = &mut finalize => panic!("Closed must not let finalize_ordinary_exit complete before the delayed ack"),
        _ = tokio::time::sleep(std::time::Duration::from_millis(499)) => {}
    }
    assert!(witness.lock().unwrap().is_empty());
    tokio::time::advance(std::time::Duration::from_millis(1)).await;
    let finalization_errors = finalize.await;
    witness.lock().unwrap().push("finalized".to_string());

    assert!(
        finalization_errors.is_empty(),
        "Closed is incidental and must not become a global timeout: {finalization_errors:?}"
    );
    assert_eq!(
        witness.lock().unwrap().as_slice(),
        &["finalized"],
        "barrier completed only after the persist id was acknowledged"
    );
}

#[tokio::test(start_paused = true)]
async fn ordinary_exit_barrier_ignores_subagent_event_and_waits_for_remaining_persists() {
    let mut h = TuiHarness::new().await;
    let a = h.app_mut();
    let (conn, mut rx) = Connection::live_for_tests();
    a.test_attach_connection(conn, None);
    let event_tx = a.master_event_tx.clone().unwrap();

    tokio::spawn(async move {
        let cmd: serde_json::Value = serde_json::from_str(&rx.recv().await.unwrap()).unwrap();
        event_tx
            .send(SourcedEvent::Subagent(
                "agent-1".to_string(),
                Event::Token {
                    token: "incidental".to_string(),
                },
            ))
            .await
            .unwrap();
        tokio::time::advance(std::time::Duration::from_millis(500)).await;
        event_tx
            .send(SourcedEvent::Master(Event::Response {
                id: cmd["id"].as_str().map(str::to_string),
                command: "persist_session".to_string(),
                success: true,
                data: None,
                error: None,
            }))
            .await
            .unwrap();
    });

    let witness = Arc::new(Mutex::new(Vec::new()));
    let finalize = a.finalize_ordinary_exit();
    tokio::pin!(finalize);

    tokio::select! {
        _ = &mut finalize => panic!("Subagent event must not let finalize_ordinary_exit complete before delayed ack"),
        _ = tokio::time::sleep(std::time::Duration::from_millis(499)) => {}
    }
    assert!(witness.lock().unwrap().is_empty());
    tokio::time::advance(std::time::Duration::from_millis(1)).await;
    let finalization_errors = finalize.await;
    witness.lock().unwrap().push("finalized".to_string());

    assert!(
        finalization_errors.is_empty(),
        "Subagent event is incidental and must not become a global timeout: {finalization_errors:?}"
    );
    assert_eq!(witness.lock().unwrap().as_slice(), &["finalized"]);
}

/// The exit persist carries the connection's id and records an owned agent
/// that is about to be killed as stopped; a detached or external agent keeps
/// its live recovery.
#[tokio::test]
async fn ordinary_exit_persistence_distinguishes_owned_killing_from_detach_and_external() {
    for (owned, kill_owned) in [(true, true), (true, false), (false, true)] {
        let mut h = TuiHarness::new().await;
        let a = h.app_mut();
        a.set_ordinary_exit_kill_owned(kill_owned);
        let (conn, mut rx) = Connection::live_for_tests();
        let watch = owned.then(|| crate::shell::child_watch::ChildWatch::for_tests(Some(123)));
        a.test_attach_connection(conn, watch);
        let id = a.enqueue_ordinary_exit_snapshot_persist().unwrap();
        let cmd: serde_json::Value = serde_json::from_str(&rx.try_recv().unwrap()).unwrap();
        assert_eq!(cmd["type"], "persist_session");
        assert_eq!(id, "tab0:persist-exit");
        assert_eq!(cmd["id"], "tab0:persist-exit");
        if owned && kill_owned {
            assert_eq!(cmd["restoreReason"], "ordinary_tui_exit_stopped");
        } else {
            assert!(
                cmd["restoreReason"].is_null(),
                "detach/external exit must preserve live recovery"
            );
        }
    }
}
