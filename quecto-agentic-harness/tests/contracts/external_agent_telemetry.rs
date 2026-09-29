//! Contract for [`ExternalAgentTelemetry`] (#2287), proven on the
//! production `TracingExternalAgentTelemetry`: every record is logged once,
//! under the `quecto::external_agent` target, with its kind; recording
//! never fails the caller. The event-log adapter (#2304) files every record
//! the log keeps into the production `AuditLog`, once each, in order.
use std::io::Write;
use std::sync::{Arc, Mutex};

use quecto::application::external_agent::dto::{PromptAccepted, SessionRecord};
use quecto::application::external_agent::ports::ExternalAgentTelemetry;
use quecto::domain::external_agent::telemetry::{
    ExternalAgentStreamDiagnostic, ExternalAgentTool, ExternalAgentTurn,
};
use quecto::infrastructure::external_agents::event_log::MemberIdentity;
use quecto::infrastructure::external_agents::telemetry::{
    EventLogExternalAgentTelemetry, TracingExternalAgentTelemetry,
};
use quecto::infrastructure::persistence::audit_log::AuditLog;

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
    type Writer = Captured;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

fn every_kind() -> Vec<SessionRecord> {
    vec![
        SessionRecord::Started,
        SessionRecord::StartRefused { kind: "not_found" },
        SessionRecord::PromptAccepted {
            accepted: PromptAccepted::Started { turn: 1 },
            bytes: 3,
        },
        SessionRecord::PromptRefused {
            refusal: "busy",
            bytes: 3,
        },
        SessionRecord::ToolCalled {
            turn: Some(1),
            tool: "Bash".into(),
        },
        SessionRecord::LineSkipped {
            turn: Some(1),
            bytes: 9,
        },
        SessionRecord::TurnEnded {
            turn: 1,
            outcome: "completed",
            duration_ms: Some(5),
            cost_micro_usd: 7,
        },
        SessionRecord::TurnContinued { turn: 1, owed: 1 },
        SessionRecord::Interrupted {
            turn: 1,
            cause: "abort",
        },
        SessionRecord::ResultWithoutIds {
            turn: 1,
            ended: true,
        },
        SessionRecord::Abandoned {
            turn: 1,
            dropped_follow_ups: 1,
        },
        SessionRecord::FollowUpStarted { turn: 2, bytes: 3 },
        SessionRecord::FollowUpFailed {
            turn: 2,
            bytes: 3,
            refusal: "input",
        },
        SessionRecord::Aborted {
            turn: Some(2),
            dropped_follow_ups: 1,
        },
        SessionRecord::Closed {
            turn: None,
            dropped_follow_ups: 0,
        },
        SessionRecord::Ended {
            clean: true,
            exit_code: Some(0),
            signal: None,
            wall_ms: Some(10),
        },
        SessionRecord::TurnReported(Box::new(turn())),
        SessionRecord::ToolFinished(Box::new(tool())),
        SessionRecord::Initialized {
            cli_version: Some("2.1.280".into()),
            claude_session_id: None,
            model: None,
        },
        SessionRecord::StreamDiagnostic(ExternalAgentStreamDiagnostic {
            kind: "unknown_event".into(),
            name: "x".into(),
            bytes: None,
            count: 1,
            member_turn: None,
        }),
    ]
}

fn turn() -> ExternalAgentTurn {
    ExternalAgentTurn {
        member_turn: 1,
        turn_end: "completed".into(),
        reason_kind: None,
        claude_session_id: None,
        model: None,
        is_error: Some(false),
        num_turns: Some(1),
        duration_ms: Some(5),
        duration_api_ms: Some(4),
        input_tokens: 1,
        output_tokens: 2,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        list_price_cost_micro_usd: 7,
        list_price_total_micro_usd: 7,
        task_id: None,
    }
}

fn tool() -> ExternalAgentTool {
    ExternalAgentTool {
        member_turn: Some(1),
        tool: "Bash".into(),
        tool_use_id: "t1".into(),
        duration_ms: 3,
        outcome: "ok".into(),
        rule_id: None,
        argument_bytes: 10,
        result_bytes: 5,
        summary: None,
        task_id: None,
    }
}

/// Every record kind, once each (a kind added to [`SessionRecord`] is
/// added here).
#[test]
fn every_kind_is_listed_once() {
    let kinds: std::collections::BTreeSet<&str> =
        every_kind().iter().map(SessionRecord::kind).collect();
    assert_eq!(kinds.len(), every_kind().len());
    assert_eq!(kinds.len(), 20, "{kinds:?}");
}

#[test]
fn every_record_is_logged_once_under_the_external_agent_target() {
    let captured = Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(captured.clone())
        .with_ansi(false)
        .finish();
    let records = every_kind();
    tracing::subscriber::with_default(subscriber, || {
        for record in &records {
            TracingExternalAgentTelemetry.record(record);
        }
    });
    let text = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), records.len(), "{text}");
    for (line, record) in lines.iter().zip(&records) {
        assert!(line.contains("quecto::external_agent"), "{line}");
        assert!(
            line.contains(&format!("kind=\"{}\"", record.kind())),
            "{line}"
        );
    }
}

#[test]
fn recording_without_a_subscriber_is_harmless() {
    for record in every_kind() {
        TracingExternalAgentTelemetry.record(&record);
    }
}

#[test]
fn the_event_log_adapter_files_each_kept_record_once_in_order() {
    let base = tempfile::tempdir().unwrap();
    let log = AuditLog::open_sync(base.path(), "cli:w1")
        .unwrap()
        .with_parent(Some("cli:coordinator".into()));
    let telemetry = EventLogExternalAgentTelemetry::new(
        Arc::new(log),
        MemberIdentity {
            member_ref: "C1".into(),
            credential_mode: "api_key",
        },
    )
    .unwrap();
    for record in every_kind() {
        telemetry.record(&record);
    }
    assert_eq!(telemetry.failures(), 0);
    drop(telemetry);
    let text = std::fs::read_to_string(AuditLog::file_path(base.path(), "cli:w1")).unwrap();
    let events: Vec<serde_json::Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let names: Vec<&str> = events
        .iter()
        .map(|event| event["event"].as_str().unwrap())
        .collect();
    let lifecycle = "external_agent_lifecycle";
    assert_eq!(
        names,
        [
            lifecycle,
            lifecycle,
            lifecycle,
            lifecycle,
            lifecycle,
            lifecycle,
            lifecycle,
            "external_agent_turn",
            "external_agent_tool",
            lifecycle,
            "external_agent_stream_diagnostic",
        ]
    );
    for event in &events {
        assert_eq!(event["session"], "cli:w1");
        assert_eq!(event["parent"], "cli:coordinator");
        assert_eq!(event["member_ref"], "C1");
    }
}
