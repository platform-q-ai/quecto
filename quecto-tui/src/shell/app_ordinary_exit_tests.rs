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
async fn ordinary_exit_kill_policy_cleans_up_in_flight_tab_spawn_watch_before_attach() {
    let mut h = TuiHarness::new().await;
    let a = h.app_mut();
    a.pending_tab_child_watches
        .lock()
        .unwrap()
        .push(crate::shell::child_watch::ChildWatch::for_tests(Some(88)));
    a.set_ordinary_exit_kill_owned(true);

    a.finalize_ordinary_exit().await;

    assert!(
        a.pending_tab_child_watches.lock().unwrap().is_empty(),
        "ordinary exit must drain TUI-owned watches registered by in-flight tab spawns"
    );
    assert!(
        a.take_all_child_exit_watches().is_empty(),
        "drained pending spawn watches must not remain for a later teardown race"
    );
}

/// #1956: a watcher that never answers cannot hold the exit past the budget
/// plus the KILL grace — terminal cleanup still runs and no error is invented.
#[tokio::test(start_paused = true)]
async fn ordinary_exit_unanswered_leader_termination_is_bounded_by_the_budget() {
    let mut h = TuiHarness::new().await;
    let a = h.app_mut();
    a.kitty.active = true;
    a.kitty.modify_other_keys = true;
    let (mut conn, mut rx) = Connection::live_for_tests();
    conn.set_tab_for_tests(TabId::MASTER);
    let (watch, mut term_rx) =
        crate::shell::child_watch::ChildWatch::for_tests_with_termination_probe(Some(99));
    a.attach_connection_to_tab(TabId::MASTER, conn, Some(watch));
    a.set_leader_exit_budget(std::time::Duration::from_millis(500));
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

    let finalization_errors = {
        let finalize = a.finalize_ordinary_exit();
        tokio::pin!(finalize);
        let (budget, _never_answered) = tokio::select! {
            req = term_rx.recv() => req.expect("leader termination requested"),
            _ = &mut finalize => panic!("finalizer completed before requesting leader termination"),
        };
        assert_eq!(
            budget,
            std::time::Duration::from_millis(500),
            "injected budget is passed through"
        );
        // budget + KILL grace (2 s) + 1 s slack = 3.5 s bound.
        tokio::select! {
            _ = &mut finalize => panic!("an unanswered termination must be bounded, not complete at once"),
            _ = tokio::time::sleep(std::time::Duration::from_millis(3_499)) => {}
        }
        tokio::time::advance(std::time::Duration::from_millis(1)).await;
        finalize.await
    };
    assert!(
        !a.kitty.active && !a.kitty.modify_other_keys,
        "terminal protocol cleanup must still run after an unanswered termination"
    );
    assert!(
        finalization_errors.is_empty(),
        "no outcome means nothing to report: {finalization_errors:?}"
    );
    assert!(
        !a.exit_policy.settling_notices_shown.is_empty(),
        "the settling notice was shown while waiting"
    );
    assert!(
        !a.notifications
            .messages()
            .iter()
            .any(|m| m.starts_with(super::SETTLING_NOTICE_PREFIX)),
        "the settling notice is dismissed once the wait ends"
    );
}

/// #1956: past ~1 s the settling notice is shown with elapsed seconds and
/// updated in place; it is dismissed when the leaders are gone.
#[tokio::test(start_paused = true)]
async fn ordinary_exit_shows_and_updates_the_settling_notice_then_dismisses_it() {
    let mut h = TuiHarness::new().await;
    let a = h.app_mut();
    let (watch, mut term_rx) =
        crate::shell::child_watch::ChildWatch::for_tests_with_termination_probe(Some(99));
    a.pending_tab_child_watches.lock().unwrap().push(watch);
    a.set_leader_exit_budget(std::time::Duration::from_secs(10));

    let errors = {
        let finalize = a.finalize_ordinary_exit();
        tokio::pin!(finalize);
        let (_budget, done) = tokio::select! {
            req = term_rx.recv() => req.expect("leader termination requested"),
            _ = &mut finalize => panic!("finalizer completed before requesting leader termination"),
        };
        // Nothing before the 1 s threshold, "(1s)" then "(2s)" afterwards.
        tokio::select! {
            _ = &mut finalize => panic!("must still be waiting"),
            _ = tokio::time::sleep(std::time::Duration::from_millis(999)) => {}
        }
        tokio::select! {
            _ = &mut finalize => panic!("must still be waiting"),
            _ = tokio::time::sleep(std::time::Duration::from_millis(1_201)) => {}
        }
        done.send(crate::shell::process::LeaderTermination {
            pid: Some(99),
            end: crate::shell::process::LeaderEnd::ExitedAfterTerm,
            waited: std::time::Duration::from_millis(2_200),
            strays: Vec::new(),
        })
        .unwrap();
        finalize.await
    };
    // The headless master never acks the persist barrier; only the leader
    // outcome matters here and a clean exit reports nothing.
    assert!(
        errors
            .iter()
            .all(|e| e.contains("persistence barrier timed out")),
        "{errors:?}"
    );
    assert_eq!(
        a.exit_policy.settling_notices_shown,
        vec![
            format!("{} (1s)", super::SETTLING_NOTICE_PREFIX),
            format!("{} (2s)", super::SETTLING_NOTICE_PREFIX),
        ],
        "notice appears at 1 s and is updated each second with the elapsed time"
    );
    // `messages()` keeps only what is still on the stack: the notice was
    // dismissed, and its updates replaced (not stacked) each other.
    assert!(
        !a.notifications
            .messages()
            .iter()
            .any(|m| m.starts_with(super::SETTLING_NOTICE_PREFIX)),
        "notice dismissed after the leaders exited: {:?}",
        a.notifications.messages()
    );
}

/// #1956: a SIGKILLed leader and canary strays are reported after cleanup;
/// a clean exit reports nothing.
#[test]
fn ordinary_exit_describes_kill_and_strays_only() {
    use crate::shell::process::{LeaderEnd, LeaderTermination};
    let budget = std::time::Duration::from_secs(30);
    let clean = LeaderTermination {
        pid: Some(4),
        end: LeaderEnd::ExitedAfterTerm,
        waited: std::time::Duration::from_secs(1),
        strays: Vec::new(),
    };
    assert_eq!(
        crate::shell::app::App::describe_leader_termination(&clean, budget),
        None
    );
    let killed = LeaderTermination {
        end: LeaderEnd::Killed,
        strays: vec![8, 9],
        ..clean.clone()
    };
    let text = crate::shell::app::App::describe_leader_termination(&killed, budget).unwrap();
    assert!(
        text.contains(
            "harness pid 4 did not exit within 30s of SIGTERM; sent SIGKILL to that process only"
        ),
        "{text}"
    );
    assert!(
        text.contains("post-exit canary: pids [8, 9] still name harness pid 4"),
        "{text}"
    );
    assert!(text.contains("not signalled"), "{text}");
    let no_pid = LeaderTermination {
        pid: None,
        ..killed.clone()
    };
    assert_eq!(
        crate::shell::app::App::describe_leader_termination(&no_pid, budget),
        None
    );
    let text = crate::shell::app::App::describe_leader_termination(
        &killed,
        std::time::Duration::from_millis(300),
    )
    .unwrap();
    assert!(text.contains("within 300ms of SIGTERM"), "{text}");
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
