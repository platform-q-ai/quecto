//! The output side of a claude member process (#2286): lines it cannot
//! read become skipped-line events, the buffer ahead of a reader that
//! stops is bounded in bytes, and waiting for the exit never hangs on
//! unread output.

use std::time::Duration;

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
async fn waiting_for_the_exit_drains_output_no_one_reads() {
    let rig = small_limits(&flood(1000));
    let process = rig.start().await;
    process.send_user_turn("go").await.unwrap();
    process.close_input().await;
    let exit = tokio::time::timeout(BOUND, process.exited())
        .await
        .expect("exited() does not hang on output no one reads");
    assert_eq!(exit, ExternalAgentExit::Code(0));
    assert_eq!(rig.mock.recorded("record"), vec!["done".to_string()]);
}
