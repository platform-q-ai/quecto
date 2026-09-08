//! Real loopback leaf scenarios. Fixtures are recording ports, not provider doubles.
//! Every Then uses a pure oracle also exercised with a valid observation and a
//! corrupt counterexample in `prove_oracles`; later Thens remain falsifiable when
//! an earlier RED setup/Then prevents cucumber from reaching them.
use super::*;
#[path = "../common/admission_attempt_fixture.rs"]
pub mod attempts;
#[path = "../common/admission_feedback_fixture.rs"]
pub mod feedback;
use quecto::domain::inference_admission::{Feedback, ThrottleFeedback};
use quecto::domain::provider::StreamEvent;
use quecto::infrastructure::providers::retry::{RetryConfig, RetryingProvider};
use tokio::sync::mpsc;

#[derive(Default)]
pub struct ProviderAdmissionState {
    runtime: Option<tokio::runtime::Runtime>,
    lifetime: Option<Lifetime>,
    receipt: Option<Receipt>,
}
impl std::fmt::Debug for ProviderAdmissionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<loopback provider admission scenario>")
    }
}
struct Lifetime {
    gate: Arc<attempts::Gate>,
    server: attempts::Server,
    provider: Arc<dyn LlmProvider>,
    first: Option<mpsc::Receiver<StreamEvent>>,
    connection: Option<attempts::Connection>,
    sibling: Option<attempts::Task<mpsc::Receiver<StreamEvent>>>,
}
struct Receipt {
    gate: Arc<feedback::Gate>,
    server: feedback::Server,
    provider: Arc<dyn LlmProvider>,
    connection: Option<feedback::Connection>,
    task: Option<feedback::Task<feedback::Transcript>>,
    output: Option<feedback::Transcript>,
    receiver: Option<mpsc::Receiver<StreamEvent>>,
}
fn runtime(w: &mut QuectoWorld) -> &mut ProviderAdmissionState {
    let s = &mut w.provider_admission;
    if s.runtime.is_none() {
        s.runtime = Some(
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap(),
        );
    }
    s
}

#[given("a phase-local provider admission group with one slot")]
fn one_slot(w: &mut QuectoWorld) {
    prove_oracles();
    runtime(w);
}
#[given("a fake provider response held open behind a transport barrier")]
fn held_transport(w: &mut QuectoWorld) {
    let s = runtime(w);
    s.lifetime = Some(s.runtime.as_ref().unwrap().block_on(async {
        let gate = attempts::Gate::new(false);
        let server = attempts::Server::start(
            attempts::Leaf::OpenAi,
            attempts::Surface::Incremental,
            1,
            attempts::Pause::Body,
            gate.clone(),
        )
        .await;
        let provider = attempts::Leaf::OpenAi.provider(&server.url, gate.clone());
        Lifetime {
            gate,
            server,
            provider,
            first: None,
            connection: None,
            sibling: None,
        }
    }));
}
#[when("an incremental inference receiver is returned")]
fn receiver(w: &mut QuectoWorld) {
    let s = runtime(w);
    let t = s.lifetime.as_mut().unwrap();
    s.runtime.as_ref().unwrap().block_on(async {
        let mut rx =
            attempts::bounded(t.provider.chat_stream_incremental(attempts::request())).await;
        t.connection = Some(t.server.connection().await);
        attempts::oracle::delta(attempts::bounded(rx.recv()).await, Some("0,"));
        t.first = Some(rx);
    });
}
#[when("another inference attempt queues in the same group")]
fn sibling(w: &mut QuectoWorld) {
    let s = runtime(w);
    let t = s.lifetime.as_mut().unwrap();
    s.runtime.as_ref().unwrap().block_on(async {
        let provider = t.provider.clone();
        t.sibling = Some(attempts::Task(tokio::spawn(async move {
            provider.chat_stream_incremental(attempts::request()).await
        })));
        // Positive queue barrier. The fixture detects an ungranted physical
        // request immediately; its deadline bounds failure, never proves absence.
        t.gate.wait_event(attempts::Event::Queued(1)).await;
    });
}
#[then("the second attempt has not started HTTP")]
fn no_second(w: &mut QuectoWorld) {
    let t = w.provider_admission.lifetime.as_ref().unwrap();
    oracle::held(&t.gate.events(), t.gate.active());
}
#[when("the first owned transport acknowledges completion")]
fn finish_first(w: &mut QuectoWorld) {
    let s = runtime(w);
    let t = s.lifetime.as_mut().unwrap();
    s.runtime.as_ref().unwrap().block_on(async {
        t.connection.take().unwrap().finish().await;
        let rx = t.first.as_mut().unwrap();
        attempts::oracle::done(attempts::bounded(rx.recv()).await, None);
        attempts::oracle::closed(attempts::bounded(rx.recv()).await);
        t.gate.wait_event(attempts::Event::Finished(0)).await;
        let connection = t.server.connection().await;
        let mut rx = attempts::bounded(&mut t.sibling.as_mut().unwrap().0)
            .await
            .unwrap();
        attempts::oracle::delta(attempts::bounded(rx.recv()).await, Some("0,"));
        connection.finish().await;
        attempts::oracle::done(attempts::bounded(rx.recv()).await, None);
        attempts::oracle::closed(attempts::bounded(rx.recv()).await);
        t.gate.wait_event(attempts::Event::Finished(1)).await;
    });
}
#[then("the second attempt starts exactly one HTTP request")]
fn exactly_second(w: &mut QuectoWorld) {
    oracle::replacement(
        &w.provider_admission
            .lifetime
            .as_ref()
            .unwrap()
            .gate
            .events(),
    );
}

#[given("a phase-local provider admission group with two slots")]
fn two_slots(w: &mut QuectoWorld) {
    prove_oracles();
    runtime(w);
}
#[given("a fake throttle response with a valid ninety second Retry-After")]
fn header_reply(w: &mut QuectoWorld) {
    let s = runtime(w);
    s.receipt = Some(s.runtime.as_ref().unwrap().block_on(async {
        let gate = Arc::new(feedback::Gate::default());
        let server = feedback::Server::start(feedback::Reply {
            status: 429,
            headers: "Retry-After: 90\r\n".into(),
            body: "billing is terminal".into(),
            stalled: true,
        })
        .await;
        let provider = feedback::Leaf::OpenAi.provider(&server.url, gate.clone());
        Receipt {
            gate,
            server,
            provider,
            connection: None,
            task: None,
            output: None,
            receiver: None,
        }
    }));
}
#[when("the throttle headers arrive while its body remains blocked")]
fn headers(w: &mut QuectoWorld) {
    let s = runtime(w);
    let r = s.receipt.as_mut().unwrap();
    s.runtime.as_ref().unwrap().block_on(async {
        let provider = r.provider.clone();
        r.task = Some(feedback::Task(tokio::spawn(async move {
            feedback::stream(provider.as_ref()).await
        })));
        r.connection = Some(r.server.connection().await);
        // Positive feedback receipt, with a bounded failure if the seam is absent.
        // No negative HTTP claim is inferred from this deadline.
        r.gate.observe_receipt().await;
    });
}
#[then("a sibling cannot start before the ninety second deadline")]
fn cooldown(w: &mut QuectoWorld) {
    let r = w.provider_admission.receipt.as_ref().unwrap();
    oracle::header(
        &r.gate.snapshot(),
        r.server.starts(),
        r.task.as_ref().unwrap().0.is_finished(),
    );
    // Exact clock arithmetic is checked through the real policy, not the
    // recording gate (which deliberately has no scheduler or clock).
    oracle::deadline_policy();
    let s = runtime(w);
    let r = s.receipt.as_ref().unwrap();
    s.runtime.as_ref().unwrap().block_on(async {
        let server = feedback::Server::start(feedback::Reply {
            status: 200,
            headers: "Content-Type: text/event-stream\r\n".into(),
            body: "data: [DONE]\n\n".into(),
            stalled: false,
        })
        .await;
        let provider = feedback::Leaf::OpenAi.provider(&server.url, r.gate.clone());
        let _task = feedback::Task(tokio::spawn(async move {
            feedback::stream(provider.as_ref()).await
        }));
        // Queue registration (or an actual forbidden POST) is the barrier.
        feedback::bounded(async {
            while r.gate.snapshot().queued < 2 && server.starts() == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await;
        oracle::blocked_sibling(r.gate.snapshot().queued, server.starts());
    });
}
#[then("an independent quota group can still start inference")]
fn independent(w: &mut QuectoWorld) {
    let s = runtime(w);
    s.runtime.as_ref().unwrap().block_on(async {
        let gate = Arc::new(feedback::Gate::default());
        let server = feedback::Server::start(feedback::Reply {
            status: 200,
            headers: "Content-Type: text/event-stream\r\n".into(),
            body: "data: [DONE]\n\n".into(),
            stalled: false,
        })
        .await;
        let provider = feedback::Leaf::OpenAi.provider(&server.url, gate.clone());
        let output = feedback::bounded(feedback::stream(provider.as_ref())).await;
        oracle::independent(&gate.snapshot(), server.starts(), &output);
    });
}
#[given("an admitted fake provider stream that has emitted text")]
fn text_stream(w: &mut QuectoWorld) {
    prove_oracles();
    let s = runtime(w);
    s.receipt = Some(s.runtime.as_ref().unwrap().block_on(async {
        let gate = Arc::new(feedback::Gate::default());
        let (body, _) =
            feedback::Leaf::Anthropic.sse(true, "overloaded_error", None, "opaque fixture");
        let mut server = feedback::Server::start(feedback::Reply {
            status: 200,
            headers: "Content-Type: text/event-stream\r\n".into(),
            body,
            stalled: false,
        })
        .await;
        let provider: Arc<dyn LlmProvider> = Arc::new(RetryingProvider::new(
            feedback::Leaf::Anthropic.provider(&server.url, gate.clone()),
            RetryConfig::no_delay(3),
        ));
        let mut rx = feedback::bounded(provider.chat_stream_incremental(feedback::request())).await;
        let connection = server.connection().await;
        attempts::oracle::delta(feedback::bounded(rx.recv()).await, Some("visible once"));
        Receipt {
            gate,
            server,
            provider,
            connection: Some(connection),
            task: None,
            output: Some(feedback::Transcript {
                text: vec!["visible once".into()],
                ..Default::default()
            }),
            receiver: Some(rx),
        }
    }));
}
#[when("a typed overload event ends the stream")]
fn overload(w: &mut QuectoWorld) {
    let s = runtime(w);
    let r = s.receipt.as_mut().unwrap();
    s.runtime.as_ref().unwrap().block_on(async {
        let rx = r.receiver.as_mut().unwrap();
        let output = r.output.as_mut().unwrap();
        while let Some(event) = feedback::bounded(rx.recv()).await {
            match event {
                StreamEvent::TextDelta(text) => output.text.push(text),
                StreamEvent::Error(error) => output.errors.push(error),
                StreamEvent::Done(_) => output.done += 1,
                other => panic!("unexpected fixture event {other:?}"),
            }
        }
    });
}
#[then("shared throttle feedback is recorded")]
fn shared_feedback(w: &mut QuectoWorld) {
    oracle::throttle(
        &w.provider_admission
            .receipt
            .as_ref()
            .unwrap()
            .gate
            .snapshot(),
    );
}
#[then("admission does not issue another HTTP attempt")]
fn no_replay(w: &mut QuectoWorld) {
    let r = w.provider_admission.receipt.as_ref().unwrap();
    oracle::no_replay(r.server.starts(), r.output.as_ref().unwrap());
}

mod oracle {
    use super::*;
    pub fn held(events: &[attempts::Event], active: usize) {
        use attempts::Event::*;
        attempts::oracle::trace(events, &[Queued(0), Granted(0), RawStart(0), Queued(1)]);
        attempts::oracle::active(active, 1);
    }
    pub fn replacement(events: &[attempts::Event]) {
        attempts::oracle::exact(events, 2);
        attempts::oracle::replacement_after_finish(events);
    }
    pub fn header(s: &feedback::Snapshot, starts: usize, completed: bool) {
        assert_eq!(starts, 1);
        assert_eq!(s.grants, 1, "header response must have been admitted");
        assert_eq!(s.receipts.len(), 1, "feedback must precede blocked body");
        assert!(matches!(s.receipts[0], ThrottleFeedback::Until(_)));
        assert!(s.finishes.is_empty());
        assert!(!completed);
    }
    pub fn blocked_sibling(queued: usize, starts: usize) {
        assert_eq!(queued, 2, "sibling reached acquire barrier");
        assert_eq!(starts, 0, "receipt blocks sibling with spare capacity");
    }
    pub fn independent(s: &feedback::Snapshot, starts: usize, out: &feedback::Transcript) {
        assert_eq!(starts, 1);
        assert_eq!(s.grants, 1);
        assert_eq!(out.done, 1);
        assert!(out.errors.is_empty());
    }
    pub fn throttle(s: &feedback::Snapshot) {
        assert_eq!(
            s.receipts.len(),
            1,
            "typed overload reports shared feedback"
        );
        assert!(matches!(s.receipts[0], ThrottleFeedback::NoHint { .. }));
        assert_eq!(s.grants, 1);
        assert_eq!(s.finishes, vec![Feedback::Failure]);
        assert_eq!(s.abandoned, 0);
    }
    pub fn no_replay(starts: usize, out: &feedback::Transcript) {
        assert_eq!(
            starts, 1,
            "one physical attempt even through real retry decorator"
        );
        assert_eq!(out.text, vec!["visible once"]);
        assert_eq!(
            out.errors,
            vec!["Anthropic stream error: type=overloaded_error: opaque fixture"]
        );
        assert_eq!(out.done, 0);
    }
    pub fn boundary(
        actual: Option<quecto::domain::inference_admission::RequestId>,
        expected: Option<quecto::domain::inference_admission::RequestId>,
    ) {
        assert_eq!(actual, expected, "exact authority cooldown boundary");
    }
    pub fn deadline_policy() {
        use quecto::application::inference_admission::{
            AdmissionClient, AdmissionDispatcher, AdmissionRegistry, AdmissionService,
        };
        use quecto::domain::inference_admission::*;
        let group = GroupId::new("fixture").unwrap();
        let mut service = AdmissionService::new(
            1,
            AdmissionConfig {
                groups: std::collections::BTreeMap::from([(
                    group.clone(),
                    GroupPolicy {
                        capacity: 2,
                        reserve: 0,
                        min_interval_ms: 1,
                        queue_capacity: 4,
                        queue_timeout_ms: 200_000,
                        attempt_timeout_ms: 200_000,
                        fallback_base_ms: 1000,
                        max_cooldown_ms: 100_000,
                    },
                )]),
                aliases: std::collections::BTreeMap::from([("fixture".into(), group.clone())]),
                max_scopes: 2,
                terminal_capacity: 4,
            },
        )
        .unwrap();
        let scope = service.register_root(WorkloadClass::Interactive).unwrap();
        service.enqueue(scope, 1, "fixture", 0).unwrap();
        assert!(service.next(&group, 0).unwrap().is_some());
        service
            .report_feedback(scope, 1, 1, ThrottleFeedback::Until(90_000), 0)
            .unwrap();
        service.enqueue(scope, 2, "fixture", 1).unwrap();
        boundary(service.next(&group, 89_999).unwrap(), None);
        boundary(
            service.next(&group, 90_000).unwrap(),
            Some(RequestId { scope, sequence: 2 }),
        );
    }
}

/// These are oracle tests, NOT fake provider executions. Both controls and
/// counterexamples go through the exact functions used by the Then definitions.
fn prove_oracles() {
    use attempts::Event::*;
    fn rejects(f: impl FnOnce() + std::panic::UnwindSafe) {
        assert!(
            std::panic::catch_unwind(f).is_err(),
            "Then oracle accepted corrupt observation"
        );
    }
    let held = vec![Queued(0), Granted(0), RawStart(0), Queued(1)];
    oracle::held(&held, 1);
    rejects(|| oracle::held(&held, 0));
    let mut bad = held.clone();
    bad.push(RawStart(1));
    rejects(|| oracle::held(&bad, 1));
    let completed = vec![
        Queued(0),
        Granted(0),
        RawStart(0),
        Queued(1),
        Finished(0),
        Granted(1),
        RawStart(1),
        Finished(1),
    ];
    oracle::replacement(&completed);
    let mut bad = completed.clone();
    bad.swap(4, 5);
    rejects(|| oracle::replacement(&bad));
    let mut bad = completed.clone();
    bad.push(RawStart(1));
    rejects(|| oracle::replacement(&bad));
    let mut header = feedback::Snapshot::default();
    header.grants = 1;
    header.receipts = vec![ThrottleFeedback::Until(90_000)];
    oracle::header(&header, 1, false);
    rejects(|| oracle::header(&header, 1, true));
    let mut absent = header.clone();
    absent.receipts.clear();
    rejects(|| oracle::header(&absent, 1, false));
    oracle::blocked_sibling(2, 0);
    rejects(|| oracle::blocked_sibling(2, 1));
    let success = feedback::Transcript {
        done: 1,
        ..Default::default()
    };
    oracle::independent(&header, 1, &success);
    rejects(|| oracle::independent(&header, 0, &success));
    let mut finished = header.clone();
    finished.receipts = vec![ThrottleFeedback::NoHint { jitter: 0 }];
    finished.finishes = vec![Feedback::Failure];
    oracle::throttle(&finished);
    rejects(|| oracle::throttle(&absent));
    let output = feedback::Transcript {
        text: vec!["visible once".into()],
        errors: vec!["Anthropic stream error: type=overloaded_error: opaque fixture".into()],
        done: 0,
    };
    oracle::no_replay(1, &output);
    rejects(|| oracle::no_replay(2, &output));
    // Per-member mutation inventory: no compound Then can pass merely because
    // its first assertion is sensitive. Every remaining observation is varied.
    for index in 0..held.len() {
        let mut bad = held.clone();
        bad.remove(index);
        rejects(|| oracle::held(&bad, 1));
    }
    let mut bad = held.clone();
    bad.swap(0, 1);
    rejects(|| oracle::held(&bad, 1));
    for index in [1, 2, 4, 5, 6] {
        let mut bad = completed.clone();
        bad.remove(index);
        rejects(|| oracle::replacement(&bad));
    }
    let mut bad = completed.clone();
    bad.swap(1, 2);
    rejects(|| oracle::replacement(&bad));
    let mut bad = completed.clone();
    bad.push(Abandoned(0));
    rejects(|| oracle::replacement(&bad));
    rejects(|| oracle::header(&header, 0, false));
    let mut bad = header.clone();
    bad.grants = 0;
    rejects(|| oracle::header(&bad, 1, false));
    let mut bad = header.clone();
    bad.receipts[0] = ThrottleFeedback::Unavailable;
    rejects(|| oracle::header(&bad, 1, false));
    let mut bad = header.clone();
    bad.finishes.push(Feedback::Failure);
    rejects(|| oracle::header(&bad, 1, false));
    rejects(|| oracle::blocked_sibling(1, 0));
    let mut bad = header.clone();
    bad.grants = 0;
    rejects(|| oracle::independent(&bad, 1, &success));
    let bad = feedback::Transcript {
        done: 0,
        ..Default::default()
    };
    rejects(|| oracle::independent(&header, 1, &bad));
    let bad = feedback::Transcript {
        done: 1,
        errors: vec!["unexpected".into()],
        ..Default::default()
    };
    rejects(|| oracle::independent(&header, 1, &bad));
    let mut bad = finished.clone();
    bad.receipts[0] = ThrottleFeedback::Unavailable;
    rejects(|| oracle::throttle(&bad));
    let mut bad = finished.clone();
    bad.receipts[0] = ThrottleFeedback::Until(90_000);
    rejects(|| oracle::throttle(&bad));
    let mut bad = finished.clone();
    bad.grants = 0;
    rejects(|| oracle::throttle(&bad));
    let mut bad = finished.clone();
    bad.finishes.clear();
    rejects(|| oracle::throttle(&bad));
    let mut bad = finished.clone();
    bad.finishes[0] = Feedback::Success;
    rejects(|| oracle::throttle(&bad));
    let mut bad = finished.clone();
    bad.abandoned = 1;
    rejects(|| oracle::throttle(&bad));
    let bad = feedback::Transcript {
        text: vec!["visible once".into(), "visible once".into()],
        errors: output.errors.clone(),
        done: 0,
    };
    rejects(|| oracle::no_replay(1, &bad));
    let bad = feedback::Transcript {
        text: output.text.clone(),
        errors: vec!["changed error".into()],
        done: 0,
    };
    rejects(|| oracle::no_replay(1, &bad));
    let bad = feedback::Transcript {
        text: output.text.clone(),
        errors: output.errors.clone(),
        done: 1,
    };
    rejects(|| oracle::no_replay(1, &bad));
    use quecto::domain::inference_admission::{RequestId, ScopeId};
    let id = RequestId {
        scope: ScopeId {
            epoch: 1,
            serial: 1,
        },
        sequence: 2,
    };
    oracle::boundary(None, None);
    oracle::boundary(Some(id), Some(id));
    rejects(|| oracle::boundary(Some(id), None));
    rejects(|| oracle::boundary(None, Some(id)));
}
