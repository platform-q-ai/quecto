//! Exercise the real owner, not a replica of permit accounting. Transport drop
//! is a synchronous destruction barrier; finish must occur strictly afterward.
use super::*;
use crate::infrastructure::providers::transport::wire_dialect::TEST_WIRE;

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
    fn acquire(
        &self,
    ) -> crate::application::admission::ports::inference_attempt::AttemptAcquisition<'_> {
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

/// #2155 review: an HTTP error declaring any billing name from the domain's
/// one list never throttles admission, even as a 429; a plain 429 does.
#[test]
fn a_billing_http_error_never_throttles_admission() {
    for name in crate::domain::inference::services::provider_error::BILLING_ERROR_NAMES {
        for field in ["type", "code"] {
            let receipt = diagnostic_receipt();
            let body = serde_json::json!({"error": { field: name }}).to_string();
            receipt.http_error(&TEST_WIRE, 429, &body);
            let state = receipt.0.lock().unwrap();
            assert!(!state.hinted, "{body}");
            assert!(state.failure, "{body}");
        }
    }
    let receipt = diagnostic_receipt();
    receipt.http_error(&TEST_WIRE, 429, r#"{"error":{"type":"throttled"}}"#);
    assert!(receipt.0.lock().unwrap().hinted);
    // #2609 review: a wire's terminal name never throttles, even as a 429.
    let receipt = diagnostic_receipt();
    receipt.http_error(&TEST_WIRE, 429, r#"{"error":{"type":"refused"}}"#);
    assert!(!receipt.0.lock().unwrap().hinted);
    // A name the wire does not know is no throttle beyond its status.
    let receipt = diagnostic_receipt();
    receipt.http_error(&TEST_WIRE, 500, r#"{"error":{"type":"made_up"}}"#);
    assert!(!receipt.0.lock().unwrap().hinted);
    let receipt = diagnostic_receipt();
    receipt.http_error(&TEST_WIRE, 500, r#"{"error":{"type":"busy"}}"#);
    assert!(receipt.0.lock().unwrap().hinted);
}
