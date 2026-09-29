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

/// #2304 review L6: a new process starts its measurements afresh: the
/// last one's open calls, session id and model are not carried over.
#[test]
fn a_new_process_forgets_the_last_one_s_open_calls_session_and_model() {
    let mut telemetry = SessionTelemetry::default();
    let mut records = Vec::new();
    telemetry.process_started(AgentClockInstant(0));
    let init = ExternalAgentEvent::Init(crate::domain::external_agent::stream::InitEvent {
        session_id: Some("s-1".into()),
        model: Some("haiku".into()),
        ..Default::default()
    });
    telemetry.observe(&init, None, AgentClockInstant(1), &mut records);
    telemetry.observe(&call("t1"), Some(1), AgentClockInstant(2), &mut records);
    telemetry.process_started(AgentClockInstant(10));
    records.clear();
    telemetry.turn_ended(2, TurnCut::Exited, AgentClockInstant(11), &mut records);
    let [SessionRecord::TurnReported(turn)] = records.as_slice() else {
        panic!("only the turn is recorded, no call of the last process: {records:?}")
    };
    assert_eq!(
        (turn.claude_session_id.as_deref(), turn.model.as_deref()),
        (None, None)
    );
    assert_eq!(telemetry.wall_ms(AgentClockInstant(15)), Some(5));
}

/// A record's family, as the event log files it.
fn family(record: &SessionRecord) -> Option<(&'static str, String)> {
    match record {
        SessionRecord::TurnReported(turn) => {
            Some(("external_agent_turn", format!("{:?}", turn.reason_kind)))
        }
        SessionRecord::ToolFinished(tool) => Some((
            "external_agent_tool",
            format!("{} {}", tool.tool_use_id, tool.outcome),
        )),
        SessionRecord::StreamDiagnostic(diagnostic) => {
            Some(("external_agent_stream_diagnostic", diagnostic.name.clone()))
        }
        SessionRecord::Initialized { .. } => Some(("external_agent_lifecycle", String::new())),
        _ => None,
    }
}

/// The families of what a running turn records from `event` (none: the
/// baseline) through its end: a call `t0` is open before it, and a failed
/// result naming `u1`, whose errors name `closing`, ends it.
fn families_through_the_turn(
    event: Option<&ExternalAgentEvent>,
) -> std::collections::BTreeSet<(&'static str, String)> {
    use crate::application::external_agent::dto::{SessionPhase, UserTurnId};
    use crate::application::external_agent::session_core::{Admission, SessionCore};
    let mut core = SessionCore::default();
    core.phase = SessionPhase::Idle;
    core.telemetry.process_started(AgentClockInstant(0));
    let Ok(Admission::Write(_)) = core.admit("go", None) else {
        panic!("an idle member starts a turn")
    };
    core.written(UserTurnId("u1".into()), "go");
    let now = AgentClockInstant(1);
    let closing = ExternalAgentEvent::Result(crate::domain::external_agent::stream::ResultEvent {
        is_error: Some(true),
        errors: vec!["closing".into()],
        user_turn_ids: vec!["u1".into()],
        ..Default::default()
    });
    let mut records = core.fold(&call("t0"), now, now).records;
    records.extend(
        event
            .map(|event| core.fold(event, now, now).records)
            .unwrap_or_default(),
    );
    records.extend(core.fold(&closing, now, now).records);
    records.iter().filter_map(family).collect()
}

/// #2304 review L1: the domain's table is what the session records. Each
/// event it maps to a family adds a record of that family to what the
/// turn records; each it ignores changes nothing.
#[test]
fn the_session_records_what_the_stream_telemetry_table_says() {
    use crate::domain::external_agent::telemetry::StreamTelemetry;
    let baseline = families_through_the_turn(None);
    for (event, expected) in crate::domain::external_agent::telemetry::tests::every_event_type() {
        let with = families_through_the_turn(Some(&event));
        let added: Vec<&str> = with
            .difference(&baseline)
            .map(|(family, _)| *family)
            .collect();
        match expected {
            StreamTelemetry::Recorded(family) => assert!(
                added.contains(&family),
                "{event:?} feeds {family}: added {added:?}"
            ),
            StreamTelemetry::Ignored => assert_eq!(with, baseline, "{event:?} is ignored"),
        }
    }
}
