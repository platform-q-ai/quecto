use super::*;
use serde_json::json;

fn feed_with_rx() -> (FeedState, mpsc::Receiver<Command>) {
    let (cmd_tx, cmd_rx) = mpsc::channel(8);
    let handle = tokio::spawn(async {});
    (
        FeedState {
            cmd_tx,
            handle,
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
    app.subagents.feeds.insert("a1".into(), feed);

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
    app.subagents.feeds.insert("a1".into(), feed);
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

    let feed = app.subagents.feeds.get("a1").unwrap();
    assert_eq!(feed.epoch, 3);
    assert_eq!(feed.rev, 8);
    assert!(feed.last_fresh_at.is_some());
    let entries = app.subagents.sessions["a1"].chat.entries();
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
    app.subagents.feeds.insert("a1".into(), feed);

    app.note_sync_capability("a1", &json!({"sync":1}));

    assert_eq!(
        app.subagents.feeds["a1"].authority,
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
    app.subagents.feeds.insert("a1".into(), feed);
    app.ensure_session("a1");

    app.route_sync_response("a1", &sync_delta(1, 1));

    assert_eq!(
        app.subagents.feeds["a1"].authority,
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
    app.subagents.feeds.insert("a1".into(), feed);
    app.ensure_session("a1");

    app.route_sync_response("a1", &sync_delta(3, 9));

    let feed = app.subagents.feeds.get("a1").unwrap();
    assert_eq!(feed.epoch, 2);
    assert_eq!(feed.rev, 5);
    assert_eq!(app.subagents.sessions["a1"].chat.entry_count(), 0);
}

#[tokio::test]
async fn epoch_mismatch_resync_replaces_stale_synced_transcript() {
    let mut h = super::tui_harness::TuiHarness::new().await;
    let app = h.app_mut();
    let (mut feed, _rx) = feed_with_rx();
    feed.epoch = 2;
    feed.rev = 5;
    feed.supports_sync = true;
    app.subagents.feeds.insert("a1".into(), feed);
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

    let feed = app.subagents.feeds.get("a1").unwrap();
    assert_eq!(feed.epoch, 3);
    assert_eq!(feed.rev, 1);
    let entries = app.subagents.sessions["a1"].chat.entries();
    assert!(matches!(entries, [ChatEntry::User { text }] if text == "fresh session"));
}

// ── Refused sync must not strand the feed (child-progress freeze fix) ────────

fn full_channel_feed() -> (FeedState, mpsc::Receiver<Command>) {
    // Capacity-1 channel, prefilled: the next try_send is refused.
    let (cmd_tx, cmd_rx) = mpsc::channel(1);
    cmd_tx
        .try_send(Command::GetState { id: None })
        .expect("prefill");
    let handle = tokio::spawn(async {});
    (
        FeedState {
            cmd_tx,
            handle,
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
    app.subagents.feeds.insert("a1".into(), feed);

    app.note_ledger_advanced("a1", 1, 9);

    assert_eq!(
        app.subagents.feeds["a1"].pending_rev, None,
        "a refused Sync marked in-flight is a phantom sync that never resolves"
    );
}

#[tokio::test]
async fn next_ledger_hint_retries_after_a_refused_sync() {
    let mut h = super::tui_harness::TuiHarness::new().await;
    let app = h.app_mut();
    let (feed, mut rx) = full_channel_feed();
    app.subagents.feeds.insert("a1".into(), feed);

    // First hint: channel full, refused, nothing recorded in-flight.
    app.note_ledger_advanced("a1", 1, 9);
    assert_eq!(app.subagents.feeds["a1"].pending_rev, None);

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
    assert_eq!(app.subagents.feeds["a1"].pending_rev, Some(10));
}

#[tokio::test]
async fn accepted_sync_still_records_pending_rev() {
    let mut h = super::tui_harness::TuiHarness::new().await;
    let app = h.app_mut();
    let (mut feed, mut rx) = feed_with_rx();
    feed.supports_sync = true;
    app.subagents.feeds.insert("a1".into(), feed);

    app.note_ledger_advanced("a1", 1, 9);

    assert!(rx.try_recv().is_ok(), "sync sent on open channel");
    assert_eq!(app.subagents.feeds["a1"].pending_rev, Some(9));
}
