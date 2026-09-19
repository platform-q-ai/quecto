//! #1956: leader-only termination on ordinary exit — bounded waits, the
//! settling notice, the post-cleanup report and the roster-derived budget.

use crate::protocol::client::Event;
use crate::shell::app::tui_harness::TuiHarness;
use crate::shell::connection::{Connection, SourcedEvent, TabId};

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
    a.test_attach_connection(TabId::MASTER, conn, Some(watch));
    a.set_leader_budget(crate::shell::process::LeaderBudget {
        settle: std::time::Duration::from_millis(300),
        force: std::time::Duration::from_millis(200),
    });
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
            budget.settle,
            std::time::Duration::from_millis(300),
            "injected budget is passed through"
        );
        // settle + force + KILL grace (2 s) + 1 s slack = 3.5 s bound.
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
    a.set_child_exit_watch(watch);
    a.set_leader_budget(crate::shell::process::LeaderBudget {
        settle: std::time::Duration::from_secs(10),
        force: std::time::Duration::from_secs(10),
    });

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
    use crate::shell::process::{LeaderBudget, LeaderEnd, LeaderTermination};
    let budget = LeaderBudget::for_children(Some(0));
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
    let repeated = LeaderTermination {
        end: LeaderEnd::ExitedAfterRepeatedTerm,
        ..clean.clone()
    };
    assert_eq!(
        crate::shell::app::App::describe_leader_termination(&repeated, budget),
        None,
        "the harness's own force-exit ending it is not an error"
    );
    let killed = LeaderTermination {
        end: LeaderEnd::Killed,
        strays: vec![8, 9],
        ..clean.clone()
    };
    let text = crate::shell::app::App::describe_leader_termination(&killed, budget).unwrap();
    assert!(
        text.contains("harness pid 4 did not exit within 30s of SIGTERM nor 45s of a repeated SIGTERM; sent SIGKILL to that process only"),
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
        LeaderBudget {
            settle: std::time::Duration::from_millis(300),
            force: std::time::Duration::from_millis(200),
        },
    )
    .unwrap();
    assert!(
        text.contains("within 300ms of SIGTERM nor 200ms of a repeated SIGTERM"),
        "{text}"
    );
}

/// #1956 review: the budget each owned leader gets is derived from the
/// subagent count its connection's roster last showed; an unknown count
/// gets the worst case.
#[tokio::test]
async fn ordinary_exit_budget_follows_each_tabs_roster() {
    use crate::shell::process::LeaderBudget;
    let mut h = TuiHarness::new().await;
    let a = h.app_mut();
    assert_eq!(
        a.leader_budget(Some(0)),
        LeaderBudget::for_children(Some(0))
    );
    assert_eq!(
        a.leader_budget(Some(9)).settle,
        std::time::Duration::from_secs(55)
    );
    assert_eq!(a.leader_budget(None), LeaderBudget::WORST_CASE);
    let tab = TabId::MASTER;
    a.conn_mut(tab).unwrap().child_exit_watch =
        Some(crate::shell::child_watch::ChildWatch::for_tests(Some(5)));
    for i in 0..9 {
        a.conn_mut(tab).unwrap().roster.tracked.insert(
            format!("agent-{i}"),
            crate::agents::roster::TrackedSubagent::new(
                crate::protocol::client::SubagentInfoEvent {
                    agent_id: format!("agent-{i}"),
                    agent_uuid: None,
                    display_name: None,
                    status: "running".into(),
                    last_tool: None,
                    last_error: None,
                    compact: false,
                    pid: 0,
                    socket_path: None,
                    parent_id: None,
                    workflow: None,
                    read_only: false,
                    execution_backend: None,
                    environment: None,
                },
            ),
        );
    }
    let taken = a.take_all_child_exit_watches_with_rosters();
    let mut counts: Vec<(Option<u32>, Option<usize>)> =
        taken.iter().map(|(w, n)| (w.pid(), *n)).collect();
    counts.sort();
    assert_eq!(counts, vec![(Some(5), Some(9))]);
    let override_budget = LeaderBudget {
        settle: std::time::Duration::from_secs(1),
        force: std::time::Duration::from_secs(1),
    };
    a.set_leader_budget(override_budget);
    assert_eq!(a.leader_budget(Some(9)), override_budget);
}
