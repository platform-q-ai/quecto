use std::sync::{Arc, Mutex};

use crate::protocol::client::Event;
use crate::shell::app::tui_harness::TuiHarness;
use crate::shell::connection::{Connection, SourcedEvent, TabId};
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
async fn ordinary_exit_finalizer_persists_all_visible_tabs_before_terminal_cleanup() {
    let mut h = TuiHarness::new().await;
    let a = h.app_mut();
    let (mut master_conn, mut master_rx) = Connection::live_for_tests();
    master_conn.set_tab_for_tests(TabId::MASTER);
    a.attach_connection_to_tab(TabId::MASTER, master_conn, None);
    let tab = a.open_placeholder_tab(Some("worker".into()));
    let (mut tab_conn, mut tab_rx) = Connection::live_for_tests();
    tab_conn.set_tab_for_tests(tab);
    a.attach_connection_to_tab(tab, tab_conn, None);

    a.finalize_ordinary_exit().await;

    let master_cmd: serde_json::Value =
        serde_json::from_str(&master_rx.try_recv().unwrap()).unwrap();
    let tab_cmd: serde_json::Value = serde_json::from_str(&tab_rx.try_recv().unwrap()).unwrap();
    assert_eq!(master_cmd["type"], "persist_session");
    assert_eq!(tab_cmd["type"], "persist_session");
    assert!(a.should_exit);
}

#[tokio::test]
async fn ordinary_exit_kill_policy_cleans_up_owned_child_watches() {
    let mut h = TuiHarness::new().await;
    let a = h.app_mut();
    let (mut conn, mut rx) = Connection::live_for_tests();
    conn.set_tab_for_tests(TabId::MASTER);
    a.attach_connection_to_tab(
        TabId::MASTER,
        conn,
        Some(crate::shell::child_watch::ChildWatch::for_tests(Some(77))),
    );
    a.set_ordinary_exit_kill_owned(true);

    a.finalize_ordinary_exit().await;

    assert!(rx.try_recv().is_ok(), "persist command still enqueued");
    assert!(
        a.take_all_child_exit_watches().is_empty(),
        "ordinary exit must drain TUI-owned watches by default"
    );
}

#[tokio::test]
async fn ordinary_exit_detach_policy_leaves_owned_child_watches() {
    let mut h = TuiHarness::new().await;
    let a = h.app_mut();
    let (mut conn, mut rx) = Connection::live_for_tests();
    conn.set_tab_for_tests(TabId::MASTER);
    a.attach_connection_to_tab(
        TabId::MASTER,
        conn,
        Some(crate::shell::child_watch::ChildWatch::for_tests(Some(77))),
    );
    a.set_ordinary_exit_kill_owned(false);

    a.finalize_ordinary_exit().await;

    assert!(rx.try_recv().is_ok(), "persist command still enqueued");
    assert_eq!(
        a.take_all_child_exit_watches().len(),
        1,
        "detach-on-exit must leave owned watches for drop/detach instead of termination"
    );
}

#[tokio::test]
async fn ordinary_exit_waits_for_persist_barrier_before_teardown() {
    let mut h = TuiHarness::new().await;
    let a = h.app_mut();
    let (mut conn, mut rx) = Connection::live_for_tests();
    conn.set_tab_for_tests(TabId::MASTER);
    a.attach_connection_to_tab(TabId::MASTER, conn, None);
    let event_tx = a.tab_event_tx.clone().unwrap();

    tokio::spawn(async move {
        let cmd: serde_json::Value = serde_json::from_str(&rx.recv().await.unwrap()).unwrap();
        event_tx
            .send(SourcedEvent::Tab(
                TabId::MASTER,
                Event::Response {
                    id: cmd["id"].as_str().map(str::to_string),
                    command: "persist_session".to_string(),
                    success: true,
                    data: None,
                    error: None,
                },
            ))
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
    let (mut conn, rx) = Connection::live_for_tests();
    drop(rx);
    conn.set_tab_for_tests(TabId::MASTER);
    a.attach_connection_to_tab(TabId::MASTER, conn, None);

    crate::shell::app::App::take_ordinary_exit_finalization_errors_for_tests();
    let finalization_errors = a.finalize_ordinary_exit().await;
    let emitted_errors = crate::shell::app::App::take_ordinary_exit_finalization_errors_for_tests();

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
    let (mut conn, mut rx) = Connection::live_for_tests();
    conn.set_tab_for_tests(TabId::MASTER);
    a.attach_connection_to_tab(TabId::MASTER, conn, None);
    let event_tx = a.tab_event_tx.clone().unwrap();

    tokio::spawn(async move {
        let cmd: serde_json::Value = serde_json::from_str(&rx.recv().await.unwrap()).unwrap();
        event_tx
            .send(SourcedEvent::Tab(
                TabId::MASTER,
                Event::Response {
                    id: cmd["id"].as_str().map(str::to_string),
                    command: "persist_session".to_string(),
                    success: false,
                    data: None,
                    error: Some("disk full".to_string()),
                },
            ))
            .await
            .unwrap();
    });

    crate::shell::app::App::take_ordinary_exit_finalization_errors_for_tests();
    let finalization_errors = a.finalize_ordinary_exit().await;
    let emitted_errors = crate::shell::app::App::take_ordinary_exit_finalization_errors_for_tests();

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
async fn ordinary_exit_partial_enqueue_failure_still_waits_for_successful_barriers() {
    let mut h = TuiHarness::new().await;
    let a = h.app_mut();
    let (mut failed_conn, failed_rx) = Connection::live_for_tests();
    drop(failed_rx);
    failed_conn.set_tab_for_tests(TabId::MASTER);
    a.attach_connection_to_tab(TabId::MASTER, failed_conn, None);
    let tab = a.open_placeholder_tab(Some("worker".into()));
    let (mut ok_conn, mut ok_rx) = Connection::live_for_tests();
    ok_conn.set_tab_for_tests(tab);
    a.attach_connection_to_tab(tab, ok_conn, None);
    let event_tx = a.tab_event_tx.clone().unwrap();

    tokio::spawn(async move {
        let cmd: serde_json::Value = serde_json::from_str(&ok_rx.recv().await.unwrap()).unwrap();
        tokio::time::advance(std::time::Duration::from_millis(500)).await;
        event_tx
            .send(SourcedEvent::Tab(
                tab,
                Event::Response {
                    id: cmd["id"].as_str().map(str::to_string),
                    command: "persist_session".to_string(),
                    success: true,
                    data: None,
                    error: None,
                },
            ))
            .await
            .unwrap();
    });

    a.finalize_ordinary_exit().await;

    let msgs = a.notifications.messages().join("\n");
    assert!(msgs.contains("ordinary-exit persistence enqueue failed"));
    assert!(
        !msgs.contains("barrier timed out"),
        "successful enqueue was awaited: {msgs}"
    );
}

#[tokio::test(start_paused = true)]
async fn ordinary_exit_mixed_barrier_failure_still_waits_for_other_ids() {
    let mut h = TuiHarness::new().await;
    let a = h.app_mut();
    let (mut master_conn, mut master_rx) = Connection::live_for_tests();
    master_conn.set_tab_for_tests(TabId::MASTER);
    a.attach_connection_to_tab(TabId::MASTER, master_conn, None);
    let tab = a.open_placeholder_tab(Some("worker".into()));
    let (mut tab_conn, mut tab_rx) = Connection::live_for_tests();
    tab_conn.set_tab_for_tests(tab);
    a.attach_connection_to_tab(tab, tab_conn, None);
    let event_tx = a.tab_event_tx.clone().unwrap();

    tokio::spawn(async move {
        let master_cmd: serde_json::Value =
            serde_json::from_str(&master_rx.recv().await.unwrap()).unwrap();
        let tab_cmd: serde_json::Value =
            serde_json::from_str(&tab_rx.recv().await.unwrap()).unwrap();
        event_tx
            .send(SourcedEvent::Tab(
                TabId::MASTER,
                Event::Response {
                    id: master_cmd["id"].as_str().map(str::to_string),
                    command: "persist_session".to_string(),
                    success: false,
                    data: None,
                    error: Some("disk full".to_string()),
                },
            ))
            .await
            .unwrap();
        tokio::time::advance(std::time::Duration::from_millis(500)).await;
        event_tx
            .send(SourcedEvent::Tab(
                tab,
                Event::Response {
                    id: tab_cmd["id"].as_str().map(str::to_string),
                    command: "persist_session".to_string(),
                    success: true,
                    data: None,
                    error: None,
                },
            ))
            .await
            .unwrap();
    });

    a.finalize_ordinary_exit().await;

    let msgs = a.notifications.messages().join("\n");
    assert!(
        msgs.contains("disk full"),
        "barrier failure visible: {msgs}"
    );
    assert!(
        !msgs.contains("barrier timed out"),
        "remaining successful persist id was awaited despite earlier failure: {msgs}"
    );
}

#[tokio::test(start_paused = true)]
async fn ordinary_exit_barrier_uses_single_overall_deadline_for_incidental_events() {
    let mut h = TuiHarness::new().await;
    let a = h.app_mut();
    let (mut conn, _rx) = Connection::live_for_tests();
    conn.set_tab_for_tests(TabId::MASTER);
    a.attach_connection_to_tab(TabId::MASTER, conn, None);
    let event_tx = a.tab_event_tx.clone().unwrap();

    tokio::spawn(async move {
        for i in 0..4 {
            tokio::time::advance(std::time::Duration::from_millis(600)).await;
            event_tx
                .send(SourcedEvent::Tab(
                    TabId::MASTER,
                    Event::Token {
                        token: format!("incidental-{i}"),
                    },
                ))
                .await
                .unwrap();
        }
    });

    crate::shell::app::App::take_ordinary_exit_finalization_errors_for_tests();
    let finalization_errors = a.finalize_ordinary_exit().await;
    let emitted_errors = crate::shell::app::App::take_ordinary_exit_finalization_errors_for_tests();

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
async fn ordinary_exit_barrier_ignores_closed_sentinel_and_waits_for_remaining_persists() {
    let mut h = TuiHarness::new().await;
    let a = h.app_mut();
    let (mut master_conn, mut master_rx) = Connection::live_for_tests();
    master_conn.set_tab_for_tests(TabId::MASTER);
    a.attach_connection_to_tab(TabId::MASTER, master_conn, None);
    let tab = a.open_placeholder_tab(Some("worker".into()));
    let (mut tab_conn, mut tab_rx) = Connection::live_for_tests();
    tab_conn.set_tab_for_tests(tab);
    a.attach_connection_to_tab(tab, tab_conn, None);
    let event_tx = a.tab_event_tx.clone().unwrap();

    tokio::spawn(async move {
        let master_cmd: serde_json::Value =
            serde_json::from_str(&master_rx.recv().await.unwrap()).unwrap();
        let tab_cmd: serde_json::Value =
            serde_json::from_str(&tab_rx.recv().await.unwrap()).unwrap();
        event_tx
            .send(SourcedEvent::Closed(TabId::MASTER))
            .await
            .unwrap();
        tokio::time::advance(std::time::Duration::from_millis(500)).await;
        event_tx
            .send(SourcedEvent::Tab(
                tab,
                Event::Response {
                    id: tab_cmd["id"].as_str().map(str::to_string),
                    command: "persist_session".to_string(),
                    success: true,
                    data: None,
                    error: None,
                },
            ))
            .await
            .unwrap();
        event_tx
            .send(SourcedEvent::Tab(
                TabId::MASTER,
                Event::Response {
                    id: master_cmd["id"].as_str().map(str::to_string),
                    command: "persist_session".to_string(),
                    success: true,
                    data: None,
                    error: None,
                },
            ))
            .await
            .unwrap();
    });

    let witness = Arc::new(Mutex::new(Vec::new()));
    let finalize = a.finalize_ordinary_exit();
    tokio::pin!(finalize);

    tokio::select! {
        _ = &mut finalize => panic!("Closed(tab) must not let finalize_ordinary_exit complete before delayed acks"),
        _ = tokio::time::sleep(std::time::Duration::from_millis(499)) => {}
    }
    assert!(witness.lock().unwrap().is_empty());
    tokio::time::advance(std::time::Duration::from_millis(1)).await;
    let finalization_errors = finalize.await;
    witness.lock().unwrap().push("finalized".to_string());

    assert!(
        finalization_errors.is_empty(),
        "Closed(tab) is incidental and must not become a global timeout: {finalization_errors:?}"
    );
    assert_eq!(
        witness.lock().unwrap().as_slice(),
        &["finalized"],
        "barrier completed only after both persist ids were acknowledged"
    );
}

#[tokio::test(start_paused = true)]
async fn ordinary_exit_barrier_ignores_subagent_event_and_waits_for_remaining_persists() {
    let mut h = TuiHarness::new().await;
    let a = h.app_mut();
    let (mut conn, mut rx) = Connection::live_for_tests();
    conn.set_tab_for_tests(TabId::MASTER);
    a.attach_connection_to_tab(TabId::MASTER, conn, None);
    let event_tx = a.tab_event_tx.clone().unwrap();

    tokio::spawn(async move {
        let cmd: serde_json::Value = serde_json::from_str(&rx.recv().await.unwrap()).unwrap();
        event_tx
            .send(SourcedEvent::Subagent(
                TabId::MASTER,
                "agent-1".to_string(),
                Event::Token {
                    token: "incidental".to_string(),
                },
            ))
            .await
            .unwrap();
        tokio::time::advance(std::time::Duration::from_millis(500)).await;
        event_tx
            .send(SourcedEvent::Tab(
                TabId::MASTER,
                Event::Response {
                    id: cmd["id"].as_str().map(str::to_string),
                    command: "persist_session".to_string(),
                    success: true,
                    data: None,
                    error: None,
                },
            ))
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
