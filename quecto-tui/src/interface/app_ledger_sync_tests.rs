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
            transcript: crate::interface::ledger_sync::LedgerTranscript::default(),
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
    app.note_ledger_advanced("a1", 1, 10);

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
    assert_eq!(feed.rev, 12);
    assert!(feed.last_fresh_at.is_some());
    assert_eq!(app.subagents.sessions["a1"].chat.entry_count(), 1);
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
async fn sync_response_for_wrong_epoch_is_ignored() {
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
