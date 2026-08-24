use super::*;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct CapturedLog(Arc<Mutex<String>>);

impl std::io::Write for CapturedLog {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap()
            .push_str(&String::from_utf8_lossy(buf));
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CapturedLog {
    type Writer = CapturedLog;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[test]
fn record_agent_result_emits_normalized_session_usage_log() {
    let logs = CapturedLog::default();
    let sink = logs.0.clone();
    let subscriber = tracing_subscriber::fmt()
        .json()
        .with_writer(logs)
        .with_ansi(false)
        .finish();

    tracing::subscriber::with_default(subscriber, || {
        let mut session = AgentSession::new("gpt-5".to_string(), "cli:test".to_string());
        let mut result = crate::domain::agent::AgentResult::text("ok");
        result.context_tokens = 105;
        result.billed_input_tokens = 70;
        result.billed_output_tokens = 20;
        result.cache_read_tokens = 30;
        result.cache_write_tokens = 5;
        result.cost_micro_usd = 1_234;

        session.record_agent_result(&result);
    });

    let output = sink.lock().unwrap().clone();
    let event: serde_json::Value = output
        .lines()
        .find_map(|line| serde_json::from_str(line).ok())
        .expect("json log event");
    let fields = &event["fields"];
    assert_eq!(fields["message"], "normalized session usage recorded");
    assert_eq!(event["target"], "session_usage");
    assert_eq!(fields["session_key"], "cli:test");
    assert_eq!(fields["input"], 70);
    assert_eq!(fields["output"], 20);
    assert_eq!(fields["cacheRead"], 30);
    assert_eq!(fields["cacheWrite"], 5);
    assert_eq!(fields["total"], 90);
    assert_eq!(fields["contextTokens"], 105);
    assert_eq!(fields["costMicroUsd"], 1_234);
    let ratio = fields["cacheHitRatio"].as_f64().expect("ratio field");
    assert!((ratio - (30.0 / 105.0)).abs() < 1e-9, "{event}");
}

#[test]
fn record_agent_result_without_usage_does_not_emit_session_usage_log() {
    let logs = CapturedLog::default();
    let sink = logs.0.clone();
    let subscriber = tracing_subscriber::fmt()
        .json()
        .with_writer(logs)
        .with_ansi(false)
        .finish();

    tracing::subscriber::with_default(subscriber, || {
        let mut session = AgentSession::new("gpt-5".to_string(), "cli:test".to_string());
        session.record_agent_result(&crate::domain::agent::AgentResult::text("ok"));
    });

    let output = sink.lock().unwrap().clone();
    assert!(
        !output.contains("normalized session usage recorded"),
        "{output}"
    );
    assert!(!output.contains("\"target\":\"session_usage\""), "{output}");
}
