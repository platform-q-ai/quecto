use super::tui_harness::TuiHarness;
use crate::agents::feed::FeedAuthority;
use crate::agents::view::FeedState;
use crate::protocol::client::{Command, Event, SubagentInfoEvent};
use tokio::sync::{mpsc, oneshot};
use tokio::time::{Duration, timeout};

async fn harness() -> TuiHarness {
    TuiHarness::new().await
}

fn subagent(id: &str) -> SubagentInfoEvent {
    SubagentInfoEvent {
        agent_uuid: None,
        display_name: None,
        agent_id: id.into(),
        status: "running".into(),
        last_tool: None,
        last_error: None,
        pid: 1,
        socket_path: None,
        parent_id: None,
        read_only: false,
        workflow: None,
        execution_backend: None,
        environment: None,
    }
}

#[tokio::test]
async fn handle_submit_delete_all_subagents_sends_command_and_clears_ui() {
    let mut h = harness().await;
    h.app_mut().handle_event(Event::SubagentStateChanged {
        subagents: vec![subagent("worker")],
    });

    h.app_mut().handle_submit("/delete-all-subagents");

    let cmds = h.drain_commands().await;
    assert_eq!(cmds.len(), 1, "expected one agent command: {cmds:?}");
    assert!(
        cmds[0].contains("\"type\":\"delete_all_subagents\""),
        "slash command should send delete_all_subagents: {cmds:?}"
    );
    assert_eq!(
        h.subagent_group_tracked(),
        0,
        "subagent panel should be cleared optimistically"
    );
}

#[tokio::test]
async fn handle_submit_delete_all_subagents_preserves_ui_when_command_send_fails() {
    let mut h = harness().await;
    h.app_mut().handle_event(Event::SubagentStateChanged {
        subagents: vec![subagent("worker")],
    });
    h.disconnect_master_commands();

    h.app_mut().handle_submit("/delete-all-subagents");

    let failure = h
        .app_mut()
        .command_send_failure_rx
        .recv()
        .await
        .expect("delete-all-subagents send failure should be reported");
    h.app_mut().handle_command_send_failure(failure);
    assert_eq!(
        h.subagent_group_tracked(),
        1,
        "subagent panel must not be cleared when delete command was not enqueued"
    );
}

fn feed_from_handle(handle: tokio::task::JoinHandle<()>) -> FeedState {
    let (cmd_tx, _cmd_rx) = mpsc::channel(1);
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
        authority: FeedAuthority::WarmSync,
    }
}

fn pending_feed() -> (FeedState, oneshot::Receiver<()>) {
    let (drop_tx, drop_rx) = oneshot::channel();
    struct DropSignal(Option<oneshot::Sender<()>>);
    impl Drop for DropSignal {
        fn drop(&mut self) {
            if let Some(tx) = self.0.take() {
                let _ = tx.send(());
            }
        }
    }
    let handle = tokio::spawn(async move {
        let _signal = DropSignal(Some(drop_tx));
        std::future::pending::<()>().await;
    });
    (feed_from_handle(handle), drop_rx)
}

#[tokio::test]
async fn successful_delete_all_aborts_all_existing_feed_tasks() {
    let mut h = harness().await;
    let (selected_feed, selected_done) = pending_feed();
    let (inactive_feed, inactive_done) = pending_feed();
    h.app_mut()
        .subagents
        .feeds
        .insert("selected".into(), selected_feed);
    h.app_mut()
        .subagents
        .feeds
        .insert("inactive".into(), inactive_feed);
    h.app_mut().subagents.active_agent_id = Some("selected".into());

    h.app_mut().handle_submit("/delete-all-subagents");

    assert!(
        timeout(Duration::from_millis(200), selected_done)
            .await
            .is_ok(),
        "selected feed should be aborted before drop"
    );
    assert!(
        timeout(Duration::from_millis(200), inactive_done)
            .await
            .is_ok(),
        "inactive feed should be aborted before drop"
    );
}

#[tokio::test]
async fn failed_delete_all_preserves_feed_task() {
    let mut h = harness().await;
    let (feed, done_rx) = pending_feed();
    h.app_mut().subagents.feeds.insert("worker".into(), feed);
    h.disconnect_master_commands();

    h.app_mut().handle_submit("/delete-all-subagents");

    let failure = h.app_mut().command_send_failure_rx.recv().await.unwrap();
    h.app_mut().handle_command_send_failure(failure);
    let feed = h
        .app_mut()
        .subagents
        .feeds
        .get("worker")
        .expect("feed preserved");
    assert!(
        !feed.handle.is_finished(),
        "failed delete-all must not abort feed"
    );
    assert!(
        timeout(Duration::from_millis(20), done_rx).await.is_err(),
        "failed delete-all must not abort feed"
    );
}

#[tokio::test]
async fn successful_delete_all_cancels_feed_blocked_on_bounded_fan_in_send() {
    let mut h = harness().await;
    let (cmd_tx, _cmd_rx) = mpsc::channel::<Command>(1);
    let (fan_tx, mut fan_rx) = mpsc::channel::<Event>(1);
    fan_tx
        .try_send(Event::Token {
            token: "prefill".into(),
        })
        .expect("prefill fan-in");
    let (blocked_tx, mut blocked_rx) = mpsc::channel(1);
    let (done_tx, done_rx) = oneshot::channel();
    struct DropSignal(Option<oneshot::Sender<()>>);
    impl Drop for DropSignal {
        fn drop(&mut self) {
            if let Some(tx) = self.0.take() {
                let _ = tx.send(());
            }
        }
    }
    let handle = tokio::spawn(async move {
        let _signal = DropSignal(Some(done_tx));
        let send = fan_tx.send(Event::Token {
            token: "blocked".into(),
        });
        tokio::pin!(send);
        tokio::select! {
            _ = &mut send => {},
            _ = async { let _ = blocked_tx.send(()).await; std::future::pending::<()>().await } => {},
        }
    });
    let mut feed = feed_from_handle(handle);
    feed.cmd_tx = cmd_tx;
    h.app_mut().subagents.feeds.insert("blocked".into(), feed);
    blocked_rx
        .recv()
        .await
        .expect("task reached blocked send path");

    h.app_mut().handle_submit("/delete-all-subagents");

    assert!(
        timeout(Duration::from_millis(200), done_rx).await.is_ok(),
        "blocked fan-in feed task should be cancelled before drop"
    );
    assert!(
        fan_rx.try_recv().is_ok(),
        "prefilled event remains until after cancellation proof"
    );
}
