//! The session's measurements stay bounded (#2304): calls awaiting their
//! result, and the diagnostic kinds counted.

use super::*;
use crate::domain::external_agent::stream::ToolResultEvent;

fn call(id: &str) -> ExternalAgentEvent {
    ExternalAgentEvent::AssistantBlock {
        message_id: "m".into(),
        block: AssistantContent::ToolUse {
            id: id.into(),
            name: "Bash".into(),
            input: serde_json::json!({"command": "ls"}),
        },
    }
}

#[test]
fn past_its_capacity_the_oldest_open_call_is_recorded_unanswered() {
    let mut telemetry = SessionTelemetry::default();
    let mut records = Vec::new();
    for n in 0..=PENDING_TOOL_CAPACITY {
        telemetry.observe(
            &call(&format!("t{n}")),
            Some(1),
            AgentClockInstant(n as u64),
            &mut records,
        );
    }
    let [SessionRecord::ToolFinished(oldest)] = records.as_slice() else {
        panic!("one call recorded: {records:?}")
    };
    assert_eq!(
        (oldest.tool_use_id.as_str(), oldest.outcome.as_str()),
        ("t0", "unanswered")
    );
    assert_eq!(telemetry.pending.len(), PENDING_TOOL_CAPACITY);
}

#[test]
fn a_result_naming_no_call_answers_the_oldest_and_one_naming_none_open_is_dropped() {
    let mut telemetry = SessionTelemetry::default();
    let mut records = Vec::new();
    let now = AgentClockInstant(10);
    let result = |id: Option<&str>| {
        ExternalAgentEvent::ToolResult(ToolResultEvent {
            tool_use_id: id.map(str::to_string),
            content: serde_json::json!([{"type": "text", "text": "abc"}]),
            is_error: false,
            permission_denied: false,
        })
    };
    telemetry.observe(&result(Some("nobody")), None, now, &mut records);
    assert!(records.is_empty());
    telemetry.observe(&call("t1"), Some(1), AgentClockInstant(4), &mut records);
    telemetry.observe(&call("t2"), Some(1), AgentClockInstant(4), &mut records);
    telemetry.observe(&result(None), Some(1), now, &mut records);
    let [SessionRecord::ToolFinished(first)] = records.as_slice() else {
        panic!("one call recorded: {records:?}")
    };
    assert_eq!(
        (
            first.tool_use_id.as_str(),
            first.duration_ms,
            first.result_bytes
        ),
        ("t1", 6, 3)
    );
}

#[test]
fn past_its_capacity_a_new_diagnostic_kind_is_not_counted() {
    let mut telemetry = SessionTelemetry::default();
    let mut records = Vec::new();
    for n in 0..=DIAGNOSTIC_KIND_CAPACITY {
        let unknown = ExternalAgentEvent::Unknown {
            kind: format!("kind{n}"),
        };
        telemetry.observe(&unknown, None, AgentClockInstant(0), &mut records);
    }
    assert_eq!(records.len(), DIAGNOSTIC_KIND_CAPACITY);
    assert_eq!(telemetry.diagnostics.len(), DIAGNOSTIC_KIND_CAPACITY);
}
