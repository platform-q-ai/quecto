//! Contract for [`ExternalAgentTelemetry`] (#2287), proven on the
//! production `TracingExternalAgentTelemetry`: every record is logged once,
//! under the `quecto::external_agent` target, with its kind; recording
//! never fails the caller.
use std::io::Write;
use std::sync::{Arc, Mutex};

use quecto::application::external_agent::dto::{PromptAccepted, SessionRecord};
use quecto::application::external_agent::ports::ExternalAgentTelemetry;
use quecto::infrastructure::external_agents::telemetry::TracingExternalAgentTelemetry;

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
        SessionRecord::Ended { clean: true },
    ]
}

/// Every record kind, once each (a kind added to [`SessionRecord`] is
/// added here).
#[test]
fn every_kind_is_listed_once() {
    let kinds: std::collections::BTreeSet<&str> =
        every_kind().iter().map(SessionRecord::kind).collect();
    assert_eq!(kinds.len(), every_kind().len());
    assert_eq!(kinds.len(), 16, "{kinds:?}");
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
