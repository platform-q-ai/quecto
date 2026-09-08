//! P2 AC5 executable RED oracle: real unproxied loopback HTTP/SSE only.
//! No credentials, paid requests, production changes, or replacement policy.
#[path = "common/admission_feedback_fixture.rs"]
mod fixture;

use fixture::*;
use quecto::domain::inference_admission::{Feedback, ThrottleFeedback};
use quecto::infrastructure::providers::retry::{RetryConfig, RetryingProvider};
use std::sync::Arc;

async fn header_receipt(leaf: Leaf, status: u16) {
    let gate = Arc::new(Gate::default());
    let body = r#"{"error":{"type":"invalid_request_error","code":"insufficient_quota","message":"billing is terminal"}}"#;
    let mut server = Server::start(Reply {
        status,
        headers: "Content-Type: application/json\r\nRetry-After: 30\r\n".into(),
        body: body.into(),
        stalled: true,
    })
    .await;
    let provider = leaf.provider(&server.url, gate.clone());
    let task = Task(tokio::spawn(async move { stream(provider.as_ref()).await }));
    let connection = server.connection().await;
    gate.observe_receipt().await;
    let before_body = gate.snapshot();
    let completed_before_body = task.0.is_finished();
    connection.release();
    let mut task = task;
    let output = bounded(&mut task.0).await.unwrap();
    let after_body = gate.snapshot();
    let expected_error = match leaf {
        Leaf::OpenAi => format!("HTTP {status} from OpenAI: {body} retry-after: 30"),
        Leaf::Responses | Leaf::OAuth => format!("HTTP {status} from Codex: {body}"),
        Leaf::Anthropic => format!("HTTP {status} from Anthropic: {body} retry-after: 30"),
    };
    verify(header_oracle(
        server.starts(),
        completed_before_body,
        &output,
        &expected_error,
        &before_body,
        &after_body,
    ));
}
macro_rules! header_case {
    ($name:ident, $leaf:ident, $status:expr) => {
        #[tokio::test]
        async fn $name() {
            header_receipt(Leaf::$leaf, $status).await;
        }
    };
}
header_case!(openai_429_receipt_precedes_stalled_body, OpenAi, 429);
header_case!(responses_429_receipt_precedes_stalled_body, Responses, 429);
header_case!(oauth_429_receipt_precedes_stalled_body, OAuth, 429);
header_case!(anthropic_529_receipt_precedes_stalled_body, Anthropic, 529);

#[tokio::test]
async fn receipt_blocks_same_group_with_spare_capacity_but_not_unrelated_group() {
    let shared = Arc::new(Gate::default());
    let other = Arc::new(Gate::default());
    let mut stalled = Server::start(Reply {
        status: 429,
        headers: "Retry-After: 30\r\n".into(),
        body: "billing is terminal".into(),
        stalled: true,
    })
    .await;
    let provider = Leaf::OpenAi.provider(&stalled.url, shared.clone());
    let first = Task(tokio::spawn(async move { stream(provider.as_ref()).await }));
    let connection = stalled.connection().await;
    shared.observe_receipt().await;
    let success = || Reply {
        status: 200,
        headers: "Content-Type: text/event-stream\r\n".into(),
        body: "data: [DONE]\n\n".into(),
        stalled: false,
    };
    let mut sibling_server = Server::start(success()).await;
    let sibling = Leaf::OpenAi.provider(&sibling_server.url, shared.clone());
    let mut sibling_task = Task(tokio::spawn(async move { stream(sibling.as_ref()).await }));
    let unrelated_server = Server::start(success()).await;
    let unrelated = Leaf::OpenAi.provider(&unrelated_server.url, other.clone());
    let unrelated_output = bounded(stream(unrelated.as_ref())).await;
    // A positive synchronization barrier: either admission has parked the
    // sibling behind receipt feedback, or the server observes a bypass POST.
    // No quiet interval/negative wall-clock observation is admission evidence.
    tokio::select! {
        _ = shared.blocked_sibling() => {},
        connection = sibling_server.connection() => { connection.release(); },
    }
    let sibling_sends_during_cooldown = sibling_server.starts();
    let before_release = shared.snapshot();
    connection.release();
    let mut first = first;
    let _ = bounded(&mut first.0).await.unwrap();
    shared.reopen();
    let _ = bounded(&mut sibling_task.0).await.unwrap();
    verify(group_oracle(
        GroupSends {
            first: stalled.starts(),
            other: unrelated_server.starts(),
            early: sibling_sends_during_cooldown,
            sibling: sibling_server.starts(),
        },
        &unrelated_output,
        &before_release,
        &shared.snapshot(),
        &other.snapshot(),
    ));
}

async fn typed_sse(
    leaf: Leaf,
    post_text: bool,
    kind: &str,
    code: Option<&str>,
    message: &str,
    throttle: bool,
) {
    let gate = Arc::new(Gate::default());
    let (body, error) = leaf.sse(post_text, kind, code, message);
    let server = Server::start(Reply {
        status: 200,
        headers: "Content-Type: text/event-stream\r\n".into(),
        body,
        stalled: false,
    })
    .await;
    // Real existing retry decorator: even a pre-text SSE error is not a license
    // for this decorator/leaf to replay a stream. No sleep and at most 3 attempts
    // keeps an accidental replay observable and bounded.
    let provider = RetryingProvider::new(
        leaf.provider(&server.url, gate.clone()),
        RetryConfig::no_delay(3),
    );
    let output = bounded(stream(&provider)).await;
    verify(sse_oracle(
        server.starts(),
        &output,
        post_text,
        &error,
        throttle,
        &gate.snapshot(),
    ));
}
macro_rules! sse_case {
    ($name:ident, $leaf:ident, $post:expr, $kind:expr, $code:expr, $message:expr, $throttle:expr) => {
        #[tokio::test]
        async fn $name() {
            typed_sse(Leaf::$leaf, $post, $kind, $code, $message, $throttle).await;
        }
    };
}
sse_case!(
    responses_typed_throttle_before_text,
    Responses,
    false,
    "rate_limit_error",
    None,
    "opaque fixture",
    true
);
sse_case!(
    responses_typed_throttle_after_text,
    Responses,
    true,
    "rate_limit_error",
    None,
    "opaque fixture",
    true
);
sse_case!(
    oauth_typed_throttle_before_text,
    OAuth,
    false,
    "rate_limit_error",
    None,
    "opaque fixture",
    true
);
sse_case!(
    oauth_typed_throttle_after_text,
    OAuth,
    true,
    "rate_limit_error",
    None,
    "opaque fixture",
    true
);
sse_case!(
    anthropic_typed_throttle_before_text,
    Anthropic,
    false,
    "overloaded_error",
    None,
    "opaque fixture",
    true
);
sse_case!(
    anthropic_typed_throttle_after_text,
    Anthropic,
    true,
    "overloaded_error",
    None,
    "opaque fixture",
    true
);
sse_case!(
    responses_billing_overrides_throttle_type,
    Responses,
    false,
    "rate_limit_error",
    Some("insufficient_quota"),
    "opaque fixture",
    false
);
sse_case!(
    anthropic_billing_overrides_throttle_type,
    Anthropic,
    true,
    "rate_limit_error",
    Some("insufficient_quota"),
    "opaque fixture",
    false
);
sse_case!(
    responses_prose_cannot_establish_cooldown,
    Responses,
    true,
    "invalid_request_error",
    None,
    "429 rate_limit_error overloaded_error retry-after: 30",
    false
);
sse_case!(
    anthropic_prose_cannot_establish_cooldown,
    Anthropic,
    false,
    "authentication_error",
    None,
    "429 rate_limit_error overloaded_error retry-after: 30",
    false
);

// Pure acceptance oracles are shared by real transport tests and independent
// counterexamples. Evaluate every condition before asserting, so one RED cannot
// hide later observations. Stable IDs are the evidence/report join keys.
type Checks = Vec<(&'static str, bool)>;
fn verify(checks: Checks) {
    for (id, passed) in &checks {
        eprintln!("{id}: {}", if *passed { "PASS" } else { "RED" });
    }
    let failures: Vec<_> = checks
        .iter()
        .filter(|(_, ok)| !ok)
        .map(|(id, _)| *id)
        .collect();
    assert!(
        failures.is_empty(),
        "acceptance oracle failures: {failures:?}"
    );
}
fn header_oracle(
    sends: usize,
    early: bool,
    out: &Transcript,
    error: &str,
    before: &Snapshot,
    after: &Snapshot,
) -> Checks {
    vec![
        ("H01", sends == 1),
        ("H02", !early),
        ("H03", out.errors.len() == 1),
        ("H04", out.errors == [error]),
        ("H05", out.text.is_empty()),
        ("H06", out.done == 0),
        ("H07", before.receipts.len() == 1),
        (
            "H08",
            matches!(before.receipts.first(), Some(ThrottleFeedback::Until(_))),
        ),
        ("H09", before.grants == 1),
        ("H10", before.finishes.is_empty()),
        ("H11", after.receipts == before.receipts),
        ("H12", after.finishes == [Feedback::Failure]),
        ("H13", after.abandoned == 0),
    ]
}
/// Physical send counts at the two group-isolation observation barriers.
struct GroupSends {
    first: usize,
    other: usize,
    early: usize,
    sibling: usize,
}

fn group_oracle(
    sends: GroupSends,
    out: &Transcript,
    before: &Snapshot,
    after: &Snapshot,
    unrelated: &Snapshot,
) -> Checks {
    let GroupSends {
        first,
        other,
        early,
        sibling,
    } = sends;
    vec![
        ("G01", first == 1),
        ("G02", other == 1),
        ("G03", out.done == 1),
        ("G04", out.errors.is_empty()),
        ("G05", early == 0),
        ("G06", before.queued == 2),
        ("G07", before.grants == 1),
        ("G08", before.finishes.is_empty()),
        ("G09", sibling == 1),
        ("G10", after.grants == 2),
        ("G11", unrelated.grants == 1),
        ("G12", before.parked > 0),
    ]
}
fn sse_oracle(
    sends: usize,
    out: &Transcript,
    post: bool,
    error: &str,
    throttle: bool,
    state: &Snapshot,
) -> Checks {
    vec![
        ("S01", sends == 1),
        (
            "S02",
            out.text == if post { vec!["visible once"] } else { vec![] },
        ),
        ("S03", out.errors == [error]),
        ("S04", out.done == 0),
        ("S05", state.receipts.len() == usize::from(throttle)),
        (
            "S06",
            !throttle
                || matches!(
                    state.receipts.first(),
                    Some(ThrottleFeedback::NoHint { .. })
                ),
        ),
        ("S07", state.grants == 1),
        ("S08", state.finishes.len() == 1),
        ("S09", state.finishes == [Feedback::Failure]),
        ("S10", state.abandoned == 0),
    ]
}

#[test]
fn every_acceptance_assertion_rejects_its_synthetic_counterexample() {
    // Counterexamples alter observed facts, never expected values or production.
    // Each ID must be true for a valid observation, false for its own mutation,
    // and the exact production-test assertion entrypoint must panic for that ID.
    fn falsify(good: Checks, bad: Checks, id: &str) {
        assert!(good.iter().all(|(_, ok)| *ok), "valid fixture for {id}");
        let condition = bad.into_iter().find(|(key, _)| *key == id).unwrap();
        assert!(!condition.1, "counterexample did not falsify {id}");
        assert!(
            std::panic::catch_unwind(|| verify(vec![condition])).is_err(),
            "assertion did not reject {id}"
        );
        eprintln!("FALSIFIED {id}");
    }
    let mut before = Snapshot::default();
    before.grants = 1;
    before.receipts = vec![ThrottleFeedback::Until(30_000)];
    let mut after = before.clone();
    after.finishes = vec![Feedback::Failure];
    for n in 1..=13 {
        let good_out = Transcript {
            text: vec![],
            errors: vec!["original".into()],
            done: 0,
        };
        let mut out = Transcript {
            text: vec![],
            errors: vec!["original".into()],
            done: 0,
        };
        let mut b = before.clone();
        let mut a = after.clone();
        let mut sends = 1;
        let mut early = false;
        match n {
            1 => sends = 2,
            2 => early = true,
            3 => out.errors.push("duplicate".into()),
            4 => out.errors[0] = "rewritten".into(),
            5 => out.text.push("unexpected".into()),
            6 => out.done = 1,
            7 => b.receipts.push(ThrottleFeedback::Until(30_000)),
            8 => b.receipts[0] = ThrottleFeedback::Unavailable,
            9 => b.grants = 0,
            10 => b.finishes.push(Feedback::Failure),
            11 => a.receipts = vec![ThrottleFeedback::Until(60_000)],
            12 => a.finishes = vec![Feedback::Throttle { delay_ms: 30_000 }],
            13 => a.abandoned = 1,
            _ => unreachable!(),
        }
        falsify(
            header_oracle(1, false, &good_out, "original", &before, &after),
            header_oracle(sends, early, &out, "original", &b, &a),
            &format!("H{n:02}"),
        );
    }
    before.queued = 2;
    before.parked = 1;
    let mut shared = Snapshot::default();
    shared.grants = 2;
    let mut other = Snapshot::default();
    other.grants = 1;
    for n in 1..=12 {
        let good_out = Transcript {
            done: 1,
            ..Transcript::default()
        };
        let mut out = Transcript {
            done: 1,
            ..Transcript::default()
        };
        let mut b = before.clone();
        let mut a = shared.clone();
        let mut o = other.clone();
        let (mut first, mut unrelated, mut early, mut sibling) = (1, 1, 0, 1);
        match n {
            1 => first = 2,
            2 => unrelated = 0,
            3 => out.done = 0,
            4 => out.errors.push("error".into()),
            5 => early = 1,
            6 => b.queued = 1,
            7 => b.grants = 2,
            8 => b.finishes.push(Feedback::Failure),
            9 => sibling = 2,
            10 => a.grants = 1,
            11 => o.grants = 0,
            12 => b.parked = 0,
            _ => unreachable!(),
        }
        falsify(
            group_oracle(
                GroupSends {
                    first: 1,
                    other: 1,
                    early: 0,
                    sibling: 1,
                },
                &good_out,
                &before,
                &shared,
                &other,
            ),
            group_oracle(
                GroupSends {
                    first,
                    other: unrelated,
                    early,
                    sibling,
                },
                &out,
                &b,
                &a,
                &o,
            ),
            &format!("G{n:02}"),
        );
    }
    for post in [false, true] {
        for throttle in [false, true] {
            for n in 1..=10 {
                if n == 6 && !throttle {
                    continue;
                } // guarded implication; no receipt required for terminal errors
                let mut state = after.clone();
                state.receipts = vec![ThrottleFeedback::NoHint { jitter: u64::MAX }];
                if !throttle {
                    state.receipts.clear();
                }
                let good_out = Transcript {
                    text: if post {
                        vec!["visible once".into()]
                    } else {
                        vec![]
                    },
                    errors: vec!["original".into()],
                    done: 0,
                };
                let mut out = Transcript {
                    text: good_out.text.clone(),
                    errors: good_out.errors.clone(),
                    done: 0,
                };
                let mut s = state.clone();
                let mut sends = 1;
                match n {
                    1 => sends = 2,
                    2 => out.text.push("replayed".into()),
                    3 => out.errors[0] = "rewritten".into(),
                    4 => out.done = 1,
                    5 => s.receipts.push(ThrottleFeedback::Until(42)),
                    6 => s.receipts[0] = ThrottleFeedback::Until(30_000),
                    7 => s.grants = 0,
                    8 => s.finishes.push(Feedback::Failure),
                    9 => s.finishes = vec![Feedback::Throttle { delay_ms: 30_000 }],
                    10 => s.abandoned = 1,
                    _ => unreachable!(),
                }
                falsify(
                    sse_oracle(1, &good_out, post, "original", throttle, &state),
                    sse_oracle(sends, &out, post, "original", throttle, &s),
                    &format!("S{n:02}"),
                );
            }
        }
    }
}
