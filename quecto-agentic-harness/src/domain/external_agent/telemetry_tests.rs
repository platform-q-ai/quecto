//! The event log's records of a claude-code member (#2304): what each
//! stream event feeds, what a tool call keeps, and that each record
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

/// Only a board tool's call names a task, and only its `task_id` is kept
/// of its input.
#[test]
fn only_a_board_call_names_a_task() {
    let input = serde_json::json!({
        "file_path": "/w/a.rs",
        "task_id": 42,
    });
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

/// #2304 swarm review: text from the stream is bounded to the allowlist
/// and never kept when it looks like a secret, however the allowlist
/// rewrites it.
#[test]
fn stream_text_is_kept_only_when_allowlisted_and_not_secret_shaped() {
    assert_eq!(
        recorded_text("claude-haiku-4-5").as_deref(),
        Some("claude-haiku-4-5")
    );
    assert_eq!(recorded_text("a b").as_deref(), Some("a?b"));
    for secret in [
        "sk-ant-api03-SECRETSECRETSECRET",
        "token=hunter2hunter2",
        "ghp_0123456789abcdef0123456789abcdef0123",
        // The allowlist would turn the space into `?`: the label is
        // judged as it came.
        "x token= hunter2hunter2",
    ] {
        assert_eq!(recorded_text(secret), None, "{secret}");
    }
    // A key past the recorded bytes is cut off, never judged whole.
    let long = format!("{}sk-ant-api03-SECRETSECRETSECRET", "x".repeat(60));
    let kept = recorded_text(&long);
    assert!(
        kept.as_deref().is_none_or(|kept| !kept.contains("SECRET")),
        "{kept:?}"
    );
}

/// #2304 swarm review: a fingerprint is a fixed-size digest, never the
/// text.
#[test]
fn a_fingerprint_is_a_fixed_size_digest_of_the_text() {
    let secret = "sk-ant-api03-SECRETSECRETSECRET";
    let print = fingerprint(secret);
    assert_eq!(print.len(), "sha256:".len() + 16, "{print}");
    assert!(print.starts_with("sha256:"), "{print}");
    assert!(
        print["sha256:".len()..]
            .chars()
            .all(|c| matches!(c, '0'..='9' | 'a'..='f')),
        "{print}"
    );
    assert!(!print.contains("SECRET"), "{print}");
    assert_eq!(fingerprint(secret), print, "the same text, the same print");
    assert_ne!(fingerprint("system/other"), print);
    assert!(is_recorded_name(&print), "a print is a recorded name");
}

/// #2304 swarm review: a board tool's task id is kept only when it is a
/// number or allowlisted text that does not look like a secret.
#[test]
fn a_board_task_id_that_looks_like_a_secret_is_not_kept() {
    let claim = |task_id: serde_json::Value| {
        board_task_id(
            "mcp__quecto__claim",
            &serde_json::json!({ "task_id": task_id }),
        )
    };
    assert_eq!(claim(serde_json::json!(7)).as_deref(), Some("7"));
    assert_eq!(claim(serde_json::json!("T1")).as_deref(), Some("T1"));
    assert_eq!(
        claim(serde_json::json!("sk-ant-api03-SECRETSECRETSECRET")),
        None
    );
}
