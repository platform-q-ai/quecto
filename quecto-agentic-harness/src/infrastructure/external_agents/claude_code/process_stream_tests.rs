//! The output side of a claude member process (#2286): lines it cannot
//! read become skipped-line events, the buffer ahead of a reader that
//! stops is bounded in bytes, waiting for the exit consumes no output,
//! waiting while discarding never hangs on unread output, and the exit
//! carries stderr's last words, waiting for them only within a bound.

use std::time::{Duration, Instant};

use super::test_rig::{BOUND, RESULT_LINE, Rig, events_to_turn_end, finish};
use crate::application::external_agent::dto::ExternalAgentExit;
use crate::domain::external_agent::stream::{ExternalAgentEvent, SkippedLine, SkippedLineReason};

/// A small line cap and buffer, so a test's scenario stays small.
const LINE_CAP: usize = 2048;
const BUFFER_BYTES: usize = 8 * 1024;

fn small_limits(scenario: &str) -> Rig {
    Rig::build(scenario.as_bytes(), &[], |launcher| {
        launcher.with_stream_limits(LINE_CAP, BUFFER_BYTES)
    })
}

/// One `thinking_tokens` line padded to about `bytes`.
fn thinking_line(bytes: usize) -> String {
    format!(
        "{{\"type\": \"system\", \"subtype\": \"thinking_tokens\", \"estimated_tokens\": 5, \"pad\": \"{}\"}}",
        "x".repeat(bytes.saturating_sub(90))
    )
}

/// `lines` thinking lines of about 1 KiB, then `@record done`, then the
/// turn's result.
fn flood(lines: usize) -> String {
    let mut scenario = String::new();
    for _ in 0..lines {
        scenario.push_str(&thinking_line(1024));
        scenario.push('\n');
    }
    scenario.push_str("@record done\n");
    scenario.push_str(RESULT_LINE);
    scenario
}

#[tokio::test]
async fn a_line_over_the_cap_or_not_utf8_is_a_skipped_line_and_the_stream_goes_on() {
    let long = thinking_line(LINE_CAP + 100);
    let short = thinking_line(200);
    let scenario = format!("{long}\n@printf \\377\\376{{}}\\n\n{short}\n{RESULT_LINE}");
    let rig = small_limits(&scenario);
    let process = rig.start().await;
    process.send_user_turn("go").await.unwrap();
    let events = events_to_turn_end(process.as_ref()).await;
    assert_eq!(
        events[..3],
        [
            ExternalAgentEvent::LineSkipped(SkippedLine {
                reason: SkippedLineReason::OverCap,
                bytes: long.len() + 1,
            }),
            ExternalAgentEvent::LineSkipped(SkippedLine {
                reason: SkippedLineReason::NotUtf8,
                bytes: 5,
            }),
            ExternalAgentEvent::ThinkingTokens {
                estimated_tokens: Some(5)
            },
        ],
        "{events:?}"
    );
    assert!(matches!(events[3], ExternalAgentEvent::Result(_)));
    assert_eq!(finish(process.as_ref()).await, ExternalAgentExit::Code(0));
}

#[tokio::test]
async fn a_reader_that_stops_holds_the_child_within_the_byte_budget() {
    // 280 KiB of output: far more than the budget, the pipe and awk's
    // buffer hold, far fewer lines than an event-count buffer would.
    let lines = 280;
    let rig = small_limits(&flood(lines));
    let process = rig.start().await;
    process.send_user_turn("go").await.unwrap();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(
        rig.mock.recorded("record"),
        Vec::<String>::new(),
        "the child is held while no one reads"
    );
    let events = events_to_turn_end(process.as_ref()).await;
    assert_eq!(events.len(), lines + 1);
    assert_eq!(rig.mock.recorded("record"), vec!["done".to_string()]);
    assert_eq!(finish(process.as_ref()).await, ExternalAgentExit::Code(0));
}

#[tokio::test]
async fn waiting_for_the_exit_while_discarding_does_not_hang_on_output_no_one_reads() {
    let rig = small_limits(&flood(1000));
    let process = rig.start().await;
    process.send_user_turn("go").await.unwrap();
    process.close_input().await;
    let exit = tokio::time::timeout(BOUND, process.exited_discarding_output())
        .await
        .expect("exited_discarding_output() does not hang on output no one reads");
    assert_eq!(exit, ExternalAgentExit::Code(0));
    assert_eq!(rig.mock.recorded("record"), vec!["done".to_string()]);
}

/// A session loop that reads events until the process exits, then reads
/// what is left: `exited()` consumes none of the output, so every event
/// reaches the loop, the turn's `result` included (#2286 review round 3).
#[tokio::test]
async fn a_select_loop_over_events_and_the_exit_loses_no_event() {
    let lines = 200;
    let rig = small_limits(&flood(lines));
    let process = rig.start().await;
    process.send_user_turn("go").await.unwrap();
    process.close_input().await;
    let mut events = Vec::new();
    let exit = tokio::time::timeout(BOUND, async {
        let exit = loop {
            tokio::select! {
                event = process.next_event() => match event {
                    Some(event) => events.push(event),
                    None => break process.exited().await,
                },
                exit = process.exited() => break exit,
            }
        };
        while let Some(event) = process.next_event().await {
            events.push(event);
        }
        exit
    })
    .await
    .expect("the loop ends within the bound");
    assert_eq!(exit, ExternalAgentExit::Code(0));
    assert_eq!(events.len(), lines + 1, "every event reached the loop");
    assert!(
        matches!(events.last(), Some(ExternalAgentEvent::Result(_))),
        "the turn's result reached the loop"
    );
}

/// A rig whose exits wait at most `grace` for stderr's end.
fn with_grace(scenario: &str, grace: Duration) -> Rig {
    Rig::build(scenario.as_bytes(), &[], |launcher| {
        launcher.with_stderr_eof_grace(grace)
    })
}

/// Last words a descendant writes to stderr after `claude` itself exited
/// are in the tail the moment the exit is returned (#2286 swarm review):
/// either wait reads stderr to its end, within the grace, first.
#[tokio::test]
async fn the_exit_is_returned_with_stderrs_last_words_in_the_tail() {
    for discarding in [false, true] {
        let rig = with_grace("@stderr-after 0.3 late-words\n", Duration::from_secs(5));
        let process = rig.start().await;
        process.close_input().await;
        let exited = if discarding {
            process.exited_discarding_output()
        } else {
            process.exited()
        };
        let exit = tokio::time::timeout(BOUND, exited)
            .await
            .expect("exit is bounded");
        assert_eq!(exit, ExternalAgentExit::Code(0));
        assert_eq!(
            process.stderr_tail(),
            "late-words",
            "discarding: {discarding}"
        );
    }
}

/// Kills the lingering grandchild when dropped, so a failed assertion
/// cannot leak it.
struct KillOnDrop(u32);

impl KillOnDrop {
    /// The grandchild the mock recorded: a real pid, never 0 or 1 (which
    /// `kill` would read as a group or init).
    fn of(recorded: &str) -> Self {
        let pid: u32 = recorded
            .trim()
            .parse()
            .unwrap_or_else(|e| panic!("the grandchild's pid {recorded:?}: {e}"));
        assert!(pid > 1, "the grandchild is a real process, not {pid}");
        Self(pid)
    }
}

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        // KILL: the grandchild is a sleep; nothing of it is worth a clean
        // exit, and KILL dumps no core.
        let _ = std::process::Command::new("kill")
            .args(["-KILL", &self.0.to_string()])
            .status();
    }
}

/// A descendant holding stderr open cannot hold the exit past the grace.
#[tokio::test]
async fn a_descendant_holding_stderr_open_delays_the_exit_by_the_grace_only() {
    let grace = Duration::from_millis(300);
    let rig = with_grace("@linger 30\n", grace);
    let process = rig.start().await;
    let grandchild =
        KillOnDrop::of(&super::test_rig::recorded_at_start(&rig.mock, "grandchild").await);
    process.close_input().await;
    let started = Instant::now();
    let exit = tokio::time::timeout(BOUND, process.exited())
        .await
        .expect("exit is bounded");
    let waited = started.elapsed();
    drop(grandchild);
    assert_eq!(exit, ExternalAgentExit::Code(0));
    assert!(
        waited < grace + Duration::from_secs(3),
        "the exit waited {waited:?}, the grace is {grace:?}"
    );
}
