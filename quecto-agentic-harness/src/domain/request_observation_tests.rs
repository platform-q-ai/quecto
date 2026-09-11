#[cfg(test)]
mod retention_tests {
    use super::super::*;

    #[test]
    fn maximum_produced_attempt_payload_fits_accounting_envelope() {
        use crate::domain::attempt_diagnostics::{HeaderName, HeaderValue, SafeHeader};
        let trace = RequestTrace::default();
        for attempt_number in 1..=16 {
            trace.record_attempt(AttemptDiagnostics {
                attempt_number,
                started_unix_ms: u64::MAX,
                finished_unix_ms: u64::MAX,
                elapsed_ms: u64::MAX,
                headers: (0..10)
                    .map(|_| SafeHeader {
                        name: HeaderName::XRequestId,
                        value: HeaderValue::Sha256("a".repeat(64)),
                    })
                    .collect(),
                ..Default::default()
            });
        }
        // Leave at least 4 KiB for existing observation/runtime measurements.
        assert!(
            serde_json::to_vec(&trace.attempt_diagnostics())
                .unwrap()
                .len()
                < 28_672
        );
    }

    #[test]
    fn attempt_retention_is_bounded_and_preserves_attempt_numbers() {
        let trace = RequestTrace::default();
        for attempt_number in 1..=20 {
            trace.record_attempt(AttemptDiagnostics {
                attempt_number,
                wire_status: Some(200),
                ..Default::default()
            });
        }
        let records = trace.attempt_diagnostics();
        assert_eq!(records.len(), 16);
        assert_eq!(records[0].attempt_number, 1);
        assert_eq!(records[15].attempt_number, 16);
    }
}
#[cfg(test)]
mod compatibility_tests {
    use super::super::*;

    #[test]
    fn legacy_observation_has_unavailable_wire_evidence() {
        let legacy = serde_json::json!({
            "request_id":"legacy", "model":"test", "provider":"test", "outcome":"failed",
            "error_class":"server", "estimated_context_tokens":0, "instrumented_attempts":3,
            "oauth_retries":0, "duration_ms":10, "harness_prefix_sha256":"",
            "harness_prefix_bytes":0
        });
        let record: RequestObservation = serde_json::from_value(legacy).unwrap();
        assert_eq!(record.started_unix_ms, None);
        assert_eq!(record.finished_unix_ms, None);
        assert!(record.attempt_diagnostics.is_empty());
    }
}

#[cfg(test)]
mod runtime_identity_pid_tests {
    use super::super::RuntimeIdentity;

    /// #1925: `pid` is additive on the wire; a pre-#1925 peer that omits it
    /// deserializes to 0, which restore treats as "not confirmed".
    #[test]
    fn runtime_identity_pid_defaults_to_zero_and_round_trips() {
        let legacy: RuntimeIdentity = serde_json::from_value(serde_json::json!({
            "process_instance_id": "peer",
            "executable_digest_pending": false,
            "package_version": "0.0.0",
            "build_source_revision": null,
            "build_dirty": null,
            "executable_sha256": null
        }))
        .unwrap();
        assert_eq!(legacy.pid, 0);

        let current = crate::infrastructure::runtime_identity::current();
        assert_eq!(current.pid, std::process::id());
        let wire = serde_json::to_value(&current).unwrap();
        assert_eq!(wire["pid"], serde_json::json!(std::process::id()));
        let back: RuntimeIdentity = serde_json::from_value(wire).unwrap();
        assert_eq!(back, current);
    }
}
