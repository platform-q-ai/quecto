//! The event log's records of a claude-code member (#2304): what each
//! stream event feeds, how a tool call is summarised, and that each record
//! round-trips through the log.

use super::*;
use crate::domain::audit::AuditEvent;
use crate::domain::external_agent::stream::{
    BackgroundTask, InitEvent, InterruptReceipt, RateLimitInfo, RateLimitStatus, ResultEvent,
    SkippedLine, SkippedLineReason, TaskNotification, TaskStarted, ToolResultEvent,
};

/// Every event type of the vocabulary and what the log does with it: also
/// fed through the session (`session_telemetry_tests`), which must record
/// what this says.
pub(crate) fn every_event_type() -> Vec<(ExternalAgentEvent, StreamTelemetry)> {
    use StreamTelemetry::{Ignored, Recorded};
    let block = |block| ExternalAgentEvent::AssistantBlock {
        message_id: "m".into(),
        block,
    };
    vec![
        (
            ExternalAgentEvent::Init(InitEvent::default()),
            Recorded("external_agent_lifecycle"),
        ),
        (
            ExternalAgentEvent::ThinkingTokens {
                estimated_tokens: None,
            },
            Ignored,
        ),
        (
            block(AssistantContent::Text("secret words".into())),
            Ignored,
        ),
        (
            block(AssistantContent::Thinking {
                text: Some("thoughts".into()),
            }),
            Ignored,
        ),
        (
            block(AssistantContent::ToolUse {
                id: "t".into(),
                name: "Bash".into(),
                input: serde_json::json!({}),
            }),
            Recorded("external_agent_tool"),
        ),
        (
            ExternalAgentEvent::AssistantError {
                message_id: None,
                kind: "x".into(),
            },
            Recorded("external_agent_turn"),
        ),
        (
            ExternalAgentEvent::ToolResult(ToolResultEvent {
                tool_use_id: None,
                content: serde_json::Value::Null,
                is_error: false,
                permission_denied: false,
            }),
            Recorded("external_agent_tool"),
        ),
        (ExternalAgentEvent::UserText { text: "hi".into() }, Ignored),
        (
            ExternalAgentEvent::TaskStarted(TaskStarted {
                task_id: "k".into(),
                tool_use_id: None,
                description: None,
                is_backgrounded: false,
            }),
            Ignored,
        ),
        (
            ExternalAgentEvent::TaskNotification(TaskNotification {
                task_id: "k".into(),
                tool_use_id: None,
                status: None,
                summary: None,
            }),
            Ignored,
        ),
        (
            ExternalAgentEvent::BackgroundTasksChanged {
                tasks: vec![BackgroundTask {
                    task_id: "k".into(),
                    description: None,
                }],
            },
            Ignored,
        ),
        (
            ExternalAgentEvent::RateLimit(RateLimitInfo {
                status: RateLimitStatus::Allowed,
                limit_type: None,
                utilization: None,
                resets_at: None,
                windows: Vec::new(),
                using_overage: None,
            }),
            Ignored,
        ),
        (
            ExternalAgentEvent::Result(ResultEvent::default()),
            Recorded("external_agent_turn"),
        ),
        (
            ExternalAgentEvent::InterruptAnswered(InterruptReceipt::default()),
            Ignored,
        ),
        (
            ExternalAgentEvent::Unknown {
                kind: "brand_new".into(),
            },
            Recorded("external_agent_stream_diagnostic"),
        ),
        (
            ExternalAgentEvent::LineSkipped(SkippedLine {
                reason: SkippedLineReason::NotUtf8,
                bytes: 1,
            }),
            Recorded("external_agent_stream_diagnostic"),
        ),
    ]
}

/// The mapping itself is exhaustive (a new type does not compile
/// unplaced); this pins each placement.
#[test]
fn every_stream_event_type_maps_to_a_telemetry_outcome() {
    for (event, expected) in every_event_type() {
        assert_eq!(stream_telemetry(&event), expected, "{event:?}");
    }
}

#[test]
fn a_bash_summary_is_its_command_redacted_and_bounded() {
    let command = format!(
        "export ANTHROPIC_API_KEY=sk-ant-api03-ABCDEFGHIJKLMNOPQRSTUV; curl https://user:hunter2@proxy.example:3128 {}",
        "x".repeat(400)
    );
    let summary = tool_summary("Bash", &serde_json::json!({ "command": command }))
        .expect("a Bash call has a summary");
    assert!(summary.len() <= TOOL_SUMMARY_BYTES);
    assert!(summary.starts_with("export"), "{summary}");
    for secret in ["ABCDEFGHIJKLMNOPQRSTUV", "hunter2"] {
        assert!(!summary.contains(secret), "{summary}");
    }
}

#[test]
fn only_allowlisted_tools_and_fields_are_summarised() {
    let input = serde_json::json!({
        "file_path": "/w/a.rs",
        "content": "the file's words",
        "task_id": 42,
        "url": "https://example.com",
    });
    assert_eq!(tool_summary("Edit", &input).as_deref(), Some("/w/a.rs"));
    assert_eq!(
        tool_summary("mcp__quecto__claim", &input).as_deref(),
        Some("42")
    );
    assert_eq!(tool_summary("WebFetch", &input), None);
    assert_eq!(
        board_task_id("mcp__quecto__claim", &input).as_deref(),
        Some("42")
    );
    assert_eq!(
        board_task_id(
            "mcp__board__board_claim",
            &serde_json::json!({"task_id": "T 1"})
        )
        .as_deref(),
        Some("T?1")
    );
    assert_eq!(
        board_task_id("Bash", &input),
        None,
        "only a board tool names a task"
    );
    assert_eq!(
        tool_summary("Bash", &serde_json::json!({"cmd": "ls"})),
        None
    );
    // A multi-byte character at the cut is never split.
    let long = serde_json::json!({ "command": "é".repeat(300) });
    let summary = tool_summary("Bash", &long).unwrap();
    assert_eq!(summary.len(), TOOL_SUMMARY_BYTES);
}

#[test]
fn a_name_from_the_stream_is_bounded_and_allowlisted() {
    assert_eq!(recorded_name("mcp__quecto__claim"), "mcp__quecto__claim");
    assert_eq!(recorded_name("a b/\u{1b}"), "a?b??");
    assert_eq!(recorded_name(&"x".repeat(200)).len(), RECORDED_NAME_BYTES);
}

#[test]
fn every_external_agent_event_round_trips_through_the_log() {
    let events = [
        AuditEvent::ExternalAgentTurn {
            member_ref: "C3".into(),
            record: Box::new(ExternalAgentTurn {
                member_turn: 2,
                turn_end: "budget_exceeded".into(),
                reason_kind: Some("budget_exhausted".into()),
                claude_session_id: Some("s".into()),
                model: Some("haiku".into()),
                is_error: Some(true),
                num_turns: Some(3),
                duration_ms: Some(10),
                duration_api_ms: Some(8),
                input_tokens: 1,
                output_tokens: 2,
                cache_read_tokens: 3,
                cache_write_tokens: 4,
                list_price_cost_micro_usd: 5,
                list_price_total_micro_usd: 6,
                task_id: Some("T-9".into()),
                cost_drop: Some(crate::domain::external_agent::usage::CostDrop {
                    previous_micro_usd: 9,
                    reported_micro_usd: 0,
                }),
            }),
        },
        AuditEvent::ExternalAgentTool {
            member_ref: "C3".into(),
            record: Box::new(ExternalAgentTool {
                member_turn: Some(2),
                tool: "Bash".into(),
                tool_use_id: "t1".into(),
                duration_ms: 4,
                outcome: "denied".into(),
                rule_id: Some("rm-rf".into()),
                argument_bytes: 10,
                result_bytes: 20,
                summary: Some(Redacted::from("ls")),
                task_id: None,
            }),
        },
        AuditEvent::ExternalAgentLifecycle {
            member_ref: "C3".into(),
            record: ExternalAgentLifecycle::Ended {
                clean: false,
                exit_code: None,
                signal: Some(9),
                wall_ms: Some(100),
            },
        },
        AuditEvent::ExternalAgentLifecycle {
            member_ref: "C3".into(),
            record: ExternalAgentLifecycle::LogIncomplete {
                dropped: 2,
                failed: 1,
            },
        },
        AuditEvent::ExternalAgentStreamDiagnostic {
            member_ref: "C3".into(),
            record: ExternalAgentStreamDiagnostic {
                kind: "skipped_line".into(),
                name: "over_cap".into(),
                bytes: Some(9),
                count: 4,
                member_turn: Some(1),
            },
        },
    ];
    for event in events {
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["member_ref"], "C3", "flat: {json}");
        assert!(json.get("turn").is_none(), "the envelope's own: {json}");
        let back: AuditEvent = serde_json::from_value(json).unwrap();
        assert_eq!(back, event);
    }
}

#[test]
fn a_board_call_is_summarised_by_its_ids_and_recipient_never_its_token() {
    let send = serde_json::json!({"to": "coordinator", "text": "the message's words"});
    assert_eq!(
        tool_summary("mcp__board__board_send", &send).as_deref(),
        Some("coordinator")
    );
    let ack = serde_json::json!({"id": 1});
    assert_eq!(
        tool_summary("mcp__quecto__board_ack", &ack).as_deref(),
        Some("1")
    );
    let submit = serde_json::json!({"task_id": "T1", "token": "06f61158", "evidence": "words"});
    let summary = tool_summary("mcp__board__board_submit", &submit).unwrap();
    assert_eq!(summary.as_str(), "T1");
    assert!(!summary.contains("06f61158"), "{summary}");
}

#[test]
fn an_error_s_reason_kind_is_its_leading_words_up_to_a_status_bounded_and_allowlisted() {
    let kind = |errors: &[&str]| {
        error_reason_kind(&errors.iter().map(|e| e.to_string()).collect::<Vec<_>>())
    };
    assert_eq!(
        kind(&["API Error: 529 Overloaded"]).as_deref(),
        Some("api_error_529")
    );
    assert_eq!(
        kind(&["Not logged in · Please run /login", "second"]).as_deref(),
        Some("not_logged_in")
    );
    assert_eq!(
        kind(&["Reached maximum budget ($0.5)"]).as_deref(),
        Some("reached_maximum_budget")
    );
    assert_eq!(
        kind(&["\u{1b}[31mfailed sk-ant-api03-SECRETSECRETSECRET"]).as_deref(),
        Some("31mfailed_redacted"),
        "redacted first; only ASCII letters, digits and `_` survive"
    );
    let long = kind(&["abcdefghijklmnopqrst abcdefghijklmnopqrst x"]).unwrap();
    assert_eq!(long, "abcdefghijklmnopqrst_abcdefghijk");
    assert_eq!(long.len(), ERROR_REASON_BYTES);
    assert_eq!(
        kind(&[&"Q".repeat(40)]),
        None,
        "a word that long is an id or a key, not a word"
    );
    assert_eq!(kind(&[]), None);
    assert_eq!(kind(&[" ·· !"]), None, "no word, no reason");
}
