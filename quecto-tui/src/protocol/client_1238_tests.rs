//! #1238 — command bursts from subagent fan-in must not reject user follow-ups.
//!
//! Loaded from `client.rs` via `#[path = "client_1238_tests.rs"]`.
//!
//! These tests pin the production constants
//! ([`COMMAND_WRITER_QUEUE_CAPACITY`], [`COMMAND_WRITER_USER_RESERVED`]) and
//! the reserve semantics in [`CommandSender::try_send`]. Lowering capacity or
//! removing the user reserve fails them.

use super::*;

fn production_queue_sender() -> (CommandSender, mpsc::Receiver<String>) {
    let (tx, rx) = mpsc::channel::<String>(COMMAND_WRITER_QUEUE_CAPACITY);
    (CommandSender { tx }, rx)
}

#[tokio::test]
async fn background_burst_fills_up_to_user_reserve_only() {
    let (sender, _rx) = production_queue_sender();
    let background = Command::GetState { id: None };
    let background_budget = COMMAND_WRITER_QUEUE_CAPACITY - COMMAND_WRITER_USER_RESERVED;

    for index in 0..background_budget {
        sender.try_send(&background).unwrap_or_else(|err| {
            panic!("background command {index} should enqueue within budget: {err}")
        });
    }

    let err = sender
        .try_send(&background)
        .expect_err("background must not consume reserved user slots");
    assert!(
        matches!(err, ClientError::Backpressure),
        "expected Backpressure, got {err}"
    );
}

#[tokio::test]
async fn user_follow_up_survives_background_filled_to_reserve() {
    let (sender, _rx) = production_queue_sender();
    let background = Command::GetState { id: None };
    let follow_up = Command::FollowUp {
        id: None,
        message: "keep going".into(),
    };
    let background_budget = COMMAND_WRITER_QUEUE_CAPACITY - COMMAND_WRITER_USER_RESERVED;

    for index in 0..background_budget {
        sender.try_send(&background).unwrap_or_else(|err| {
            panic!("background command {index} should enqueue within budget: {err}")
        });
    }

    sender
        .try_send(&follow_up)
        .expect("user follow-up must use reserved headroom after background fan-in");

    // Additional interactive user commands may still use remaining reserved slots.
    sender
        .try_send(&Command::Abort { id: None })
        .expect("abort is interactive user and must share reserved headroom");
}

#[tokio::test]
async fn fully_full_queue_rejects_user_commands_too() {
    let (sender, _rx) = production_queue_sender();
    let follow_up = Command::FollowUp {
        id: None,
        message: "x".into(),
    };

    // Fill every slot with interactive commands (they may use the reserve).
    for index in 0..COMMAND_WRITER_QUEUE_CAPACITY {
        sender.try_send(&follow_up).unwrap_or_else(|err| {
            panic!("user command {index} should fill the full capacity: {err}")
        });
    }

    let err = sender
        .try_send(&follow_up)
        .expect_err("truly full queue must still backpressure user commands");
    assert!(
        matches!(err, ClientError::Backpressure),
        "expected Backpressure, got {err}"
    );
}

#[test]
fn user_reserve_is_strictly_inside_capacity() {
    // Compile-time pins so a zero/overflowing reserve fails the build, not only runtime.
    const {
        assert!(COMMAND_WRITER_USER_RESERVED > 0);
        assert!(COMMAND_WRITER_USER_RESERVED < COMMAND_WRITER_QUEUE_CAPACITY);
    }
}

#[tokio::test]
async fn small_closed_channel_still_reports_disconnected_not_false_backpressure() {
    // Mirrors Client::disconnected_for_tests (capacity 1, rx dropped). The
    // user-reserve gate must not fire on undersized queues or send failures
    // mis-report as "command queue full" instead of disconnected.
    let (tx, rx) = mpsc::channel::<String>(1);
    drop(rx);
    let sender = CommandSender { tx };
    let err = sender
        .try_send(&Command::GetState { id: None })
        .expect_err("closed channel must fail");
    assert!(
        matches!(err, ClientError::Disconnected),
        "expected Disconnected, got {err}"
    );
}

#[test]
fn interactive_user_command_kinds() {
    assert!(
        Command::Prompt {
            id: None,
            message: "hi".into(),
            streaming_behavior: None,
        }
        .is_interactive_user()
    );
    assert!(
        Command::Steer {
            id: None,
            message: "nudge".into(),
        }
        .is_interactive_user()
    );
    assert!(
        Command::FollowUp {
            id: None,
            message: "more".into(),
        }
        .is_interactive_user()
    );
    assert!(Command::Abort { id: None }.is_interactive_user());
    assert!(!Command::GetState { id: None }.is_interactive_user());
    assert!(!Command::GetSubagents { id: None }.is_interactive_user());
    assert!(
        !Command::Sync {
            id: None,
            epoch: 0,
            since_rev: 0,
        }
        .is_interactive_user()
    );
}

// ── Feed-liveness exemption (child-progress freeze fix, 2026-07-29) ──────────
//
// `Command::Sync` is the child-feed refresh path. Refusing it under the
// background reserve froze child feeds exactly when the parent was busy (the
// only time the queue approaches the reserve), so Sync bypasses the reserve —
// while remaining subject to true full-queue backpressure.

#[tokio::test]
async fn sync_may_use_the_outer_reserve_but_not_the_interactive_floor() {
    let (sender, _rx) = production_queue_sender();
    let background = Command::GetState { id: None };
    let sync = Command::Sync {
        id: None,
        epoch: 1,
        since_rev: 0,
    };
    let background_budget = COMMAND_WRITER_QUEUE_CAPACITY - COMMAND_WRITER_USER_RESERVED;

    for _ in 0..background_budget {
        sender.try_send(&background).expect("within budget");
    }
    // Background traffic is refused at the reserve…
    assert!(matches!(
        sender.try_send(&background),
        Err(ClientError::Backpressure)
    ));
    // …a feed-liveness Sync may use the OUTER half of the reserve…
    for _ in 0..(COMMAND_WRITER_USER_RESERVED - COMMAND_WRITER_INTERACTIVE_FLOOR) {
        sender
            .try_send(&sync)
            .expect("Sync may use the outer reserve — refusing it freezes the child feed");
    }
    // …but never the interactive floor (PR #1307 review: an unthrottled sync
    // burst must not consume the slots protecting prompt/steer/abort).
    assert!(matches!(
        sender.try_send(&sync),
        Err(ClientError::Backpressure)
    ));
    // Interactive commands still enqueue from the protected floor.
    sender
        .try_send(&Command::Abort { id: None })
        .expect("the interactive floor belongs to user commands");
}

#[test]
fn interactive_floor_is_strictly_inside_the_reserve() {
    const {
        assert!(COMMAND_WRITER_INTERACTIVE_FLOOR > 0);
        assert!(COMMAND_WRITER_INTERACTIVE_FLOOR < COMMAND_WRITER_USER_RESERVED);
    }
}

#[tokio::test]
async fn sync_alone_can_never_fill_past_the_interactive_floor() {
    let (sender, _rx) = production_queue_sender();
    let sync = Command::Sync {
        id: None,
        epoch: 1,
        since_rev: 0,
    };
    let sync_budget = COMMAND_WRITER_QUEUE_CAPACITY - COMMAND_WRITER_INTERACTIVE_FLOOR;

    for index in 0..sync_budget {
        sender
            .try_send(&sync)
            .unwrap_or_else(|err| panic!("sync {index} should fill to the floor: {err}"));
    }

    assert!(
        matches!(sender.try_send(&sync), Err(ClientError::Backpressure)),
        "sync stops at the interactive floor even with the queue otherwise unbounded"
    );
    // The floor still admits interactive commands to true fullness.
    for _ in 0..COMMAND_WRITER_INTERACTIVE_FLOOR {
        sender
            .try_send(&Command::Abort { id: None })
            .expect("interactive fills the protected floor");
    }
    assert!(matches!(
        sender.try_send(&Command::Abort { id: None }),
        Err(ClientError::Backpressure)
    ));
}

#[test]
fn feed_liveness_command_kinds() {
    assert!(
        Command::Sync {
            id: None,
            epoch: 0,
            since_rev: 0,
        }
        .is_feed_liveness()
    );
    assert!(!Command::GetState { id: None }.is_feed_liveness());
    assert!(!Command::GetSubagents { id: None }.is_feed_liveness());
    assert!(
        !Command::Prompt {
            id: None,
            message: "hi".into(),
            streaming_behavior: None,
        }
        .is_feed_liveness()
    );
}
