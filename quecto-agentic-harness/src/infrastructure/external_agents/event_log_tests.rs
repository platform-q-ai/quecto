//! Every session record is placed in the event log or left to `tracing`
//! (#2304), and every one logged names its member.

use super::*;
use crate::application::external_agent::dto::PromptAccepted;
use crate::domain::external_agent::telemetry::{
    ExternalAgentStreamDiagnostic, ExternalAgentTool, ExternalAgentTurn,
};

fn member() -> MemberIdentity {
    MemberIdentity {
        member_ref: "C3".into(),
        credential_mode: "api_key",
    }
}

fn turn(turn: u64) -> ExternalAgentTurn {
    ExternalAgentTurn {
        member_turn: turn,
        turn_end: "completed".into(),
        reason_kind: None,
        claude_session_id: None,
        model: None,
        is_error: Some(false),
        num_turns: Some(1),
        duration_ms: None,
        duration_api_ms: None,
        input_tokens: 0,
        output_tokens: 0,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        list_price_cost_micro_usd: 0,
        list_price_total_micro_usd: 0,
        task_id: None,
        cost_drop: None,
    }
}

fn tool() -> ExternalAgentTool {
    ExternalAgentTool {
        member_turn: Some(2),
        tool: "Bash".into(),
        tool_use_id: "t1".into(),
        duration_ms: 5,
        outcome: "ok".into(),
        rule_id: None,
        argument_bytes: 1,
        result_bytes: 1,
        task_id: None,
    }
}

/// Each record, and the event (by its `event` tag) it is logged as.
fn table() -> Vec<(SessionRecord, Option<&'static str>)> {
    let lifecycle = Some("external_agent_lifecycle");
    vec![
        (SessionRecord::Started, lifecycle),
        (SessionRecord::StartRefused { kind: "not_found" }, lifecycle),
        (
            SessionRecord::PromptAccepted {
                accepted: PromptAccepted::Started { turn: 1 },
                bytes: 3,
            },
            None,
        ),
        (
            SessionRecord::PromptRefused {
                refusal: "busy",
                bytes: 3,
            },
            lifecycle,
        ),
        (
            SessionRecord::ToolCalled {
                turn: Some(1),
                tool: "Bash".into(),
            },
            None,
        ),
        (
            SessionRecord::LineSkipped {
                turn: Some(1),
                bytes: 3,
            },
            None,
        ),
        (
            SessionRecord::TurnEnded {
                turn: 1,
                outcome: "completed",
                duration_ms: None,
                cost_micro_usd: 0,
            },
            None,
        ),
        (SessionRecord::TurnContinued { turn: 1, owed: 1 }, None),
        (
            SessionRecord::ResultWithoutIds {
                turn: 1,
                ended: true,
            },
            None,
        ),
        (
            SessionRecord::Interrupted {
                turn: 1,
                cause: "abort",
            },
            lifecycle,
        ),
        (
            SessionRecord::Abandoned {
                turn: 1,
                dropped_follow_ups: 0,
            },
            lifecycle,
        ),
        (SessionRecord::FollowUpStarted { turn: 2, bytes: 3 }, None),
        (
            SessionRecord::FollowUpFailed {
                turn: 2,
                bytes: 3,
                refusal: "input",
            },
            lifecycle,
        ),
        (
            SessionRecord::Aborted {
                turn: None,
                dropped_follow_ups: 1,
            },
            lifecycle,
        ),
        (
            SessionRecord::Closed {
                turn: Some(2),
                dropped_follow_ups: 0,
            },
            lifecycle,
        ),
        (
            SessionRecord::Ended {
                clean: true,
                exit_code: Some(0),
                signal: None,
                wall_ms: Some(9),
            },
            lifecycle,
        ),
        (
            SessionRecord::TurnReported(Box::new(turn(1))),
            Some("external_agent_turn"),
        ),
        (
            SessionRecord::ToolFinished(Box::new(tool())),
            Some("external_agent_tool"),
        ),
        (
            SessionRecord::Initialized {
                cli_version: Some("2.1.280".into()),
                claude_session_id: None,
                model: None,
            },
            lifecycle,
        ),
        (
            SessionRecord::StreamDiagnostic(ExternalAgentStreamDiagnostic {
                kind: "unknown_event".into(),
                name: "x".into(),
                bytes: None,
                count: 1,
                member_turn: None,
            }),
            Some("external_agent_stream_diagnostic"),
        ),
    ]
}

#[test]
fn every_session_record_is_logged_or_left_to_tracing() {
    let table = table();
    let mut kinds: Vec<&str> = table.iter().map(|(record, _)| record.kind()).collect();
    kinds.sort_unstable();
    kinds.dedup();
    assert_eq!(kinds.len(), table.len(), "one row per record kind");
    for (record, expected) in table {
        let logged = audit_event(&record, &member()).map(|(_, event)| {
            let json = serde_json::to_value(&event).unwrap();
            assert_eq!(json["member_ref"], "C3", "{json}");
            json["event"].as_str().unwrap().to_string()
        });
        assert_eq!(logged.as_deref(), expected, "{}", record.kind());
    }
}

#[test]
fn a_record_is_filed_under_its_turn_and_names_the_credential_mode_only() {
    let (filed, event) = audit_event(&SessionRecord::TurnReported(Box::new(turn(7))), &member())
        .expect("a turn is logged");
    assert_eq!(filed, 7);
    let huge = turn(u64::MAX);
    let (filed, _) = audit_event(&SessionRecord::TurnReported(Box::new(huge)), &member()).unwrap();
    assert_eq!(filed, u32::MAX, "saturated");
    assert!(matches!(event, AuditEvent::ExternalAgentTurn { .. }));
    let (_, started) = audit_event(&SessionRecord::Started, &member()).unwrap();
    let json = serde_json::to_value(&started).unwrap();
    assert_eq!(json["kind"], "started");
    assert_eq!(json["credential_mode"], "api_key");
}

/// #2304 review round 2: a refused prompt and a follow-up that could not
/// be written are logged by their refusal's kind alone: never the text,
/// nor its size.
#[test]
fn a_refusal_is_logged_by_its_kind_alone() {
    for (record, expected) in [
        (
            SessionRecord::PromptRefused {
                refusal: "busy",
                bytes: 3,
            },
            serde_json::json!({
                "event": "external_agent_lifecycle",
                "kind": "prompt_refused",
                "member_ref": "C3",
                "refusal": "busy",
            }),
        ),
        (
            SessionRecord::FollowUpFailed {
                turn: 2,
                bytes: 3,
                refusal: "input",
            },
            serde_json::json!({
                "event": "external_agent_lifecycle",
                "kind": "follow_up_failed",
                "member_ref": "C3",
                "member_turn": 2,
                "refusal": "input",
            }),
        ),
    ] {
        let (_, event) = audit_event(&record, &member()).expect("a refusal is logged");
        assert_eq!(serde_json::to_value(&event).unwrap(), expected);
    }
}
