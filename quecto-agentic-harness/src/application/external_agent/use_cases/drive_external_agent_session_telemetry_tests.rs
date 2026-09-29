//! What the member session measures for its event log (#2304): each tool
//! call once answered, each turn as it ended (its charge the growth of the
//! process's cumulative cost), the process's first init, what the stream
//! said that could not be read, and the process's end.

use std::time::Duration;

use super::test_rig::*;
use crate::application::external_agent::dto::SessionRecord;
use crate::domain::external_agent::stream::{
    AssistantContent, ExternalAgentEvent, InitEvent, ResultEvent, TokenCounts, ToolResultEvent,
};
use crate::domain::external_agent::telemetry::{ExternalAgentTool, ExternalAgentTurn};

const SECRET: &str = "sk-ant-api03-TOOLSECRETTOOLSECRETTOOLSECRET";

fn tool_use(id: &str, name: &str, input: serde_json::Value) -> ExternalAgentEvent {
    ExternalAgentEvent::AssistantBlock {
        message_id: "m1".into(),
        block: AssistantContent::ToolUse {
            id: id.into(),
            name: name.into(),
            input,
        },
    }
}

fn tool_result(id: &str, content: &str, is_error: bool, denied: bool) -> ExternalAgentEvent {
    ExternalAgentEvent::ToolResult(ToolResultEvent {
        tool_use_id: Some(id.into()),
        content: serde_json::json!(content),
        is_error,
        permission_denied: denied,
    })
}

fn tools(rig: &Rig) -> Vec<ExternalAgentTool> {
    rig.records
        .all()
        .into_iter()
        .filter_map(|record| match record {
            SessionRecord::ToolFinished(tool) => Some(*tool),
            _ => None,
        })
        .collect()
}

fn turns(rig: &Rig) -> Vec<ExternalAgentTurn> {
    rig.records
        .all()
        .into_iter()
        .filter_map(|record| match record {
            SessionRecord::TurnReported(turn) => Some(*turn),
            _ => None,
        })
        .collect()
}

fn costed(total_usd: f64, input: u64, output: u64) -> ExternalAgentEvent {
    ExternalAgentEvent::Result(ResultEvent {
        is_error: Some(false),
        terminal_reason: Some("completed".into()),
        result_text: Some("done".into()),
        total_cost_usd: Some(total_usd),
        usage: TokenCounts {
            input,
            output,
            cache_read: 7,
            cache_write: 3,
        },
        num_turns: Some(2),
        duration_ms: Some(1200),
        duration_api_ms: Some(900),
        ..ResultEvent::default()
    })
}

#[tokio::test(start_paused = true)]
async fn a_tool_call_is_recorded_once_answered_with_its_sizes_and_a_redacted_summary() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    let input = serde_json::json!({"command": format!("curl -H 'x-api-key: {SECRET}' {SECRET}")});
    rig.feed(tool_use("toolu_1", "Bash", input.clone())).await;
    assert!(tools(&rig).is_empty(), "a call is recorded once answered");
    tokio::time::advance(Duration::from_millis(250)).await;
    rig.feed(tool_result("toolu_1", "hello", false, false))
        .await;
    let [tool] = tools(&rig).try_into().expect("one call");
    assert_eq!(
        (
            tool.member_turn,
            tool.tool.as_str(),
            tool.tool_use_id.as_str()
        ),
        (Some(1), "Bash", "toolu_1")
    );
    assert_eq!((tool.duration_ms, tool.outcome.as_str()), (250, "ok"));
    assert_eq!(tool.argument_bytes, input.to_string().len());
    assert_eq!(tool.result_bytes, 5);
    let summary = tool
        .summary
        .expect("a Bash call is summarised by its command");
    assert!(summary.starts_with("curl"), "{summary}");
    assert!(!summary.contains("TOOLSECRET"), "{summary}");
}

#[tokio::test]
async fn a_denied_an_erring_and_an_unanswered_call_each_say_so() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.feed(tool_use(
        "t1",
        "Write",
        serde_json::json!({"file_path": "/w/a.rs", "content": SECRET}),
    ))
    .await;
    rig.feed(tool_use(
        "t2",
        "Bash",
        serde_json::json!({"command": "false"}),
    ))
    .await;
    rig.feed(tool_use(
        "t3",
        "WebFetch",
        serde_json::json!({"url": "https://example.com"}),
    ))
    .await;
    rig.feed(tool_result("t2", "exit 1", true, false)).await;
    rig.feed(tool_result("t1", "denied by rule", true, true))
        .await;
    rig.feed(completed("done")).await;
    let outcomes: Vec<(String, String, Option<String>)> = tools(&rig)
        .into_iter()
        .map(|tool| {
            (
                tool.tool_use_id,
                tool.outcome,
                tool.summary.map(|s| s.to_string()),
            )
        })
        .collect();
    assert_eq!(
        outcomes,
        [
            ("t2".into(), "error".into(), Some("false".into())),
            ("t1".into(), "denied".into(), Some("/w/a.rs".into())),
            ("t3".into(), "unanswered".into(), None),
        ],
        "a file tool is summarised by its path only; another tool not at all"
    );
}

#[tokio::test]
async fn each_turn_is_charged_the_growth_of_the_cumulative_cost() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.feed(ExternalAgentEvent::Init(InitEvent {
        session_id: Some("sess-1".into()),
        model: Some("claude-haiku-4-5".into()),
        cli_version: Some("2.1.280".into()),
        ..InitEvent::default()
    }))
    .await;
    rig.feed(costed(0.01, 100, 20)).await;
    rig.session.prompt("two", None).await.unwrap();
    rig.feed(costed(0.03, 50, 5)).await;
    let turns = turns(&rig);
    let charges: Vec<(u64, u64, u64)> = turns
        .iter()
        .map(|t| {
            (
                t.member_turn,
                t.list_price_cost_micro_usd,
                t.list_price_total_micro_usd,
            )
        })
        .collect();
    assert_eq!(charges, [(1, 10_000, 10_000), (2, 20_000, 30_000)]);
    let first = &turns[0];
    assert_eq!(first.turn_end, "completed");
    assert_eq!(first.reason_kind, None);
    assert_eq!(first.claude_session_id.as_deref(), Some("sess-1"));
    assert_eq!(first.model.as_deref(), Some("claude-haiku-4-5"));
    assert_eq!(
        (
            first.is_error,
            first.num_turns,
            first.duration_ms,
            first.duration_api_ms
        ),
        (Some(false), Some(2), Some(1200), Some(900))
    );
    assert_eq!(
        (
            first.input_tokens,
            first.output_tokens,
            first.cache_read_tokens,
            first.cache_write_tokens
        ),
        (100, 20, 7, 3)
    );
    assert_eq!(first.task_id, None, "no board task is held before #2291");
    let initialized = rig
        .records
        .kinds()
        .iter()
        .filter(|kind| **kind == "initialized")
        .count();
    assert_eq!(initialized, 1);
}

#[tokio::test]
async fn a_spent_budget_and_an_api_error_are_classified() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.feed(result(true, "budget_exhausted", None)).await;
    rig.session.prompt("two", None).await.unwrap();
    rig.feed(ExternalAgentEvent::Result(ResultEvent {
        is_error: Some(true),
        api_error_status: Some(401),
        ..ResultEvent::default()
    }))
    .await;
    rig.session.prompt("three", None).await.unwrap();
    rig.feed(result(true, "aborted_streaming", None)).await;
    let ends: Vec<(String, Option<String>)> = turns(&rig)
        .into_iter()
        .map(|t| (t.turn_end, t.reason_kind))
        .collect();
    assert_eq!(
        ends,
        [
            ("budget_exceeded".into(), Some("budget_exhausted".into())),
            ("failed".into(), Some("api_401".into())),
            ("aborted".into(), Some("aborted_streaming".into())),
        ]
    );
}

#[tokio::test]
async fn unknown_events_and_skipped_lines_are_counted_and_rate_limited() {
    let rig = started().await;
    for _ in 0..5 {
        rig.feed(ExternalAgentEvent::Unknown {
            kind: "system/brand_new\u{1b}".into(),
        })
        .await;
    }
    rig.feed(skipped(9)).await;
    let diagnostics: Vec<(String, String, u64, Option<usize>)> = rig
        .records
        .all()
        .into_iter()
        .filter_map(|record| match record {
            SessionRecord::StreamDiagnostic(d) => Some((d.kind, d.name, d.count, d.bytes)),
            _ => None,
        })
        .collect();
    let unknown = |count| {
        (
            "unknown_event".to_string(),
            "system?brand_new?".to_string(),
            count,
            None,
        )
    };
    assert_eq!(
        diagnostics,
        [
            unknown(1),
            unknown(2),
            unknown(4),
            ("skipped_line".into(), "over_cap".into(), 1, Some(9)),
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn the_end_of_the_process_records_its_exit_and_wall_time_and_the_cut_turn() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    tokio::time::advance(Duration::from_millis(3000)).await;
    rig.wire.end_output();
    rig.step().await;
    let [turn] = turns(&rig).try_into().expect("the cut turn is reported");
    assert_eq!((turn.member_turn, turn.turn_end.as_str()), (1, "exited"));
    let ended = rig.records.all().into_iter().last();
    assert_eq!(
        ended,
        Some(SessionRecord::Ended {
            clean: true,
            exit_code: Some(0),
            signal: None,
            wall_ms: Some(3000),
        })
    );
}

/// The records after the last `tool_called`: what ending the member
/// recorded of the call left open.
fn after_the_call(rig: &Rig) -> Vec<SessionRecord> {
    let all = rig.records.all();
    let at = all
        .iter()
        .rposition(|record| matches!(record, SessionRecord::ToolCalled { .. }))
        .expect("a call was made");
    all[at + 1..].to_vec()
}

fn ended_once(records: &[SessionRecord]) -> SessionRecord {
    let ended: Vec<&SessionRecord> = records
        .iter()
        .filter(|record| matches!(record, SessionRecord::Ended { .. }))
        .collect();
    let [ended] = ended.as_slice() else {
        panic!("one end is recorded: {records:?}")
    };
    (*ended).clone()
}

/// #2304 review H1: `close` is how the runner ends a member, so the turn
/// it cuts, the call it leaves open and the process's end are recorded.
#[tokio::test(start_paused = true)]
async fn closing_mid_turn_records_the_open_call_the_cut_turn_and_the_end() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.feed(tool_use(
        "t1",
        "Bash",
        serde_json::json!({"command": "sleep 9"}),
    ))
    .await;
    tokio::time::advance(Duration::from_millis(400)).await;
    rig.session.close().await.unwrap();
    let after = after_the_call(&rig);
    let kinds: Vec<&str> = after.iter().map(SessionRecord::kind).collect();
    assert_eq!(
        kinds,
        ["tool_finished", "turn_reported", "closed", "ended"],
        "{after:?}"
    );
    let [tool] = tools(&rig).try_into().expect("one call");
    assert_eq!(
        (
            tool.tool_use_id.as_str(),
            tool.outcome.as_str(),
            tool.duration_ms
        ),
        ("t1", "unanswered", 400)
    );
    let [turn] = turns(&rig).try_into().expect("the cut turn");
    assert_eq!((turn.member_turn, turn.turn_end.as_str()), (1, "closed"));
    assert_eq!(
        ended_once(&after),
        SessionRecord::Ended {
            clean: true,
            exit_code: Some(0),
            signal: None,
            wall_ms: Some(400),
        }
    );
    assert!(rig.wire.dropped(), "the process is let go once it ended");
}

/// Closing an idle member records its end too.
#[tokio::test]
async fn closing_an_idle_member_records_the_end() {
    let rig = started().await;
    rig.session.close().await.unwrap();
    assert_eq!(rig.records.kinds(), ["started", "closed", "ended"]);
    assert!(turns(&rig).is_empty(), "no turn ran");
}

/// #2304 review H1: an abandoned member (its interrupt could not be
/// written) records the turn it cut, the call left open and its end.
#[tokio::test(start_paused = true)]
async fn abandoning_mid_turn_records_the_open_call_the_cut_turn_and_the_end() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.feed(tool_use(
        "t1",
        "Bash",
        serde_json::json!({"command": "sleep 9"}),
    ))
    .await;
    tokio::time::advance(Duration::from_millis(700)).await;
    rig.wire
        .refuse_interrupts
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let outcome = rig.session.abort().await.unwrap();
    assert!(outcome.member_ended, "an unwritable interrupt ends it");
    let after = after_the_call(&rig);
    let kinds: Vec<&str> = after.iter().map(SessionRecord::kind).collect();
    assert_eq!(
        kinds,
        [
            "tool_finished",
            "turn_reported",
            "abandoned",
            "ended",
            "aborted"
        ],
        "{after:?}"
    );
    let [tool] = tools(&rig).try_into().expect("one call");
    assert_eq!(
        (tool.outcome.as_str(), tool.duration_ms),
        ("unanswered", 700)
    );
    let [turn] = turns(&rig).try_into().expect("the cut turn");
    assert_eq!((turn.member_turn, turn.turn_end.as_str()), (1, "abandoned"));
    assert_eq!(
        ended_once(&after),
        SessionRecord::Ended {
            clean: true,
            exit_code: Some(0),
            signal: None,
            wall_ms: Some(700),
        }
    );
}

/// #2304 review L3: a failed result naming no `terminal_reason` takes its
/// reason kind from its `errors[]`, bounded.
#[tokio::test]
async fn a_failure_without_a_terminal_reason_is_named_by_its_errors() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.feed(ExternalAgentEvent::Result(ResultEvent {
        is_error: Some(true),
        errors: vec!["API Error: 529 Overloaded".into()],
        ..ResultEvent::default()
    }))
    .await;
    let [turn] = turns(&rig).try_into().expect("one turn");
    assert_eq!(
        (turn.turn_end.as_str(), turn.reason_kind.as_deref()),
        ("failed", Some("api_error_529"))
    );
}

/// #2304 review L4: a cumulative cost that went down is carried on the
/// turn it was reported in.
#[tokio::test]
async fn a_cumulative_cost_that_dropped_is_on_the_turn_record() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.feed(costed(0.03, 1, 1)).await;
    rig.session.prompt("two", None).await.unwrap();
    rig.feed(costed(0.0, 1, 1)).await;
    let drops: Vec<_> = turns(&rig).into_iter().map(|t| t.cost_drop).collect();
    assert_eq!(
        drops,
        [
            None,
            Some(crate::domain::external_agent::usage::CostDrop {
                previous_micro_usd: 30_000,
                reported_micro_usd: 0,
            }),
        ]
    );
}
