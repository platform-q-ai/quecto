//! The input side of a claude member process (#2286): one whole line per
//! turn, cancel-safe sends, and a close after which nothing is written.

use std::time::Duration;

use futures::StreamExt;
use futures::stream::FuturesUnordered;
use serde_json::json;

use super::INPUT_QUEUE;
use super::test_rig::{
    BOUND, FALLBACK_BOUND, Rig, events_to_turn_end, finish, turns_read, until_retired,
};
use crate::application::external_agent::dto::{ExternalAgentExit, ExternalAgentInputError};

#[tokio::test]
async fn a_user_turn_is_written_as_one_stream_json_user_message() {
    let rig = Rig::new("{\"type\": \"result\", \"subtype\": \"success\"}\n");
    let process = rig.start().await;
    process.send_user_turn("hello\nworld").await.unwrap();
    events_to_turn_end(process.as_ref()).await;
    finish(process.as_ref()).await;
    let input = rig.mock.recorded("input");
    assert_eq!(input.len(), 1, "one line per turn: {input:?}");
    let sent: serde_json::Value = serde_json::from_str(&input[0]).unwrap();
    assert_eq!(
        sent,
        json!({"type": "user", "message": {"role": "user", "content": [
            {"type": "text", "text": "hello\nworld"}
        ]}})
    );
}

#[tokio::test]
async fn a_cancelled_turn_never_leaves_half_a_line() {
    // The mock reads nothing for a while, so a turn larger than the pipe
    // is still being written when its send is cancelled.
    let rig = Rig::new("@stall-input 2\n");
    let process = rig.start().await;
    let big = "a".repeat(256 * 1024);
    let cancelled =
        tokio::time::timeout(Duration::from_millis(100), process.send_user_turn(&big)).await;
    assert!(
        cancelled.is_err(),
        "the send was still writing: {cancelled:?}"
    );
    process.send_user_turn("after").await.unwrap();
    assert_eq!(finish(process.as_ref()).await, ExternalAgentExit::Code(0));
    let texts = turns_read(&rig.mock);
    assert_eq!(texts.len(), 2, "each turn is its own whole line");
    assert_eq!(texts[0], big, "an accepted turn is written whole");
    assert_eq!(texts[1], "after");
}

#[tokio::test]
async fn closing_the_input_does_not_wait_behind_a_stuck_write() {
    // The mock never reads: a turn larger than the pipe never finishes.
    let rig = Rig::new("@stall-input 3600\n");
    let process = rig.start().await;
    let big = "a".repeat(256 * 1024);
    {
        let send = process.send_user_turn(&big);
        tokio::pin!(send);
        let pending = tokio::time::timeout(Duration::from_millis(200), &mut send).await;
        assert!(pending.is_err(), "the write is stuck: {pending:?}");
        tokio::time::timeout(Duration::from_secs(2), process.close_input())
            .await
            .expect("close_input returns while a write is stuck");
    }
    // Bounded: were the input left open, the late turn would queue behind
    // the stuck write for good.
    let late = tokio::time::timeout(Duration::from_secs(2), process.send_user_turn("late"))
        .await
        .expect("a turn sent after the close is answered at once");
    assert_eq!(late, Err(ExternalAgentInputError::Closed));
    drop(process);
    until_retired(&rig.supervisor, FALLBACK_BOUND).await;
}

#[tokio::test]
async fn no_line_waiting_at_the_close_is_written_after_it() {
    // The mock reads nothing for a while: the big turn is stuck mid-write,
    // the next turns fill the queue, and the last ones wait for space.
    let rig = Rig::new("@stall-input 3\n");
    let process = rig.start().await;
    let big = "a".repeat(256 * 1024);
    let mut first = Box::pin(process.send_user_turn(&big));
    let stuck = tokio::time::timeout(Duration::from_millis(200), &mut first).await;
    assert!(stuck.is_err(), "the big turn is stuck mid-write: {stuck:?}");
    let waiting_for_space = 4;
    let texts: Vec<String> = (0..INPUT_QUEUE + waiting_for_space)
        .map(|n| format!("later-{n}"))
        .collect();
    let mut later: FuturesUnordered<_> = texts
        .iter()
        .map(|text| process.send_user_turn(text))
        .collect();
    let none = tokio::time::timeout(Duration::from_millis(200), later.next()).await;
    assert!(none.is_err(), "no later turn is answered before the close");
    process.close_input().await;
    for _ in 0..waiting_for_space {
        let outcome = tokio::time::timeout(Duration::from_secs(1), later.next())
            .await
            .expect("a sender waiting for queue space is answered at the close")
            .expect("a send");
        assert_eq!(outcome, Err(ExternalAgentInputError::Closed));
    }
    let (first, rest) = tokio::time::timeout(BOUND, async {
        let rest: Vec<_> = later.collect().await;
        (first.await, rest)
    })
    .await
    .expect("every send is answered once the stuck write lands");
    assert_eq!(first, Ok(()), "the line in flight at the close lands whole");
    assert_eq!(rest.len(), INPUT_QUEUE);
    assert!(
        rest.iter()
            .all(|outcome| *outcome == Err(ExternalAgentInputError::Closed)),
        "a queued line is not written after the close: {rest:?}"
    );
    assert_eq!(finish(process.as_ref()).await, ExternalAgentExit::Code(0));
    assert_eq!(turns_read(&rig.mock), vec![big]);
}
