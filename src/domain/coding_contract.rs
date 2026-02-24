use std::collections::HashMap;

use thiserror::Error;

use super::coding_event::{
    EventEnvelope, EventPayload, EventSource, is_compatible_version, is_known_event_type,
};

pub const MAX_ID_LEN: usize = 128;

pub fn is_valid_runtime_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ID_LEN
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SeqScope {
    pub source: EventSource,
    pub run_id: String,
    pub job_id: String,
}

impl SeqScope {
    pub fn new(source: EventSource, run_id: impl Into<String>, job_id: impl Into<String>) -> Self {
        Self {
            source,
            run_id: run_id.into(),
            job_id: job_id.into(),
        }
    }
}

pub fn next_seq_for(scope: &SeqScope, seq_by_scope: &HashMap<SeqScope, u64>) -> u64 {
    seq_by_scope.get(scope).copied().unwrap_or(0) + 1
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CodingContractError {
    #[error("incompatible event version: {0}")]
    IncompatibleVersion(String),
    #[error("unknown event type: {0}")]
    UnknownEventType(String),
    #[error("missing required envelope field: {0}")]
    MissingEnvelopeField(&'static str),
    #[error("invalid seq: expected > {expected_prev}, got {actual}")]
    InvalidSeq { expected_prev: u64, actual: u64 },
    #[error("invalid payload for event type {event_type}: {reason}")]
    InvalidPayload { event_type: String, reason: String },
    #[error("scope mismatch between envelope and tracker key")]
    ScopeMismatch,
    #[error("invalid identifier for field {field}")]
    InvalidIdentifier { field: &'static str },
}

fn validate_event_envelope(envelope: &EventEnvelope) -> Result<(), CodingContractError> {
    if !is_compatible_version(&envelope.v) {
        return Err(CodingContractError::IncompatibleVersion(envelope.v.clone()));
    }
    if !is_known_event_type(&envelope.event_type) {
        return Err(CodingContractError::UnknownEventType(
            envelope.event_type.clone(),
        ));
    }
    if envelope.ts.is_empty() {
        return Err(CodingContractError::MissingEnvelopeField("ts"));
    }
    if !is_valid_runtime_id(&envelope.run_id) {
        return Err(CodingContractError::InvalidIdentifier { field: "run_id" });
    }
    if !is_valid_runtime_id(&envelope.job_id) {
        return Err(CodingContractError::InvalidIdentifier { field: "job_id" });
    }
    if !envelope.payload.is_object() {
        return Err(CodingContractError::InvalidPayload {
            event_type: envelope.event_type.clone(),
            reason: "payload must be an object".to_string(),
        });
    }

    let mut payload_obj = envelope
        .payload
        .as_object()
        .cloned()
        .expect("checked object above");
    payload_obj.insert(
        "type".to_string(),
        serde_json::Value::String(envelope.event_type.clone()),
    );
    let payload_value = serde_json::Value::Object(payload_obj);
    if let Err(err) = serde_json::from_value::<EventPayload>(payload_value) {
        return Err(CodingContractError::InvalidPayload {
            event_type: envelope.event_type.clone(),
            reason: err.to_string(),
        });
    }

    Ok(())
}

pub fn validate_and_track_event(
    envelope: &EventEnvelope,
    seq_by_scope: &mut HashMap<SeqScope, u64>,
) -> Result<(), CodingContractError> {
    validate_event_envelope(envelope)?;

    let scope = SeqScope::new(envelope.source, &envelope.run_id, &envelope.job_id);
    track_event_seq(envelope, scope, seq_by_scope)
}

pub fn validate_and_track_event_with_scope(
    envelope: &EventEnvelope,
    scope: SeqScope,
    seq_by_scope: &mut HashMap<SeqScope, u64>,
) -> Result<(), CodingContractError> {
    validate_event_envelope(envelope)?;

    let derived = SeqScope::new(envelope.source, &envelope.run_id, &envelope.job_id);
    if scope != derived {
        return Err(CodingContractError::ScopeMismatch);
    }

    track_event_seq(envelope, scope, seq_by_scope)
}

fn track_event_seq(
    envelope: &EventEnvelope,
    scope: SeqScope,
    seq_by_scope: &mut HashMap<SeqScope, u64>,
) -> Result<(), CodingContractError> {
    let prev = seq_by_scope.get(&scope).copied().unwrap_or(0);
    if envelope.seq <= prev {
        return Err(CodingContractError::InvalidSeq {
            expected_prev: prev,
            actual: envelope.seq,
        });
    }
    seq_by_scope.insert(scope, envelope.seq);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(
        event_type: &str,
        seq: u64,
        source: EventSource,
        payload: serde_json::Value,
    ) -> EventEnvelope {
        EventEnvelope {
            v: "1.0".to_string(),
            ts: "2026-01-01T00:00:00Z".to_string(),
            run_id: "run_1".to_string(),
            job_id: "job_1".to_string(),
            source,
            event_type: event_type.to_string(),
            seq,
            payload,
        }
    }

    #[test]
    fn test_next_seq_for_starts_at_one() {
        let scope = SeqScope::new(EventSource::Worker, "run_1", "job_1");
        let m = HashMap::new();
        assert_eq!(next_seq_for(&scope, &m), 1);
    }

    #[test]
    fn test_validate_and_track_event_accepts_valid_event() {
        let mut seq = HashMap::new();
        let env = event(
            "tool.start",
            1,
            EventSource::Worker,
            serde_json::json!({"tool":"read_file","call_id":"c1"}),
        );
        assert_eq!(validate_and_track_event(&env, &mut seq), Ok(()));
    }

    #[test]
    fn test_validate_and_track_event_rejects_bad_seq() {
        let mut seq = HashMap::new();
        let first = event(
            "tool.start",
            2,
            EventSource::Worker,
            serde_json::json!({"tool":"read_file","call_id":"c1"}),
        );
        assert_eq!(validate_and_track_event(&first, &mut seq), Ok(()));

        let second = event(
            "tool.result",
            2,
            EventSource::Worker,
            serde_json::json!({"tool":"read_file","call_id":"c1","ok":true}),
        );
        assert_eq!(
            validate_and_track_event(&second, &mut seq),
            Err(CodingContractError::InvalidSeq {
                expected_prev: 2,
                actual: 2
            })
        );
    }

    #[test]
    fn test_validate_and_track_event_rejects_unknown_type() {
        let mut seq = HashMap::new();
        let env = event(
            "unknown.future_event",
            1,
            EventSource::Worker,
            serde_json::json!({"foo":"bar"}),
        );
        assert_eq!(
            validate_and_track_event(&env, &mut seq),
            Err(CodingContractError::UnknownEventType(
                "unknown.future_event".to_string()
            ))
        );
    }

    #[test]
    fn test_validate_and_track_event_rejects_payload_shape_mismatch() {
        let mut seq = HashMap::new();
        let env = event(
            "tool.result",
            1,
            EventSource::Worker,
            serde_json::json!({"tool":"exec","call_id":"c1"}),
        );
        let res = validate_and_track_event(&env, &mut seq);
        assert!(matches!(
            res,
            Err(CodingContractError::InvalidPayload { .. })
        ));
    }

    #[test]
    fn test_seq_scope_isolated_by_run_id() {
        let mut seq = HashMap::new();
        let first = EventEnvelope {
            v: "1.0".to_string(),
            ts: "2026-01-01T00:00:00Z".to_string(),
            run_id: "run_1".to_string(),
            job_id: "job_1".to_string(),
            source: EventSource::Worker,
            event_type: "tool.start".to_string(),
            seq: 1,
            payload: serde_json::json!({"tool":"read_file","call_id":"c1"}),
        };
        assert_eq!(validate_and_track_event(&first, &mut seq), Ok(()));

        let second_other_run = EventEnvelope {
            run_id: "run_2".to_string(),
            ..first
        };
        assert_eq!(
            validate_and_track_event(&second_other_run, &mut seq),
            Ok(())
        );
    }

    #[test]
    fn test_validate_with_scope_rejects_mismatch() {
        let mut seq = HashMap::new();
        let env = event(
            "tool.start",
            1,
            EventSource::Worker,
            serde_json::json!({"tool":"read_file","call_id":"c1"}),
        );
        let wrong_scope = SeqScope::new(EventSource::Worker, "run_other", "job_1");
        assert_eq!(
            validate_and_track_event_with_scope(&env, wrong_scope, &mut seq),
            Err(CodingContractError::ScopeMismatch)
        );
    }

    #[test]
    fn test_rejects_invalid_run_id_characters() {
        let mut seq = HashMap::new();
        let mut env = event(
            "tool.start",
            1,
            EventSource::Worker,
            serde_json::json!({"tool":"read_file","call_id":"c1"}),
        );
        env.run_id = "run/1".to_string();
        assert_eq!(
            validate_and_track_event(&env, &mut seq),
            Err(CodingContractError::InvalidIdentifier { field: "run_id" })
        );
    }

    #[test]
    fn test_rejects_invalid_job_id_length() {
        let mut seq = HashMap::new();
        let mut env = event(
            "tool.start",
            1,
            EventSource::Worker,
            serde_json::json!({"tool":"read_file","call_id":"c1"}),
        );
        env.job_id = "a".repeat(MAX_ID_LEN + 1);
        assert_eq!(
            validate_and_track_event(&env, &mut seq),
            Err(CodingContractError::InvalidIdentifier { field: "job_id" })
        );
    }
}
