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

/// #2151: the earliest first-token mark stays.
#[test]
fn the_earliest_first_token_mark_stays() {
    let trace = super::RequestTrace::default();
    assert_eq!(trace.first_token(), None);
    let early = std::time::Instant::now();
    let late = early + std::time::Duration::from_millis(5);
    trace.mark_first_token(late);
    trace.mark_first_token(early);
    trace.mark_first_token(late);
    assert_eq!(trace.first_token(), Some(early));
}

/// #2398: where a request's input first differs from its session's previous
/// request travels in the observation as counts, an index and a kind.
#[cfg(test)]
mod input_prefix_tests {
    use super::super::*;

    fn observation(input_prefix: Option<InputPrefix>) -> RequestObservation {
        RequestObservation {
            request_id: "r".into(),
            started_unix_ms: None,
            finished_unix_ms: None,
            attempt_diagnostics: Vec::new(),
            model: "m".into(),
            provider: "codex".into(),
            outcome: "succeeded".into(),
            error_class: None,
            input_tokens: None,
            context_input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
            estimated_cost_micro_usd: None,
            estimated_context_tokens: 0,
            instrumented_attempts: 1,
            oauth_retries: 0,
            duration_ms: 0,
            first_token_ms: None,
            harness_prefix_sha256: String::new(),
            harness_prefix_bytes: 0,
            harness_prefix_unchanged: None,
            input_prefix,
        }
    }

    /// An append-only request of 5 items after one of 4.
    fn append_only() -> InputPrefixParts {
        InputPrefixParts {
            input_items: 5,
            previous_items: Some(4),
            first_changed_item: None,
            first_changed_kind: None,
            prefix_tokens_estimate: 40,
            unchanged_prefix_tokens_estimate: 140,
            request_tokens_estimate: 150,
        }
    }

    /// Item 3 of 9 changed in place.
    fn changed() -> InputPrefixParts {
        InputPrefixParts {
            input_items: 9,
            previous_items: Some(8),
            first_changed_item: Some(3),
            first_changed_kind: Some(InputItemKind::FunctionCallOutput),
            prefix_tokens_estimate: 12,
            unchanged_prefix_tokens_estimate: 112,
            request_tokens_estimate: 300,
        }
    }

    /// A session's first observed request: nothing compared.
    fn uncompared() -> InputPrefixParts {
        InputPrefixParts {
            input_items: 2,
            previous_items: None,
            first_changed_item: None,
            first_changed_kind: None,
            prefix_tokens_estimate: 0,
            unchanged_prefix_tokens_estimate: 0,
            request_tokens_estimate: 90,
        }
    }

    #[test]
    fn an_append_only_input_records_a_null_first_changed_item() {
        let record = observation(Some(InputPrefix::new(append_only()).unwrap()));
        let value = serde_json::to_value(&record).unwrap();
        assert_eq!(value["input_items"], 5);
        assert_eq!(value["previous_items"], 4);
        assert!(
            value
                .as_object()
                .unwrap()
                .contains_key("first_changed_item")
        );
        assert_eq!(value["first_changed_item"], serde_json::Value::Null);
        assert_eq!(value["first_changed_kind"], serde_json::Value::Null);
        assert_eq!(value["prefix_tokens_estimate"], 40);
        assert_eq!(value["unchanged_prefix_tokens_estimate"], 140);
        assert_eq!(value["request_tokens_estimate"], 150);
        let back: RequestObservation = serde_json::from_value(value).unwrap();
        assert_eq!(back, record);
    }

    #[test]
    fn a_changed_item_records_its_index_and_kind() {
        let record = observation(Some(InputPrefix::new(changed()).unwrap()));
        let value = serde_json::to_value(&record).unwrap();
        assert_eq!(value["first_changed_item"], 3);
        assert_eq!(value["first_changed_kind"], "function_call_output");
        let back: RequestObservation = serde_json::from_value(value).unwrap();
        assert_eq!(back, record);
    }

    /// Nothing compared is not the same record as append-only.
    #[test]
    fn an_uncompared_request_records_a_null_previous_count() {
        let value =
            serde_json::to_value(observation(Some(InputPrefix::new(uncompared()).unwrap())))
                .unwrap();
        assert!(value.as_object().unwrap().contains_key("previous_items"));
        assert_eq!(value["previous_items"], serde_json::Value::Null);
        assert_eq!(value["first_changed_item"], serde_json::Value::Null);
    }

    /// A provider that does not observe its input writes none of the
    /// fields, and a record without them reads as unobserved.
    #[test]
    fn an_unobserved_input_writes_no_fields_and_reads_back_unobserved() {
        let value = serde_json::to_value(observation(None)).unwrap();
        for key in [
            "input_items",
            "previous_items",
            "first_changed_item",
            "first_changed_kind",
            "prefix_tokens_estimate",
            "unchanged_prefix_tokens_estimate",
            "request_tokens_estimate",
        ] {
            assert!(!value.as_object().unwrap().contains_key(key), "{key}");
        }
        let back: RequestObservation = serde_json::from_value(value).unwrap();
        assert_eq!(back.input_prefix, None);
    }

    #[test]
    fn every_kind_has_its_wire_name() {
        for (kind, name) in [
            (InputItemKind::User, "user"),
            (InputItemKind::Assistant, "assistant"),
            (InputItemKind::FunctionCall, "function_call"),
            (InputItemKind::FunctionCallOutput, "function_call_output"),
            (InputItemKind::Reasoning, "reasoning"),
        ] {
            assert_eq!(serde_json::to_value(kind).unwrap(), name);
        }
    }

    /// An edit making a consistent record inconsistent.
    type Edit = fn(&mut InputPrefixParts);

    /// Each inconsistent record is refused, by the constructor and when read.
    #[test]
    fn an_inconsistent_record_is_refused() {
        let cases: [(&str, Edit); 13] = [
            ("a kind without an index", |p| {
                p.first_changed_item = None;
            }),
            ("an index without a previous input", |p| {
                p.previous_items = None;
                p.prefix_tokens_estimate = 0;
                p.unchanged_prefix_tokens_estimate = 0;
            }),
            ("an uncompared request with an item prefix", |p| {
                *p = InputPrefixParts {
                    prefix_tokens_estimate: 1,
                    ..uncompared_parts()
                };
            }),
            ("an uncompared request with a whole prefix", |p| {
                *p = InputPrefixParts {
                    unchanged_prefix_tokens_estimate: 1,
                    ..uncompared_parts()
                };
            }),
            ("append-only after a longer input", |p| {
                p.first_changed_item = None;
                p.first_changed_kind = None;
                p.previous_items = Some(10);
            }),
            ("an index past the previous input", |p| {
                p.first_changed_item = Some(8);
            }),
            ("an index two past this input", |p| {
                p.previous_items = Some(20);
                p.first_changed_item = Some(10);
                p.first_changed_kind = None;
            }),
            ("a kind for an item this request lacks", |p| {
                p.previous_items = Some(20);
                p.first_changed_item = Some(9);
            }),
            ("a prefix before the first item", |p| {
                p.first_changed_item = Some(0);
            }),
            ("an item prefix after an empty previous input", |p| {
                p.previous_items = Some(0);
                p.first_changed_item = None;
                p.first_changed_kind = None;
                p.prefix_tokens_estimate = 50;
            }),
            ("an item prefix larger than its request", |p| {
                p.prefix_tokens_estimate = 301;
                p.unchanged_prefix_tokens_estimate = 0;
            }),
            ("a whole prefix larger than its request", |p| {
                p.unchanged_prefix_tokens_estimate = 301;
            }),
            ("a whole prefix smaller than its item prefix", |p| {
                p.unchanged_prefix_tokens_estimate = 11;
            }),
        ];
        for (case, edit) in cases {
            let mut parts = changed();
            edit(&mut parts);
            assert!(InputPrefix::new(parts).is_err(), "{case}: {parts:?}");
            let read = serde_json::from_value::<InputPrefix>(serde_json::to_value(parts).unwrap());
            assert!(read.is_err(), "{case}");
        }
        for parts in [append_only(), changed(), uncompared()] {
            assert_eq!(InputPrefix::new(parts).map(InputPrefix::parts), Ok(parts));
        }
        // A changed instruction or tool leaves no whole prefix, whatever
        // the items kept; a truncated input names the first item it lacks.
        let head_changed = InputPrefixParts {
            unchanged_prefix_tokens_estimate: 0,
            ..changed()
        };
        let truncated = InputPrefixParts {
            previous_items: Some(12),
            first_changed_item: Some(9),
            first_changed_kind: None,
            ..changed()
        };
        for parts in [head_changed, truncated] {
            assert!(InputPrefix::new(parts).is_ok(), "{parts:?}");
        }
    }

    fn uncompared_parts() -> InputPrefixParts {
        uncompared()
    }

    /// A retry sends the same input again: the first record stays.
    #[test]
    fn the_first_input_prefix_a_request_records_stays() {
        let trace = RequestTrace::default();
        assert_eq!(trace.input_prefix(), None);
        let first = InputPrefix::new(changed()).unwrap();
        trace.record_input_prefix(first);
        trace.record_input_prefix(InputPrefix::new(append_only()).unwrap());
        assert_eq!(trace.input_prefix(), Some(first));
    }
}

/// #2398: the opaque baseline a session owns and its requests' traces carry.
#[cfg(test)]
mod input_baseline_tests {
    use super::super::*;

    #[test]
    fn a_baseline_keeps_one_value_its_clones_share() {
        let baseline = InputBaseline::default();
        assert_eq!(baseline.read(|kept: Option<&u64>| kept.copied()), None);
        let shared = baseline.clone();
        shared.keep(7u64);
        assert_eq!(baseline.read(|kept: Option<&u64>| kept.copied()), Some(7));
        assert_eq!(baseline.read(|kept: Option<&String>| kept.cloned()), None);
        assert!(baseline.is(&shared));
        assert!(!baseline.is(&InputBaseline::default()));
        assert_eq!(format!("{baseline:?}"), "InputBaseline(..)");
    }

    #[test]
    fn a_trace_carries_the_first_baseline_attached() {
        let trace = RequestTrace::default();
        assert!(trace.input_baseline().is_none());
        let baseline = InputBaseline::default();
        trace.attach_input_baseline(baseline.clone());
        trace.attach_input_baseline(InputBaseline::default());
        assert!(trace.input_baseline().expect("attached").is(&baseline));
    }
}
