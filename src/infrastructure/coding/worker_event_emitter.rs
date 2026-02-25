//! Worker event emitter — writes EventEnvelopes as JSON Lines.
//!
//! The worker runs inside nsjail. It emits structured events to a
//! `Write` sink (stdout in production, buffer in tests). Each event
//! is serialized as a single JSON line terminated by `\n`.

use std::io::Write;

use crate::domain::coding_event::{EventEnvelope, EventSource, is_known_event_type};

/// Configuration for the event emitter.
#[derive(Debug)]
pub struct EmitterConfig {
    pub run_id: String,
    pub job_id: String,
    pub version: String,
}

/// Worker event emitter — builds and writes EventEnvelopes.
pub struct WorkerEventEmitter<W: Write> {
    config: EmitterConfig,
    writer: W,
    seq: u64,
}

impl<W: Write> std::fmt::Debug for WorkerEventEmitter<W> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerEventEmitter")
            .field("run_id", &self.config.run_id)
            .field("job_id", &self.config.job_id)
            .field("seq", &self.seq)
            .finish()
    }
}

impl<W: Write> WorkerEventEmitter<W> {
    /// Create a new emitter with the given configuration and sink.
    pub fn new(config: EmitterConfig, writer: W) -> Self {
        Self {
            config,
            writer,
            seq: 0,
        }
    }

    /// Emit an event with the given type and payload.
    ///
    /// Returns the sequence number assigned to this event, or an error
    /// if the event type is unknown or serialization fails.
    pub fn emit(&mut self, event_type: &str, payload: serde_json::Value) -> Result<u64, EmitError> {
        if !is_known_event_type(event_type) {
            return Err(EmitError::UnknownEventType(event_type.to_string()));
        }

        self.seq += 1;
        let envelope = EventEnvelope {
            v: self.config.version.clone(),
            ts: now_iso8601(),
            run_id: self.config.run_id.clone(),
            job_id: self.config.job_id.clone(),
            source: EventSource::Worker,
            event_type: event_type.to_string(),
            seq: self.seq,
            payload,
        };

        let json = serde_json::to_string(&envelope)
            .map_err(|e| EmitError::Serialization(e.to_string()))?;

        writeln!(self.writer, "{json}").map_err(|e| EmitError::Write(e.to_string()))?;

        Ok(self.seq)
    }

    /// Return the current sequence number (last assigned).
    pub fn current_seq(&self) -> u64 {
        self.seq
    }

    /// Get a reference to the inner writer (for testing).
    pub fn writer(&self) -> &W {
        &self.writer
    }
}

/// Errors from event emission.
#[derive(Debug)]
pub enum EmitError {
    UnknownEventType(String),
    Serialization(String),
    Write(String),
}

impl std::fmt::Display for EmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownEventType(t) => {
                write!(f, "unknown event type: {t}")
            }
            Self::Serialization(e) => write!(f, "serialization error: {e}"),
            Self::Write(e) => write!(f, "write error: {e}"),
        }
    }
}

/// Generate an ISO 8601 timestamp for the current UTC time.
fn now_iso8601() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_emitter() -> WorkerEventEmitter<Vec<u8>> {
        WorkerEventEmitter::new(
            EmitterConfig {
                run_id: "run-1".to_string(),
                job_id: "job-1".to_string(),
                version: "1.0".to_string(),
            },
            Vec::new(),
        )
    }

    fn parse_last_line(emitter: &WorkerEventEmitter<Vec<u8>>) -> serde_json::Value {
        let output = String::from_utf8(emitter.writer().clone()).unwrap();
        let last_line = output.lines().last().unwrap();
        serde_json::from_str(last_line).unwrap()
    }

    #[test]
    fn test_emit_produces_valid_json() {
        let mut emitter = test_emitter();
        emitter
            .emit(
                "log.message",
                serde_json::json!({"level":"info","message":"test"}),
            )
            .unwrap();
        let json = parse_last_line(&emitter);
        assert_eq!(json["source"], "worker");
        assert_eq!(json["type"], "log.message");
    }

    #[test]
    fn test_seq_increments() {
        let mut emitter = test_emitter();
        let s1 = emitter
            .emit(
                "log.message",
                serde_json::json!({"level":"info","message":"a"}),
            )
            .unwrap();
        let s2 = emitter
            .emit(
                "log.message",
                serde_json::json!({"level":"info","message":"b"}),
            )
            .unwrap();
        assert_eq!(s1, 1);
        assert_eq!(s2, 2);
    }

    #[test]
    fn test_envelope_fields() {
        let mut emitter = test_emitter();
        emitter
            .emit(
                "tool.start",
                serde_json::json!({"tool":"worker_edit","call_id":"c1"}),
            )
            .unwrap();
        let json = parse_last_line(&emitter);
        assert_eq!(json["v"], "1.0");
        assert_eq!(json["run_id"], "run-1");
        assert_eq!(json["job_id"], "job-1");
        assert_eq!(json["source"], "worker");
        assert_eq!(json["type"], "tool.start");
        assert_eq!(json["seq"], 1);
    }

    #[test]
    fn test_unknown_event_type_rejected() {
        let mut emitter = test_emitter();
        let result = emitter.emit("unknown.bad", serde_json::json!({}));
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("unknown event type"));
    }

    #[test]
    fn test_lines_end_with_newline() {
        let mut emitter = test_emitter();
        emitter
            .emit(
                "log.message",
                serde_json::json!({"level":"info","message":"x"}),
            )
            .unwrap();
        let output = String::from_utf8(emitter.writer().clone()).unwrap();
        assert!(output.ends_with('\n'));
    }

    #[test]
    fn test_each_event_is_separate_line() {
        let mut emitter = test_emitter();
        for _ in 0..3 {
            emitter
                .emit(
                    "log.message",
                    serde_json::json!({"level":"info","message":"x"}),
                )
                .unwrap();
        }
        let output = String::from_utf8(emitter.writer().clone()).unwrap();
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 3);
        for line in &lines {
            let _: serde_json::Value = serde_json::from_str(line).unwrap();
        }
    }

    #[test]
    fn test_timestamp_is_iso8601() {
        let mut emitter = test_emitter();
        emitter
            .emit(
                "log.message",
                serde_json::json!({"level":"info","message":"x"}),
            )
            .unwrap();
        let json = parse_last_line(&emitter);
        let ts = json["ts"].as_str().unwrap();
        // ISO 8601: should contain T and end with Z
        assert!(ts.contains('T'), "timestamp should contain T: {ts}");
        assert!(ts.ends_with('Z'), "timestamp should end with Z: {ts}");
    }

    #[test]
    fn test_payload_included_in_output() {
        let mut emitter = test_emitter();
        emitter
            .emit(
                "tool.result",
                serde_json::json!({
                    "tool": "worker_grep",
                    "call_id": "c2",
                    "ok": true,
                    "duration_ms": 42
                }),
            )
            .unwrap();
        let json = parse_last_line(&emitter);
        assert_eq!(json["payload"]["tool"], "worker_grep");
        assert_eq!(json["payload"]["duration_ms"], 42);
    }

    #[test]
    fn test_current_seq() {
        let mut emitter = test_emitter();
        assert_eq!(emitter.current_seq(), 0);
        emitter
            .emit(
                "log.message",
                serde_json::json!({"level":"info","message":"x"}),
            )
            .unwrap();
        assert_eq!(emitter.current_seq(), 1);
    }
}
