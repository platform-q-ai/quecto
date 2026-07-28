//! #1238 — command bursts from subagent fan-in must not reject user follow-ups.
//!
//! Loaded from `client.rs` via `#[path = "client_1238_tests.rs"]`.

use super::*;

#[tokio::test]
async fn command_sender_accepts_large_bursts_without_backpressure() {
    let (tx, _rx) = mpsc::channel::<String>(4096);
    let sender = CommandSender { tx };
    let cmd = Command::GetState { id: None };

    for index in 0..2048 {
        sender
            .try_send(&cmd)
            .unwrap_or_else(|err| panic!("command {index} should enqueue during a burst: {err}"));
    }
}
