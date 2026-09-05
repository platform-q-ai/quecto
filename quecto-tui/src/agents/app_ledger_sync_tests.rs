use super::*;
use serde_json::json;

fn feed_with_rx() -> (FeedState, mpsc::Receiver<Command>) {
    let (cmd_tx, cmd_rx) = mpsc::channel(8);
    let handle = tokio::spawn(async {});
    (
        FeedState {
            cmd_tx,
            handle,
            inspection_only: false,
            epoch: 0,
            rev: 0,
            last_fresh_at: None,
            supports_sync: false,
            pending_rev: None,
            transcript: crate::agents::ledger::LedgerTranscript::default(),
            authority: crate::agents::feed::FeedAuthority::WarmSync,
        },
        cmd_rx,
    )
}

fn sync_delta(epoch: u64, rev: u64) -> serde_json::Value {
    json!({
        "epoch": epoch,
        "rev": rev,
        "messages": [{"id":"m1","role":"user","content":"hello"}],
        "nextRev": null,
        "caughtUp": true,
        "resync": false
    })
}

#[tokio::test]
async fn ledger_hint_requests_sync_only_after_capability_is_known() {
    let mut h = super::tui_harness::TuiHarness::new().await;
    let app = h.app_mut();
    let (feed, mut rx) = feed_with_rx();
    app.ac_mut().roster.feeds.insert("a1".into(), feed);

    app.note_ledger_advanced("a1", 1, 9);
    assert!(rx.try_recv().is_err());

    app.note_sync_capability("a1", &json!({"sync":1}));

    let cmd = rx.try_recv().expect("sync command after capability");
    assert!(matches!(
        cmd,
        Command::Sync {
            epoch: 1,
            since_rev: 0,
            ..
        }
    ));
}

#[tokio::test]
async fn sync_response_updates_cursor_and_uses_next_revision_for_follow_up() {
    let mut h = super::tui_harness::TuiHarness::new().await;
    let app = h.app_mut();
    let (mut feed, mut rx) = feed_with_rx();
    feed.supports_sync = true;
    app.ac_mut().roster.feeds.insert("a1".into(), feed);
    app.ensure_session("a1");

    app.route_sync_response(
        "a1",
        &json!({
            "epoch": 3,
            "rev": 12,
            "messages": [{"id":"m1","role":"user","content":"page"}],
            "nextRev": 8,
            "caughtUp": false,
            "resync": false
        }),
    );

    let feed = app.ac().roster.feeds.get("a1").unwrap();
    assert_eq!(feed.epoch, 3);
    assert_eq!(feed.rev, 8);
    assert!(feed.last_fresh_at.is_some());
    let entries = app.ac().roster.sessions["a1"].chat.entries();
    assert!(matches!(entries, [ChatEntry::User { text }] if text == "page"));
    let cmd = rx.try_recv().expect("continuation sync command");
    assert!(matches!(
        cmd,
        Command::Sync {
            epoch: 3,
            since_rev: 8,
            ..
        }
    ));
}

#[tokio::test]
async fn capability_alone_does_not_make_warm_feed_authoritative() {
    let mut h = super::tui_harness::TuiHarness::new().await;
    let app = h.app_mut();
    let (mut feed, _rx) = feed_with_rx();
    feed.authority = crate::agents::feed::FeedAuthority::WarmSync;
    app.ac_mut().roster.feeds.insert("a1".into(), feed);

    app.note_sync_capability("a1", &json!({"sync":1}));

    assert_eq!(
        app.ac().roster.feeds["a1"].authority,
        crate::agents::feed::FeedAuthority::WarmSync,
        "a feed is authoritative only after a sync delta is applied"
    );
}

#[tokio::test]
async fn sync_response_promotes_warm_feed_to_authoritative() {
    let mut h = super::tui_harness::TuiHarness::new().await;
    let app = h.app_mut();
    let (mut feed, _rx) = feed_with_rx();
    feed.authority = crate::agents::feed::FeedAuthority::WarmSync;
    app.ac_mut().roster.feeds.insert("a1".into(), feed);
    app.ensure_session("a1");

    app.route_sync_response("a1", &sync_delta(1, 1));

    assert_eq!(
        app.ac().roster.feeds["a1"].authority,
        crate::agents::feed::FeedAuthority::SyncedAuthoritative
    );
}

#[tokio::test]
async fn sync_response_for_wrong_epoch_without_resync_is_ignored() {
    let mut h = super::tui_harness::TuiHarness::new().await;
    let app = h.app_mut();
    let (mut feed, _rx) = feed_with_rx();
    feed.epoch = 2;
    feed.rev = 5;
    app.ac_mut().roster.feeds.insert("a1".into(), feed);
    app.ensure_session("a1");

    app.route_sync_response("a1", &sync_delta(3, 9));

    let feed = app.ac().roster.feeds.get("a1").unwrap();
    assert_eq!(feed.epoch, 2);
    assert_eq!(feed.rev, 5);
    assert_eq!(app.ac().roster.sessions["a1"].chat.entry_count(), 0);
}

#[tokio::test]
async fn epoch_mismatch_resync_replaces_stale_synced_transcript() {
    let mut h = super::tui_harness::TuiHarness::new().await;
    let app = h.app_mut();
    let (mut feed, _rx) = feed_with_rx();
    feed.epoch = 2;
    feed.rev = 5;
    feed.supports_sync = true;
    app.ac_mut().roster.feeds.insert("a1".into(), feed);
    app.ensure_session("a1");
    app.route_sync_response("a1", &sync_delta(2, 5));

    app.route_sync_response(
        "a1",
        &json!({
            "epoch": 3,
            "rev": 1,
            "messages": [{"id":"fresh","role":"user","content":"fresh session"}],
            "nextRev": null,
            "caughtUp": true,
            "resync": true
        }),
    );

    let feed = app.ac().roster.feeds.get("a1").unwrap();
    assert_eq!(feed.epoch, 3);
    assert_eq!(feed.rev, 1);
    let entries = app.ac().roster.sessions["a1"].chat.entries();
    assert!(matches!(entries, [ChatEntry::User { text }] if text == "fresh session"));
}

// ── Refused sync must not strand the feed (child-progress freeze fix) ────────

fn full_channel_feed() -> (FeedState, mpsc::Receiver<Command>) {
    // Capacity-1 channel, prefilled: the next try_send is refused.
    let (cmd_tx, cmd_rx) = mpsc::channel(1);
    cmd_tx
        .try_send(Command::GetState {
            id: None,
            agent_id: None,
        })
        .expect("prefill");
    let handle = tokio::spawn(async {});
    (
        FeedState {
            cmd_tx,
            handle,
            inspection_only: false,
            epoch: 0,
            rev: 0,
            last_fresh_at: None,
            supports_sync: true,
            pending_rev: None,
            transcript: crate::agents::ledger::LedgerTranscript::default(),
            authority: crate::agents::feed::FeedAuthority::WarmSync,
        },
        cmd_rx,
    )
}

#[tokio::test]
async fn refused_sync_send_leaves_no_phantom_pending_rev() {
    let mut h = super::tui_harness::TuiHarness::new().await;
    let app = h.app_mut();
    let (feed, _rx) = full_channel_feed();
    app.ac_mut().roster.feeds.insert("a1".into(), feed);

    app.note_ledger_advanced("a1", 1, 9);

    assert_eq!(
        app.ac().roster.feeds["a1"].pending_rev,
        None,
        "a refused Sync marked in-flight is a phantom sync that never resolves"
    );
}

#[tokio::test]
async fn next_ledger_hint_retries_after_a_refused_sync() {
    let mut h = super::tui_harness::TuiHarness::new().await;
    let app = h.app_mut();
    let (feed, mut rx) = full_channel_feed();
    app.ac_mut().roster.feeds.insert("a1".into(), feed);

    // First hint: channel full, refused, nothing recorded in-flight.
    app.note_ledger_advanced("a1", 1, 9);
    assert_eq!(app.ac().roster.feeds["a1"].pending_rev, None);

    // Channel drains (the prefill pops), then the next hint must retry.
    let _ = rx.try_recv().expect("drain prefill");
    app.note_ledger_advanced("a1", 1, 10);

    let cmd = rx.try_recv().expect("retry sync after refusal");
    assert!(matches!(
        cmd,
        Command::Sync {
            epoch: 1,
            since_rev: 0,
            ..
        }
    ));
    assert_eq!(app.ac().roster.feeds["a1"].pending_rev, Some(10));
}

#[tokio::test]
async fn accepted_sync_still_records_pending_rev() {
    let mut h = super::tui_harness::TuiHarness::new().await;
    let app = h.app_mut();
    let (mut feed, mut rx) = feed_with_rx();
    feed.supports_sync = true;
    app.ac_mut().roster.feeds.insert("a1".into(), feed);

    app.note_ledger_advanced("a1", 1, 9);

    assert!(rx.try_recv().is_ok(), "sync sent on open channel");
    assert_eq!(app.ac().roster.feeds["a1"].pending_rev, Some(9));
}

#[tokio::test]
async fn app_1196_child_sync_caps_feed_and_session_chat() {
    let mut h = super::tui_harness::TuiHarness::new().await;
    let app = h.app_mut();
    let (mut feed, _rx) = feed_with_rx();
    feed.supports_sync = true;
    app.ac_mut().roster.feeds.insert("a1".into(), feed);
    app.ensure_session("a1");
    let messages: Vec<_> = (0..(crate::components::chat::CHAT_RETAINED_ENTRY_CAP + 40))
        .map(|i| json!({"id": format!("m-{i}"), "role":"user", "content": format!("msg-{i}")}))
        .collect();

    app.route_sync_response("a1", &json!({"epoch":1,"rev":1,"messages":messages,"nextRev":null,"caughtUp":true,"resync":false}));

    assert!(
        app.ac().roster.feeds["a1"]
            .transcript
            .retained_message_count()
            <= crate::agents::ledger::LEDGER_RETAINED_MESSAGE_CAP
    );
    assert!(
        app.ac().roster.sessions["a1"].chat.entry_count()
            <= crate::components::chat::CHAT_RETAINED_ENTRY_CAP
    );
    assert!(
        app.ac().roster.sessions["a1"]
            .chat
            .entries()
            .iter()
            .any(|e| matches!(e, ChatEntry::User { text } if text == "msg-1063"))
    );
}

#[tokio::test]
async fn app_1196_multi_session_overflow_is_isolated() {
    let mut h = super::tui_harness::TuiHarness::new().await;
    let app = h.app_mut();
    let (mut feed_a, _rx_a) = feed_with_rx();
    let (mut feed_b, _rx_b) = feed_with_rx();
    feed_a.supports_sync = true;
    feed_b.supports_sync = true;
    app.ac_mut().roster.feeds.insert("a".into(), feed_a);
    app.ac_mut().roster.feeds.insert("b".into(), feed_b);
    app.ensure_session("a");
    app.ensure_session("b");
    app.ac_mut().roster.active_agent_id = Some("b".into());
    app.route_sync_response("b", &json!({"epoch":1,"rev":1,"messages":[{"id":"b1","role":"user","content":"keep-b"}],"nextRev":null,"caughtUp":true,"resync":false}));
    let messages: Vec<_> = (0..(crate::components::chat::CHAT_RETAINED_ENTRY_CAP + 10))
        .map(|i| json!({"id": format!("a-{i}"), "role":"user", "content": format!("a-msg-{i}")}))
        .collect();

    app.route_sync_response("a", &json!({"epoch":1,"rev":1,"messages":messages,"nextRev":null,"caughtUp":true,"resync":false}));

    assert_eq!(app.ac().roster.active_agent_id.as_deref(), Some("b"));
    assert!(
        matches!(app.ac().roster.sessions["b"].chat.entries(), [ChatEntry::User { text }] if text == "keep-b")
    );
    assert!(
        app.ac().roster.sessions["a"].chat.entry_count()
            <= crate::components::chat::CHAT_RETAINED_ENTRY_CAP
    );
}

// Exercise the production event loop rather than explicitly calling a retry
// helper: after the last hint there must be an autonomous wakeup, even idle.
async fn service_without_new_hint(h: &mut super::tui_harness::TuiHarness) {
    h.app_mut().suppress_paint = true;
    let _ = tokio::time::timeout(std::time::Duration::from_millis(2300), h.app_mut().run()).await;
}

#[tokio::test]
async fn issue_1605_refused_final_sync_recovers_without_another_hint() {
    let mut h = super::tui_harness::TuiHarness::new().await;
    let (feed, mut rx) = full_channel_feed();
    h.app_mut().ac_mut().roster.feeds.insert("a1".into(), feed);
    h.app_mut().note_ledger_advanced("a1", 1, 9);
    rx.try_recv().expect("drain prefill after refusal");
    service_without_new_hint(&mut h).await;
    assert!(
        matches!(rx.try_recv(), Ok(Command::Sync { .. })),
        "last refused sync must retry automatically while idle"
    );
}

#[tokio::test]
async fn issue_1605_lost_final_sync_response_recovers_without_another_hint() {
    let mut h = super::tui_harness::TuiHarness::new().await;
    let (mut feed, mut rx) = feed_with_rx();
    // Keep this simulated feed alive; a finished task is a disconnected feed.
    feed.handle = tokio::spawn(std::future::pending());
    feed.supports_sync = true;
    h.app_mut().ac_mut().roster.feeds.insert("a1".into(), feed);
    h.app_mut().note_ledger_advanced("a1", 1, 9);
    rx.try_recv()
        .expect("accepted request whose response is lost");
    service_without_new_hint(&mut h).await;
    assert!(
        matches!(rx.try_recv(), Ok(Command::Sync { .. })),
        "accepted request with lost response must not block autonomous refresh"
    );
}

#[tokio::test]
async fn issue_1605_refused_pagination_continuation_recovers_without_hint() {
    let mut h = super::tui_harness::TuiHarness::new().await;
    let (feed, mut rx) = full_channel_feed();
    h.app_mut().ac_mut().roster.feeds.insert("a1".into(), feed);
    h.app_mut().ensure_session("a1");
    h.app_mut().route_sync_response(
        "a1",
        &json!({
            "epoch":1,"rev":9,"messages":[],"nextRev":4,"caughtUp":false,"resync":false
        }),
    );
    rx.try_recv()
        .expect("drain prefill after refused continuation");
    service_without_new_hint(&mut h).await;
    assert!(
        matches!(rx.try_recv(), Ok(Command::Sync { since_rev: 4, .. })),
        "pagination must resume at the applied cursor without a later hint"
    );
}

#[tokio::test]
async fn issue_1605_inspection_only_refreshes_automatically_while_idle() {
    let mut h = super::tui_harness::TuiHarness::new().await;
    h.event(super::tui_harness::subagents_changed(vec![
        super::tui_harness::subagent("socketless", "idle", None),
    ]));
    h.select(Some("socketless"));
    assert!(h.app_mut().ac().roster.feeds["socketless"].inspection_only);
    let _ = h.drain_commands().await;
    service_without_new_hint(&mut h).await;
    let commands = h.drain_commands().await;
    assert!(
        commands.iter().any(|line| {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            value["type"] == "sync" && value["agent_id"] == "socketless"
        }),
        "inspection-only feed needs automatic routed sync after initial requests: {commands:?}"
    );
}

#[tokio::test]
async fn issue_1605_slim_state_after_successful_sync_does_not_disable_hints() {
    let mut h = super::tui_harness::TuiHarness::new().await;
    let (feed, mut rx) = feed_with_rx();
    let app = h.app_mut();
    app.ac_mut().roster.feeds.insert("a1".into(), feed);
    app.ensure_session("a1");
    app.route_sync_response("a1", &sync_delta(1, 1));
    app.note_sync_capability("a1", &json!({"state":"thinking","generation":2}));
    app.note_ledger_advanced("a1", 1, 2);
    assert!(matches!(
        rx.try_recv(),
        Ok(Command::Sync { since_rev: 1, .. })
    ));
}

#[tokio::test]
async fn issue_1605_failed_sync_cannot_establish_capability_or_project_data() {
    let mut h = super::tui_harness::TuiHarness::new().await;
    let (feed, _rx) = feed_with_rx();
    let app = h.app_mut();
    app.ac_mut().roster.feeds.insert("a1".into(), feed);
    app.ensure_session("a1");
    app.route_subagent_event(
        "a1",
        Event::Response {
            id: None,
            command: "sync".into(),
            success: false,
            data: Some(sync_delta(1, 1)),
            error: Some("failed".into()),
        },
    );
    assert!(!app.ac().roster.feeds["a1"].supports_sync);
    assert_eq!(app.ac().roster.feeds["a1"].rev, 0);
    assert_eq!(app.ac().roster.sessions["a1"].chat.entry_count(), 0);
}

#[tokio::test]
async fn issue_1605_delayed_older_refresh_cannot_roll_back_applied_transcript() {
    let mut h = super::tui_harness::TuiHarness::new().await;
    let (feed, _rx) = feed_with_rx();
    let app = h.app_mut();
    app.ac_mut().roster.feeds.insert("a1".into(), feed);
    app.ensure_session("a1");
    app.route_sync_response(
        "a1",
        &json!({"epoch":1,"rev":9,"messages":[
        {"id":"m1","role":"user","content":"new value"}],
        "caughtUp":true,"nextRev":null,"resync":false}),
    );
    app.route_sync_response("a1", &sync_delta(1, 1));
    assert_eq!(app.ac().roster.feeds["a1"].rev, 9);
    assert!(matches!(app.ac().roster.sessions["a1"].chat.entries(),
        [ChatEntry::User { text }] if text == "new value"));
}

#[tokio::test]
async fn issue_1605_periodic_refresh_uses_each_tabs_namespace_and_cursor() {
    use crate::shell::connection::TabId;
    let mut h = super::tui_harness::TuiHarness::new().await;
    h.open_background_tab();
    let mut receivers = Vec::new();
    for tab in 0..2 {
        let (mut feed, rx) = feed_with_rx();
        feed.epoch = 3 + u64::from(tab);
        feed.rev = 8 + u64::from(tab);
        h.app_mut()
            .conn_mut(TabId(tab))
            .unwrap()
            .roster
            .feeds
            .insert("same-child".into(), feed);
        receivers.push(rx);
    }
    service_without_new_hint(&mut h).await;
    for (tab, rx) in receivers.iter_mut().enumerate() {
        let command = rx.try_recv().expect("periodic refresh on each owning tab");
        assert!(
            matches!(command, Command::Sync { id: Some(id), epoch, since_rev, .. }
            if id == format!("tab{tab}:subagent-sync") && epoch == 3 + tab as u64 && since_rev == 8 + tab as u64)
        );
    }
    assert_eq!(h.active_tab_index(), 0);
}
