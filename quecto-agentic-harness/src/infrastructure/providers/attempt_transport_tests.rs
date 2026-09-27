//! Exercise the real owner, not a replica of permit accounting. Transport drop
//! is a synchronous destruction barrier; finish must occur strictly afterward.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Event {
    TransportDestroyed,
    Finished(Feedback),
}
#[derive(Debug)]
struct Permit(Arc<Mutex<Vec<Event>>>);
impl AttemptPermit for Permit {
    fn feedback(&mut self, _: ThrottleFeedback) {}
    fn finish(self: Box<Self>, feedback: Feedback) {
        self.0.lock().unwrap().push(Event::Finished(feedback));
    }
}
struct Transport {
    events: Arc<Mutex<Vec<Event>>>,
    ready: bool,
}
impl Future for Transport {
    type Output = ();
    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
        if self.ready {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}
impl Drop for Transport {
    fn drop(&mut self) {
        self.events.lock().unwrap().push(Event::TransportDestroyed);
    }
}
fn observe(ready: bool) -> Vec<Event> {
    let events = Arc::new(Mutex::new(Vec::new()));
    let receipt = Receipt(Arc::new(Mutex::new(State {
        permit: Some(Box::new(Permit(events.clone()))),
        failure: false,
        hinted: false,
        trace: None,
        diagnostics: Default::default(),
        started: std::time::Instant::now(),
    })));
    let mut owner = OwnedTransport {
        operation: Some(Box::pin(Transport {
            events: events.clone(),
            ready,
        })),
        receipt,
        completed: false,
    };
    let mut cx = Context::from_waker(std::task::Waker::noop());
    assert_eq!(Pin::new(&mut owner).poll(&mut cx).is_ready(), ready);
    assert!(
        events.lock().unwrap().is_empty(),
        "poll completion is not destruction evidence"
    );
    drop(owner);
    Arc::try_unwrap(events).unwrap().into_inner().unwrap()
}
#[test]
fn unfinished_owner_destroys_transport_before_acknowledging_failure() {
    assert_eq!(
        observe(false),
        vec![
            Event::TransportDestroyed,
            Event::Finished(Feedback::Failure)
        ]
    );
}
#[test]
fn completed_owner_destroys_transport_before_acknowledging_success() {
    assert_eq!(
        observe(true),
        vec![
            Event::TransportDestroyed,
            Event::Finished(Feedback::Success)
        ]
    );
}

#[test]
fn protocol_dispatch_matches_each_vendors_terminal_vocabulary() {
    use super::super::attempt_profile::{Surface, Vendor};
    let cases = [
        (
            Vendor::OpenAi,
            "",
            r#"{"type":"response.completed"}"#,
            false,
            false,
        ),
        (
            Vendor::OpenAi,
            "",
            r#"{"choices":[{"finish_reason":"stop"}]}"#,
            false,
            false,
        ),
        (
            Vendor::OpenAi,
            "",
            r#"{"type":"error","code":"rate_limit_exceeded"}"#,
            false,
            false,
        ),
        (
            Vendor::OpenAi,
            "",
            r#"{"error":{"code":"rate_limit_exceeded"}}"#,
            true,
            true,
        ),
        // #2236: a bare string error (Ollama's and some OpenAI-compatible
        // servers' shape) ends the attempt as a failure too; an `error`
        // field of any other shape is not an error chunk.
        (
            Vendor::OpenAi,
            "",
            r#"{"error":"model crashed"}"#,
            true,
            true,
        ),
        (Vendor::OpenAi, "", r#"{"error":null}"#, false, false),
        (Vendor::OpenAi, "", r#"{"error":42}"#, false, false),
        (Vendor::OpenAi, "", r#"{"error":["x"]}"#, false, false),
        (
            Vendor::Codex,
            "",
            r#"{"type":"extension","error":{"code":"rate_limit_exceeded"}}"#,
            false,
            false,
        ),
        (
            Vendor::Codex,
            "",
            r#"{"type":"response.completed","response":{}}"#,
            true,
            false,
        ),
        (
            Vendor::Codex,
            "",
            r#"{"type":"error","code":"rate_limit_exceeded"}"#,
            true,
            true,
        ),
        (
            Vendor::Codex,
            "",
            r#"{"type":"response.failed","response":{"error":{}}}"#,
            true,
            true,
        ),
        (
            Vendor::Codex,
            "",
            r#"{"type":"response.incomplete","response":{}}"#,
            true,
            true,
        ),
        // #2249 review: an untyped error chunk fails a Responses attempt
        // too, `{}` included; an empty string `error`, or one on a typed
        // event, is no failure.
        (
            Vendor::Codex,
            "",
            r#"{"error":"model crashed"}"#,
            true,
            true,
        ),
        (
            Vendor::Codex,
            "",
            r#"{"error":{"message":"model crashed"}}"#,
            true,
            true,
        ),
        (
            Vendor::Codex,
            "",
            r#"{"type":null,"error":{"message":"model crashed"}}"#,
            true,
            true,
        ),
        (Vendor::Codex, "", r#"{"error":""}"#, false, false),
        (Vendor::Codex, "", r#"{"error":{}}"#, true, true),
        (
            Vendor::Codex,
            "",
            r#"{"type":"response.created","error":"x"}"#,
            false,
            false,
        ),
        (Vendor::OpenAi, "", r#"{"error":""}"#, false, false),
        (Vendor::OpenAi, "", r#"{"error":{}}"#, true, true),
        (
            Vendor::Anthropic,
            "ignored",
            r#"{"type":"error","error":{"type":"overloaded_error"}}"#,
            false,
            false,
        ),
        (Vendor::Anthropic, "message_stop", "{}", true, false),
        (
            Vendor::Anthropic,
            "error",
            r#"{"error":{"type":"overloaded_error"}}"#,
            true,
            true,
        ),
    ];
    for (vendor, event, data, terminal, failure) in cases {
        let events = Arc::new(Mutex::new(Vec::new()));
        let receipt = Receipt(Arc::new(Mutex::new(State {
            permit: Some(Box::new(Permit(events))),
            failure: false,
            hinted: false,
            trace: None,
            diagnostics: Default::default(),
            started: std::time::Instant::now(),
        })));
        let mut observer =
            ProtocolObserver::new(Profile::new(vendor, Surface::Assembled, Default::default()));
        observer.observe(&format!("event: {event}"), &receipt);
        observer.observe(&format!("data: {data}"), &receipt);
        assert_eq!(
            (observer.terminal, receipt.0.lock().unwrap().failure),
            (terminal, failure),
            "event={event} data={data}"
        );
    }
}

#[test]
fn supported_error_and_reasoning_events_retain_truthful_metadata() {
    let receipt = diagnostic_receipt();
    let mut openai = ProtocolObserver::new(Profile::new(
        Vendor::OpenAi,
        super::super::attempt_profile::Surface::Incremental,
        Default::default(),
    ));
    openai.observe(
        r#"data: {"error":{"code":"rate_limit_exceeded","message":"SECRET"}}"#,
        &receipt,
    );
    let d = &receipt.0.lock().unwrap().diagnostics;
    assert_eq!(d.terminal_event, Some(TerminalEvent::Error));
    assert_eq!(d.termination, Termination::Completed);
}

fn diagnostic_receipt() -> Receipt {
    Receipt(Arc::new(Mutex::new(State {
        permit: Some(Box::new(Permit(Arc::new(Mutex::new(Vec::new()))))),
        failure: false,
        hinted: false,
        trace: None,
        diagnostics: Default::default(),
        started: std::time::Instant::now(),
    })))
}

#[test]
fn supported_dotted_reasoning_remains_visible_on_read_failure() {
    let receipt = diagnostic_receipt();
    let mut protocol = ProtocolObserver::new(Profile::new(
        Vendor::Codex,
        super::super::attempt_profile::Surface::Incremental,
        Default::default(),
    ));
    protocol.observe(
        r#"data: {"type":"response.reasoning.summary_text.delta","delta":"SECRET"}"#,
        &receipt,
    );
    receipt.termination(Termination::ReadError);
    let d = &receipt.0.lock().unwrap().diagnostics;
    assert!(d.generated_thinking);
    assert_eq!(d.unknown_events, 0);
    assert_eq!(d.termination, Termination::ReadError);
    assert!(!serde_json::to_string(d).unwrap().contains("SECRET"));
}

#[test]
fn malformed_anthropic_terminal_retains_event_without_payload_content() {
    for (event, terminal) in [
        ("error", TerminalEvent::Error),
        ("message_stop", TerminalEvent::MessageStop),
    ] {
        let receipt = diagnostic_receipt();
        let mut protocol = ProtocolObserver::new(Profile::new(
            Vendor::Anthropic,
            super::super::attempt_profile::Surface::Incremental,
            Default::default(),
        ));
        protocol.observe(&format!("event: {event}"), &receipt);
        protocol.observe("data: {SECRET", &receipt);
        let d = &receipt.0.lock().unwrap().diagnostics;
        assert_eq!(d.terminal_event, Some(terminal));
        assert_eq!(d.termination, Termination::Completed);
        assert_eq!(d.parse_errors, 1);
        assert!(!serde_json::to_string(d).unwrap().contains("SECRET"));
    }
}

/// #2158: a recorded Codex tool-call stream counts no unknown events; an
/// event nobody knows still counts.
#[test]
fn a_codex_tool_call_stream_has_no_unknown_events() {
    let receipt = diagnostic_receipt();
    let mut protocol = ProtocolObserver::new(Profile::new(
        Vendor::Codex,
        super::super::attempt_profile::Surface::Incremental,
        Default::default(),
    ));
    for line in [
        r#"data: {"type":"response.created","response":{}}"#,
        r#"data: {"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","call_id":"c1","name":"read","arguments":""}}"#,
        r#"data: {"type":"response.function_call_arguments.delta","output_index":0,"delta":"{\"path\""}"#,
        r#"data: {"type":"response.function_call_arguments.delta","output_index":0,"delta":":\"a\"}"}"#,
        r#"data: {"type":"response.function_call_arguments.done","output_index":0,"arguments":"{\"path\":\"a\"}"}"#,
        r#"data: {"type":"response.output_item.done","output_index":0,"item":{"type":"function_call"}}"#,
    ] {
        protocol.observe(line, &receipt);
    }
    assert_eq!(receipt.0.lock().unwrap().diagnostics.unknown_events, 0);
    // Before the terminal event, after which nothing is counted.
    protocol.observe(r#"data: {"type":"response.never_heard_of"}"#, &receipt);
    protocol.observe(
        r#"data: {"type":"response.completed","response":{"status":"completed"}}"#,
        &receipt,
    );
    assert_eq!(receipt.0.lock().unwrap().diagnostics.unknown_events, 1);
}

/// A waker that counts how often it is woken.
#[derive(Default)]
struct Wakes(std::sync::atomic::AtomicUsize);
impl std::task::Wake for Wakes {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// #2155: a wait for cancellation registers no timer. With nothing else
/// happening, time passing wakes it never; the cancel alone wakes it, once.
#[tokio::test(start_paused = true)]
async fn a_cancel_wait_is_woken_by_the_cancel_never_by_polling() {
    use std::sync::atomic::Ordering;
    let flag = CancelFlag::new();
    let wakes = Arc::new(Wakes::default());
    let waker = std::task::Waker::from(wakes.clone());
    let mut cx = Context::from_waker(&waker);
    let mut wait = Box::pin(cancelled(Some(&flag)));
    assert!(wait.as_mut().poll(&mut cx).is_pending());
    for _ in 0..10 {
        tokio::time::advance(std::time::Duration::from_secs(1)).await;
    }
    assert_eq!(
        wakes.0.load(Ordering::SeqCst),
        0,
        "no wake without a cancel"
    );
    flag.cancel();
    assert_eq!(wakes.0.load(Ordering::SeqCst), 1);
    assert!(wait.as_mut().poll(&mut cx).is_ready());
    // Without a flag the wait never ends, and never wakes either.
    let mut never = Box::pin(cancelled(None));
    assert!(never.as_mut().poll(&mut cx).is_pending());
    tokio::time::advance(std::time::Duration::from_secs(10)).await;
    assert_eq!(wakes.0.load(Ordering::SeqCst), 1);
}

/// A gate that grants `permit` on acquire, or never grants when it is none.
#[derive(Debug)]
struct Grant(Mutex<Option<Box<dyn AttemptPermit>>>);
impl AttemptAdmission for Grant {
    fn acquire(&self) -> crate::application::ports::AttemptAcquisition<'_> {
        let permit = self.0.lock().unwrap().take();
        Box::pin(async move {
            match permit {
                Some(permit) => Ok(permit),
                None => std::future::pending().await,
            }
        })
    }
}

/// #2155: a cancel ends an attempt at once, with no other activity: waiting
/// in the gate's queue or in flight, in no virtual time at all (a polling
/// wait would take its poll interval), and an in-flight attempt still
/// finishes its permit as a failure.
#[tokio::test(start_paused = true)]
async fn a_cancel_stops_a_queued_or_running_attempt_in_no_time() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let gates: [Arc<dyn AttemptAdmission>; 2] = [
        Arc::new(Grant(Mutex::new(None))),
        Arc::new(Grant(Mutex::new(Some(Box::new(Permit(events.clone())))))),
    ];
    for gate in gates {
        let flag = CancelFlag::new();
        let canceller = flag.clone();
        tokio::spawn(async move {
            tokio::task::yield_now().await;
            canceller.cancel();
        });
        let started = tokio::time::Instant::now();
        // Bounded, so a wait the cancel never wakes fails (the paused clock
        // reaches the bound at once) instead of hanging.
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(3600),
            run(&gate, None, Some(&flag), None, |_| {
                std::future::pending::<Result<(), DomainError>>()
            }),
        )
        .await
        .expect("the cancel ends the attempt");
        assert!(matches!(result, Err(AttemptError::Stopped)));
        assert_eq!(started.elapsed(), std::time::Duration::ZERO);
    }
    assert_eq!(
        *events.lock().unwrap(),
        vec![Event::Finished(Feedback::Failure)]
    );
}

/// #2155 review: an OpenAI-compatible error chunk with a numeric 429 or 529
/// (OpenRouter's shape) throttles admission like a typed one; a billing
/// error, a server status or another vendor's numeric code does not.
#[test]
fn numeric_throttle_chunks_throttle_admission() {
    use super::super::attempt_profile::{Surface, Vendor};
    for (vendor, data, throttled) in [
        (
            Vendor::OpenAi,
            r#"{"error":{"code":429,"message":"m"}}"#,
            true,
        ),
        (Vendor::OpenAi, r#"{"error":{"code":529}}"#, true),
        (
            Vendor::OpenAi,
            r#"{"error":{"type":"rate_limit_error"}}"#,
            true,
        ),
        (
            Vendor::OpenAi,
            r#"{"error":{"code":500,"type":"rate_limit_error"}}"#,
            true,
        ),
        (
            Vendor::OpenAi,
            r#"{"error":{"code":429,"type":"insufficient_quota"}}"#,
            false,
        ),
        (
            Vendor::OpenAi,
            r#"{"error":{"code":529,"type":"billing_error"}}"#,
            false,
        ),
        (Vendor::OpenAi, r#"{"error":{"code":502}}"#, false),
        (
            Vendor::OpenAi,
            r#"{"error":{"type":"server_error"}}"#,
            false,
        ),
        (
            Vendor::Codex,
            r#"{"type":"error","error":{"code":429}}"#,
            false,
        ),
    ] {
        let receipt = diagnostic_receipt();
        let mut observer = ProtocolObserver::new(Profile::new(
            vendor,
            Surface::Incremental,
            Default::default(),
        ));
        observer.observe(&format!("data: {data}"), &receipt);
        let state = receipt.0.lock().unwrap();
        assert_eq!(state.hinted, throttled, "{data}");
        assert!(
            state.failure,
            "an error chunk always fails the attempt: {data}"
        );
    }
}

/// #2155 review: an HTTP error declaring any billing name from the domain's
/// one list never throttles admission, even as a 429; a plain 429 does.
#[test]
fn a_billing_http_error_never_throttles_admission() {
    for name in crate::domain::provider_error::BILLING_ERROR_NAMES {
        for field in ["type", "code"] {
            let receipt = diagnostic_receipt();
            let body = serde_json::json!({"error": { field: name }}).to_string();
            receipt.http_error(429, &body);
            let state = receipt.0.lock().unwrap();
            assert!(!state.hinted, "{body}");
            assert!(state.failure, "{body}");
        }
    }
    let receipt = diagnostic_receipt();
    receipt.http_error(429, r#"{"error":{"type":"rate_limit_error"}}"#);
    assert!(receipt.0.lock().unwrap().hinted);
}

/// #2236: through admission, a mid-stream error chunk of either shape ends
/// the stream as an error after the partial text, never as a `Done` reply,
/// and the end of file after it cannot turn the partial text into a whole
/// answer.
#[tokio::test]
async fn an_error_chunk_of_either_shape_ends_an_admitted_stream_as_an_error() {
    use super::super::attempt_profile::Surface;
    let text = r#"data: {"choices":[{"index":0,"delta":{"content":"partial"}}]}"#;
    for (error, status) in [
        (r#"{"error":"model crashed"}"#, "HTTP 502 "),
        (r#"{"error":{"message":"model crashed"}}"#, "HTTP 502 "),
        (r#"{"error":{"type":"invalid_request_error"}}"#, "HTTP 400 "),
    ] {
        let receipt = diagnostic_receipt();
        let profile = Profile::new(Vendor::OpenAi, Surface::Incremental, Default::default());
        let mut handler = ObservedHandler {
            inner: super::super::openai::openai_sse::OpenAiSseHandler::with_model("m"),
            receipt: receipt.clone(),
            protocol: ProtocolObserver::new(profile),
        };
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        assert!(matches!(
            handler.process_line(text, &tx).await,
            SseLineOutcome::Continue
        ));
        assert!(matches!(
            handler.process_line(&format!("data: {error}"), &tx).await,
            SseLineOutcome::Done
        ));
        drop(tx);
        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            events.push(event);
        }
        assert!(
            matches!(&events[..], [StreamEvent::TextDelta(t), StreamEvent::Error(e)]
                if t == "partial" && e.starts_with(status)),
            "{error}: {events:?}"
        );
        let state = receipt.0.lock().unwrap();
        assert_eq!(state.diagnostics.terminal_event, Some(TerminalEvent::Error));
    }
}
