//! #2210: the live progress of a request in flight, its output cap, and the
//! record of an attempt interrupted in flight.
use super::*;
use crate::domain::request_observation::MAX_ATTEMPT_RECORDS;
use std::time::Duration;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

#[test]
fn the_output_cap_is_eight_bytes_per_token_of_the_model_output_limit() {
    assert_eq!(OUTPUT_BYTES_PER_TOKEN, 8);
    assert_eq!(output_cap_bytes(Some(128_000), 8_192), 1_024_000);
    // A request asking for more than the model declares keeps its own.
    assert_eq!(output_cap_bytes(Some(4_096), 8_192), 65_536);
}

#[test]
fn an_unknown_output_limit_falls_back_to_a_default() {
    assert_eq!(FALLBACK_OUTPUT_TOKENS, 32_768);
    assert_eq!(output_cap_bytes(None, 8_192), 262_144);
    // A declared limit of zero is no limit known.
    assert_eq!(output_cap_bytes(Some(0), 8_192), 262_144);
    assert_eq!(output_cap_bytes(None, 100_000), 800_000);
}

#[test]
fn the_output_cap_error_names_the_cap_and_classifies_as_output_capped() {
    let error = OutputCapped { cap: 65_536 };
    let message = error.to_string();
    assert!(message.starts_with(OUTPUT_CAP_EXCEEDED), "{message}");
    assert!(message.contains("65536 bytes"), "{message}");
    let class = crate::domain::provider_error::classify_provider_error(
        &crate::domain::error::DomainError::Provider(message),
    );
    assert_eq!(
        class,
        crate::domain::provider_error::ProviderErrorClass::OutputCapped
    );
    assert!(!class.is_retryable());
}

#[test]
fn a_trace_has_no_output_cap_until_one_is_set() {
    let trace = RequestTrace::default();
    assert_eq!(trace.output_cap(), None);
    trace.set_output_cap(4_096);
    assert_eq!(trace.output_cap(), Some(4_096));
}

#[test]
#[should_panic(expected = "an output cap is more than zero")]
fn a_zero_output_cap_is_refused() {
    RequestTrace::default().set_output_cap(0);
}

#[test]
fn the_attempt_in_flight_counts_events_output_and_the_last_event() {
    let trace = RequestTrace::default();
    let start = Instant::now();
    assert_eq!(trace.attempt_progress(start), None);
    trace.begin_attempt(1, start, 1_000);
    trace.observe_event(start + ms(10), 0, false);
    trace.observe_event(start + ms(30), 5, true);
    trace.observe_event(start + ms(50), 7, true);
    let progress = trace.attempt_progress(start + ms(80)).unwrap();
    assert_eq!(
        progress,
        AttemptProgressSnapshot {
            number: 1,
            elapsed_ms: 80,
            events: 3,
            output_bytes: 12,
            since_last_event_ms: Some(30),
            first_token_ms: Some(30),
        }
    );
}

#[test]
fn an_attempt_before_its_first_event_has_no_last_event() {
    let trace = RequestTrace::default();
    let start = Instant::now();
    trace.begin_attempt(2, start, 1_000);
    let progress = trace.attempt_progress(start + ms(5)).unwrap();
    assert_eq!(progress.number, 2);
    assert_eq!(progress.events, 0);
    assert_eq!(progress.since_last_event_ms, None);
    assert_eq!(progress.first_token_ms, None);
}

#[test]
fn a_new_attempt_starts_its_counts_again() {
    let trace = RequestTrace::default();
    let start = Instant::now();
    trace.begin_attempt(1, start, 1_000);
    trace.observe_event(start + ms(1), 100, true);
    trace.record_attempt(AttemptDiagnostics {
        attempt_number: 1,
        ..Default::default()
    });
    assert_eq!(trace.attempt_progress(start + ms(2)), None);
    trace.begin_attempt(2, start + ms(3), 1_003);
    let progress = trace.attempt_progress(start + ms(4)).unwrap();
    assert_eq!((progress.number, progress.events), (2, 0));
    assert_eq!(progress.output_bytes, 0);
}

#[test]
fn an_event_before_an_attempt_is_not_counted_in_it() {
    let trace = RequestTrace::default();
    let start = Instant::now();
    trace.observe_event(start, 10, true);
    assert_eq!(trace.attempt_progress(start), None);
    trace.begin_attempt(1, start, 1_000);
    let progress = trace.attempt_progress(start).unwrap();
    assert_eq!((progress.events, progress.output_bytes), (0, 0));
    assert_eq!(progress.first_token_ms, None);
}

#[test]
fn an_attempt_open_when_its_request_ends_is_recorded_as_interrupted() {
    let trace = RequestTrace::default();
    let start = Instant::now();
    trace.begin_attempt(3, start, 5_000);
    trace.observe_event(start + ms(20), 0, false);
    trace.observe_event(start + ms(40), 9, true);
    let records = trace.attempts_when_dropped(start + ms(100), Some(5_100));
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record.attempt_number, 3);
    assert_eq!(record.started_unix_ms, 5_000);
    assert_eq!(record.finished_unix_ms, 5_100);
    assert_eq!(record.elapsed_ms, 100);
    assert_eq!(record.event_count, 2);
    assert_eq!(record.output_bytes, 9);
    assert_eq!(record.first_token_ms, Some(40));
    assert_eq!(record.termination, Termination::Interrupted);
}

#[test]
fn a_request_with_no_attempt_has_no_attempt_records() {
    let trace = RequestTrace::default();
    assert!(
        trace
            .attempts_when_dropped(Instant::now(), Some(1))
            .is_empty()
    );
}

/// An attempt that ended on its own keeps its own record.
#[test]
fn an_attempt_that_ended_keeps_its_own_record() {
    let trace = RequestTrace::default();
    let start = Instant::now();
    trace.begin_attempt(1, start, 1_000);
    trace.record_attempt(AttemptDiagnostics {
        attempt_number: 1,
        termination: Termination::Idle,
        ..Default::default()
    });
    let records = trace.attempts_when_dropped(start, Some(1));
    let terminations: Vec<_> = records.iter().map(|r| r.termination).collect();
    assert_eq!(terminations, [Termination::Idle]);
}

/// #2210 review: a transport run inside the request (every `chat`) is
/// dropped with it and records `Dropped` on its way out, after the request
/// marked itself dropping: that record is the interruption. An attempt
/// that genuinely ended `Dropped` before keeps its termination.
#[test]
fn only_a_transport_dropped_with_its_request_is_recorded_as_interrupted() {
    let trace = RequestTrace::default();
    let start = Instant::now();
    trace.begin_attempt(1, start, 1_000);
    trace.record_attempt(AttemptDiagnostics {
        attempt_number: 1,
        termination: Termination::Dropped,
        ..Default::default()
    });
    trace.begin_attempt(2, start, 1_000);
    trace.mark_dropping();
    trace.record_attempt(AttemptDiagnostics {
        attempt_number: 2,
        event_count: 4,
        output_bytes: 30,
        termination: Termination::Dropped,
        ..Default::default()
    });
    let records = trace.attempts_when_dropped(start, Some(1));
    let terminations: Vec<_> = records.iter().map(|r| r.termination).collect();
    assert_eq!(
        terminations,
        [Termination::Dropped, Termination::Interrupted]
    );
    assert_eq!((records[1].event_count, records[1].output_bytes), (4, 30));
    assert_eq!(trace.attempt_diagnostics(), records);
}

/// A request that is not being dropped keeps every `Dropped` record as it
/// is, the last one included.
#[test]
fn a_dropped_record_before_the_request_drops_keeps_its_termination() {
    let trace = RequestTrace::default();
    let start = Instant::now();
    trace.begin_attempt(1, start, 1_000);
    trace.record_attempt(AttemptDiagnostics {
        attempt_number: 1,
        termination: Termination::Dropped,
        ..Default::default()
    });
    let records = trace.attempts_when_dropped(start, Some(1));
    assert_eq!(records[0].termination, Termination::Dropped);
}

/// Marking the request dropping leaves every other termination alone.
#[test]
fn marking_a_request_dropping_relabels_only_dropped_records() {
    let trace = RequestTrace::default();
    trace.mark_dropping();
    trace.begin_attempt(1, Instant::now(), 1);
    trace.record_attempt(AttemptDiagnostics {
        attempt_number: 1,
        termination: Termination::Eof,
        ..Default::default()
    });
    assert_eq!(trace.attempt_diagnostics()[0].termination, Termination::Eof);
}

#[test]
fn a_late_record_of_an_earlier_attempt_leaves_the_one_in_flight() {
    let trace = RequestTrace::default();
    let start = Instant::now();
    trace.begin_attempt(2, start, 1_000);
    trace.record_attempt(AttemptDiagnostics {
        attempt_number: 1,
        ..Default::default()
    });
    assert_eq!(trace.attempt_progress(start).unwrap().number, 2);
}

#[test]
fn an_interrupted_attempt_without_a_wall_clock_keeps_its_start() {
    let trace = RequestTrace::default();
    let start = Instant::now();
    trace.begin_attempt(1, start, 7_000);
    let records = trace.attempts_when_dropped(start + ms(3), None);
    assert_eq!(records[0].finished_unix_ms, 7_000);
}

/// #2210 review: an interrupted attempt never takes a request past the
/// bounded prefix of attempt records.
#[test]
fn the_interrupted_attempt_respects_the_attempt_record_cap() {
    let trace = RequestTrace::default();
    let start = Instant::now();
    for number in 1..=MAX_ATTEMPT_RECORDS as u32 {
        trace.begin_attempt(number, start, 1);
        trace.record_attempt(AttemptDiagnostics {
            attempt_number: number,
            termination: Termination::Idle,
            ..Default::default()
        });
    }
    trace.begin_attempt(17, start, 1);
    let records = trace.attempts_when_dropped(start, Some(1));
    assert_eq!(records.len(), MAX_ATTEMPT_RECORDS);
    assert!(records.iter().all(|r| r.termination == Termination::Idle));
}

/// Wait until `trace`'s live lock is held by another thread: `true` once
/// it is, `false` when it never is within two seconds.
fn live_lock_taken(trace: &RequestTrace) -> bool {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if matches!(
            trace.live.try_lock(),
            Err(std::sync::TryLockError::WouldBlock)
        ) {
            return true;
        }
        std::thread::yield_now();
    }
    false
}

/// #2210 review: an attempt ending is closed and recorded under one live
/// lock, so a reader never sees it neither in flight nor recorded. The
/// records are held so the ending must wait between the two steps.
#[test]
fn an_attempt_ends_closed_and_recorded_under_one_live_lock() {
    let trace = Arc::new(RequestTrace::default());
    trace.begin_attempt(1, Instant::now(), 1);
    let records = trace.diagnostics.lock().unwrap();
    let ending = {
        let trace = trace.clone();
        std::thread::spawn(move || {
            trace.record_attempt(AttemptDiagnostics {
                attempt_number: 1,
                termination: Termination::Eof,
                ..Default::default()
            })
        })
    };
    // Held for good, not only while closing: 100 ms without a free moment.
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut blocked_since = None;
    loop {
        match trace.live.try_lock() {
            Err(std::sync::TryLockError::WouldBlock) => {
                let since = *blocked_since.get_or_insert_with(Instant::now);
                if since.elapsed() > Duration::from_millis(100) {
                    break;
                }
            }
            Ok(live) => {
                assert!(live.open, "closed while its record waits: seen as neither");
                blocked_since = None;
            }
            Err(error) => panic!("{error}"),
        }
        assert!(
            Instant::now() < deadline,
            "the ending never held the live lock"
        );
        std::thread::yield_now();
    }
    drop(records);
    ending.join().unwrap();
    assert_eq!(trace.attempt_diagnostics().len(), 1);
    assert_eq!(trace.attempt_progress(Instant::now()), None);
}

/// #2210 review: a dropped request's attempts are read under the live lock,
/// so an attempt cannot end between reading what is in flight and what is
/// recorded.
#[test]
fn a_dropped_requests_attempts_are_read_under_the_live_lock() {
    let trace = Arc::new(RequestTrace::default());
    trace.begin_attempt(1, Instant::now(), 1);
    let records = trace.diagnostics.lock().unwrap();
    let reading = {
        let trace = trace.clone();
        std::thread::spawn(move || trace.attempts_when_dropped(Instant::now(), Some(1)))
    };
    assert!(
        live_lock_taken(&trace),
        "the reader holds the live lock over the records"
    );
    drop(records);
    let read = reading.join().unwrap();
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].termination, Termination::Interrupted);
}

#[test]
fn ending_another_request_leaves_the_one_in_flight() {
    let in_flight = InFlightRequest::default();
    let start = Instant::now();
    let current = Arc::new(RequestTrace::default());
    in_flight.begin(start, current.clone());
    in_flight.end(&Arc::new(RequestTrace::default()));
    assert!(in_flight.snapshot(start).is_some());
    in_flight.end(&current);
    assert!(in_flight.snapshot(start).is_none());
}

#[test]
fn a_clock_behind_the_start_reads_as_no_time() {
    let in_flight = InFlightRequest::default();
    let start = Instant::now();
    let trace = Arc::new(RequestTrace::default());
    in_flight.begin(start + ms(50), trace.clone());
    trace.begin_attempt(1, start + ms(50), 1);
    let snapshot = in_flight.snapshot(start).unwrap();
    assert_eq!(snapshot.elapsed_ms, 0);
    assert_eq!(snapshot.attempt.unwrap().elapsed_ms, 0);
}

#[test]
fn the_snapshot_serializes_as_the_documented_camel_case_shape() {
    let snapshot = ModelTurnSnapshot {
        elapsed_ms: 90_000,
        output_cap_bytes: Some(1_024_000),
        attempt: Some(AttemptProgressSnapshot {
            number: 1,
            elapsed_ms: 89_000,
            events: 412,
            output_bytes: 20_480,
            since_last_event_ms: Some(35),
            first_token_ms: Some(2_100),
        }),
    };
    let json = serde_json::to_value(&snapshot).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "elapsedMs": 90_000,
            "outputCapBytes": 1_024_000,
            "attempt": {
                "number": 1,
                "elapsedMs": 89_000,
                "events": 412,
                "outputBytes": 20_480,
                "sinceLastEventMs": 35,
                "firstTokenMs": 2_100,
            }
        })
    );
    let back: ModelTurnSnapshot = serde_json::from_value(json).unwrap();
    assert_eq!(back, snapshot);
    let bare = ModelTurnSnapshot {
        elapsed_ms: 1,
        output_cap_bytes: None,
        attempt: None,
    };
    assert_eq!(
        serde_json::to_value(&bare).unwrap(),
        serde_json::json!({"elapsedMs": 1})
    );
}
