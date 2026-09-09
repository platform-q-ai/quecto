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
        let mut observer = ProtocolObserver::new(Profile::new(vendor, Surface::Assembled));
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
