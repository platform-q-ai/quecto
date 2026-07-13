//! #926: an idle parent (blocked in `recv_next_message`) must WAKE on a single
//! child completion notification. This exercises the `tokio::select!` wake half
//! of the path — the act half (drain → parent turn) is covered in `uds.rs`.
use super::*;
use crate::infrastructure::tools::subagent_registry::{
    SequencedSubagentNotification, SubagentNotification, new_notification_channel,
};

/// With a live `notification_rx` and no client traffic, a child completion
/// queued onto the notify channel wakes the idle `recv_next_message` and is
/// surfaced as a `DispatchMsg::Notification` — the parent will then act on it.
#[tokio::test]
async fn test_926_idle_parent_wakes_on_single_completion() {
    // No client messages will arrive: the only thing that can wake the parent
    // is the completion notification.
    let (_cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<ClientMessage>(4);
    let (notify_tx, notify_rx) = new_notification_channel();
    let mut notification_rx = Some(notify_rx);

    notify_tx
        .send(SequencedSubagentNotification::new(
            1,
            SubagentNotification::Completed {
                agent_id: "researcher".to_string(),
            },
        ))
        .await
        .unwrap();

    let msg = recv_next_message(&mut cmd_rx, &mut notification_rx).await;
    match msg {
        Some(DispatchMsg::Notification(notif)) => {
            let (agent_id, sequence) = notif.dedupe_key();
            assert_eq!(agent_id, "researcher");
            assert_eq!(sequence, 1);
        }
        Some(DispatchMsg::Client(_)) => panic!("woke on a client message, not the completion"),
        None => panic!("idle parent must wake on the completion, channel closed instead"),
    }
}

/// The wake gap shape: if the receiver is dropped (`None`) — exactly what the
/// old `has_base_dir` gate did while `notify_tx` stayed wired — the parent only
/// ever wakes on client traffic and a completion can never wake it. Asserting
/// this documents WHY the receiver must stay live (#926).
#[tokio::test]
async fn test_926_dropped_rx_means_completion_cannot_wake_parent() {
    let (_cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<ClientMessage>(4);
    let mut notification_rx: Option<
        crate::infrastructure::tools::subagent_registry::NotificationRx,
    > = None;

    // With no rx and no client messages, recv_next_message blocks forever;
    // bound it so the test proves "no wake" instead of hanging.
    let res = tokio::time::timeout(
        std::time::Duration::from_millis(50),
        recv_next_message(&mut cmd_rx, &mut notification_rx),
    )
    .await;
    assert!(
        res.is_err(),
        "a dropped notification rx leaves the idle parent unwakeable by completions"
    );
}
