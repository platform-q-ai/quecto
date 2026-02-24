use std::collections::HashMap;

use thiserror::Error;

use crate::domain::coding_event::{
    EventEnvelope, EventPayload, EventSource, is_compatible_version, is_known_event_type,
};

fn seq_key(source: EventSource, job_id: &str) -> String {
    format!("{source}:{job_id}")
}

pub fn next_seq_for(
    source: EventSource,
    job_id: &str,
    seq_by_source_job: &HashMap<String, u64>,
) -> u64 {
    seq_by_source_job
        .get(&seq_key(source, job_id))
        .copied()
        .unwrap_or(0)
        + 1
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
}

pub fn validate_and_track_event(
    envelope: &EventEnvelope,
    seq_by_source_job: &mut HashMap<String, u64>,
) -> Result<(), CodingContractError> {
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
    if envelope.run_id.is_empty() {
        return Err(CodingContractError::MissingEnvelopeField("run_id"));
    }
    if envelope.job_id.is_empty() {
        return Err(CodingContractError::MissingEnvelopeField("job_id"));
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

    let key = seq_key(envelope.source, &envelope.job_id);
    let prev = seq_by_source_job.get(&key).copied().unwrap_or(0);
    if envelope.seq <= prev {
        return Err(CodingContractError::InvalidSeq {
            expected_prev: prev,
            actual: envelope.seq,
        });
    }
    seq_by_source_job.insert(key, envelope.seq);
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
        let m = HashMap::new();
        assert_eq!(next_seq_for(EventSource::Worker, "job_1", &m), 1);
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
}
