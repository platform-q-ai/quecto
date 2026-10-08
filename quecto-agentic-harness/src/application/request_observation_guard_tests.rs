use super::*;
use crate::domain::inference::events::request_progress::InFlightRequest;
use crate::domain::inference::value_objects::attempt_diagnostics::Termination;

/// The sinks one guard publishes to.
#[derive(Default)]
struct Sinks {
    log: Mutex<RequestDiagnostics>,
    in_flight: InFlightRequest,
    interrupted: Mutex<Vec<InterruptedRequest>>,
}

impl Sinks {
    fn guard(&self, trace: &Arc<RequestTrace>) -> ObservationGuard<'_> {
        let messages =
            vec![crate::domain::conversation::value_objects::message::Message::user("hi")];
        let request = ChatRequest {
            trace: None,
            admission: None,
            model: "m",
            messages: &messages,
            tools: &[],
            max_tokens: 1,
            temperature: 0.0,
            thinking_level: None,
            effort: None,
            tool_choice: None,
            metadata: None,
            session_id: None,
            cancel_flag: None,
        };
        ObservationGuard::new(
            ObservationSinks {
                log: &self.log,
                outbox: None,
                in_flight: &self.in_flight,
                interrupted: &self.interrupted,
                turn: 7,
            },
            &request,
            "p",
            0,
            PrefixObservation {
                sha256: String::new(),
                bytes: 0,
                unchanged: None,
            },
            trace.clone(),
        )
    }

    fn interrupted(&self) -> Vec<RequestObservation> {
        let queued = self.interrupted.lock().unwrap().clone();
        assert!(queued.iter().all(|request| request.turn == 7), "{queued:?}");
        queued
            .into_iter()
            .map(|request| request.observation)
            .collect()
    }
}

/// #2151: a published observation carries the request's time to first
/// token, taken from its attempts.
#[test]
fn a_published_observation_carries_the_time_to_first_token() {
    let sinks = Sinks::default();
    let trace = Arc::new(RequestTrace::default());
    let mut guard = sinks.guard(&trace);
    std::thread::sleep(std::time::Duration::from_millis(20));
    trace.mark_first_token(std::time::Instant::now());
    let record = guard.finish(&Err(DomainError::Other("stopped".into())));
    let first = record.first_token_ms.expect("time to first token");
    assert!(first >= 20, "{first}");
    assert!(
        first <= record.duration_ms,
        "{first} {}",
        record.duration_ms
    );
}

/// #2210: `get_state` sees the request from its start until it finishes.
#[test]
fn a_request_is_in_flight_until_it_finishes() {
    let sinks = Sinks::default();
    let trace = Arc::new(RequestTrace::default());
    trace.set_output_cap(64);
    let mut guard = sinks.guard(&trace);
    let progress = sinks.in_flight.snapshot(Instant::now()).unwrap();
    assert_eq!(progress.output_cap_bytes, Some(64));
    guard.finish(&Err(DomainError::Other("failed".into())));
    assert_eq!(sinks.in_flight.snapshot(Instant::now()), None);
    drop(guard);
    assert!(sinks.interrupted().is_empty());
}

/// #2210: a request dropped mid-attempt (a deadline, an abort, a shutdown)
/// is published as `cancelled` with its attempt in flight recorded as
/// `Interrupted` — what it had streamed — and queued for its audit record.
#[test]
fn a_request_dropped_mid_attempt_records_the_attempt_as_interrupted() {
    let sinks = Sinks::default();
    let trace = Arc::new(RequestTrace::default());
    let guard = sinks.guard(&trace);
    let started = Instant::now();
    trace.begin_attempt(1, started, 1_000);
    trace.observe_event(started, 0, false);
    trace.observe_event(started, 12, true);
    drop(guard);
    assert_eq!(sinks.in_flight.snapshot(Instant::now()), None);
    let interrupted = sinks.interrupted();
    assert_eq!(interrupted.len(), 1);
    let record = &interrupted[0];
    assert_eq!(record.outcome, "cancelled");
    let attempt = record.attempt_diagnostics.last().unwrap();
    assert_eq!(attempt.termination, Termination::Interrupted);
    assert_eq!((attempt.event_count, attempt.output_bytes), (2, 12));
    assert_eq!(attempt.first_token_ms, Some(0));
    let logged = &sinks.log.lock().unwrap().recent;
    assert_eq!(logged.len(), 1);
    assert_eq!(logged[0], *record);
}

/// #2210: a request dropped between attempts has no attempt in flight to
/// invent; it is still queued for its audit record.
#[test]
fn a_request_dropped_between_attempts_records_only_its_ended_attempts() {
    let sinks = Sinks::default();
    let trace = Arc::new(RequestTrace::default());
    let guard = sinks.guard(&trace);
    trace.begin_attempt(1, Instant::now(), 1_000);
    trace.record_attempt(
        crate::domain::inference::value_objects::attempt_diagnostics::AttemptDiagnostics {
            attempt_number: 1,
            termination: Termination::Idle,
            ..Default::default()
        },
    );
    drop(guard);
    let interrupted = sinks.interrupted();
    assert_eq!(interrupted.len(), 1);
    let terminations: Vec<_> = interrupted[0]
        .attempt_diagnostics
        .iter()
        .map(|attempt| attempt.termination)
        .collect();
    assert_eq!(terminations, [Termination::Idle]);
}

/// #2210: a finished request never invents an interrupted attempt, even
/// when its transport has not recorded the attempt yet.
#[test]
fn a_finished_request_never_records_an_interrupted_attempt() {
    let sinks = Sinks::default();
    let trace = Arc::new(RequestTrace::default());
    let mut guard = sinks.guard(&trace);
    trace.begin_attempt(1, Instant::now(), 1_000);
    let record = guard.finish(&Err(DomainError::Provider("HTTP 500".into())));
    assert!(record.attempt_diagnostics.is_empty());
    drop(guard);
    assert!(sinks.interrupted().is_empty());
}

/// #2210: requests ended in flight wait for their audit record in a bounded
/// queue: a mode that never audits them keeps only the newest.
#[test]
fn the_interrupted_queue_keeps_only_the_newest_requests() {
    let sinks = Sinks::default();
    let mut ids = Vec::new();
    for _ in 0..(INTERRUPTED_RETAINED + 1) {
        let trace = Arc::new(RequestTrace::default());
        drop(sinks.guard(&trace));
        ids.push(sinks.interrupted().last().unwrap().request_id.clone());
    }
    let kept: Vec<_> = sinks
        .interrupted()
        .into_iter()
        .map(|record| record.request_id)
        .collect();
    assert_eq!(kept.len(), INTERRUPTED_RETAINED);
    assert_eq!(kept, ids[1..]);
}

/// #2210 review: a transport run inside the request (every `chat`) records
/// `Dropped` before the request's guard publishes; the dropped request
/// reads that attempt as `Interrupted`.
#[test]
fn a_request_dropped_after_its_transport_records_the_attempt_as_interrupted() {
    let sinks = Sinks::default();
    let trace = Arc::new(RequestTrace::default());
    let guard = sinks.guard(&trace);
    trace.begin_attempt(1, Instant::now(), 1_000);
    trace.mark_dropping();
    trace.record_attempt(
        crate::domain::inference::value_objects::attempt_diagnostics::AttemptDiagnostics {
            attempt_number: 1,
            event_count: 3,
            termination: Termination::Dropped,
            ..Default::default()
        },
    );
    drop(guard);
    let interrupted = sinks.interrupted();
    let attempt = &interrupted[0].attempt_diagnostics[0];
    assert_eq!(
        (attempt.termination, attempt.event_count),
        (Termination::Interrupted, 3)
    );
}

/// #2210 review: a request future dropped before it completes marks its
/// trace dropping before anything it owns is dropped; one that completed
/// does not.
#[test]
fn a_request_dropped_unfinished_is_marked_dropping_first() {
    struct Records(Arc<RequestTrace>);
    impl Drop for Records {
        fn drop(&mut self) {
            self.0.begin_attempt(1, Instant::now(), 1);
            self.0.record_attempt(
                crate::domain::inference::value_objects::attempt_diagnostics::AttemptDiagnostics {
                    attempt_number: 1,
                    termination: Termination::Dropped,
                    ..Default::default()
                },
            );
        }
    }
    let trace = Arc::new(RequestTrace::default());
    let owned = Records(trace.clone());
    let request = MarkDropping::new(trace.clone(), async move {
        let _owned = owned;
        std::future::pending::<()>().await;
    });
    let mut request = Box::pin(request);
    let waker = std::task::Waker::noop();
    assert!(
        request
            .as_mut()
            .poll(&mut Context::from_waker(waker))
            .is_pending()
    );
    drop(request);
    assert_eq!(
        trace.attempt_diagnostics()[0].termination,
        Termination::Interrupted
    );

    let finished = Arc::new(RequestTrace::default());
    let mut done = Box::pin(MarkDropping::new(finished.clone(), async {}));
    assert!(
        done.as_mut()
            .poll(&mut Context::from_waker(waker))
            .is_ready()
    );
    drop(done);
    finished.begin_attempt(1, Instant::now(), 1);
    finished.record_attempt(
        crate::domain::inference::value_objects::attempt_diagnostics::AttemptDiagnostics {
            attempt_number: 1,
            termination: Termination::Dropped,
            ..Default::default()
        },
    );
    assert_eq!(
        finished.attempt_diagnostics()[0].termination,
        Termination::Dropped
    );
}

/// #2398: the published observation carries where the request's input
/// first differed from its session's previous request, as its provider
/// recorded it on the trace, whether the request finished or was dropped.
#[test]
fn a_published_observation_carries_the_input_prefix_its_provider_recorded() {
    use crate::domain::inference::events::request_observation::{
        InputItemKind, InputPrefix, InputPrefixParts,
    };
    let prefix = InputPrefix::new(InputPrefixParts {
        input_items: 6,
        previous_items: Some(6),
        first_changed_item: Some(4),
        first_changed_kind: Some(InputItemKind::Reasoning),
        prefix_tokens_estimate: 321,
        unchanged_prefix_tokens_estimate: 400,
        request_tokens_estimate: 500,
    })
    .unwrap();
    let sinks = Sinks::default();
    let trace = Arc::new(RequestTrace::default());
    let mut guard = sinks.guard(&trace);
    trace.record_input_prefix(prefix);
    let record = guard.finish(&Err(DomainError::Other("stopped".into())));
    assert_eq!(record.input_prefix, Some(prefix));

    let trace = Arc::new(RequestTrace::default());
    let guard = sinks.guard(&trace);
    trace.record_input_prefix(prefix);
    drop(guard);
    assert_eq!(sinks.interrupted()[0].input_prefix, Some(prefix));

    let unobserved = Arc::new(RequestTrace::default());
    let mut guard = sinks.guard(&unobserved);
    let record = guard.finish(&Err(DomainError::Other("stopped".into())));
    assert_eq!(record.input_prefix, None);
}
