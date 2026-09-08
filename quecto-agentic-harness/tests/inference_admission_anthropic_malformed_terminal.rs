//! Anthropic terminal dispatch depends on event name even when data is not JSON.
#[path = "common/admission_feedback_fixture.rs"]
pub mod fixture;
use fixture::*;
use quecto::domain::{
    error::DomainError,
    inference_admission::{Feedback, ThrottleFeedback},
};
use std::sync::Arc;

const ERROR: &str = "Anthropic stream error: type=error: Anthropic stream error";
fn checks(sends: usize, output: &Transcript, state: &Snapshot, error: bool) -> [bool; 7] {
    [
        sends == 1,
        output.done == usize::from(!error),
        output.errors == if error { vec![ERROR] } else { vec![] },
        state.receipts.is_empty(),
        state.finishes
            == [if error {
                Feedback::Failure
            } else {
                Feedback::Success
            }],
        state.grants == 1,
        state.abandoned == 0,
    ]
}
fn verify(checks: [bool; 7]) {
    assert!(
        checks.into_iter().all(|ok| ok),
        "malformed terminal acceptance: {checks:?}"
    );
}
async fn check(incremental: bool, error: bool) {
    let event = if error { "error" } else { "message_stop" };
    let data = if error { "{not-json" } else { "[DONE]" };
    let server = Server::start(Reply { status: 200, headers: "Content-Type: text/event-stream\r\n".into(),
        body: format!("event: {event}\ndata: {data}\n\nevent: error\ndata: {{\"type\":\"error\",\"error\":{{\"type\":\"overloaded_error\",\"message\":\"must be ignored\"}}}}\n\n"), stalled: false }).await;
    let gate = Arc::new(Gate::default());
    let provider = Leaf::Anthropic.provider(&server.url, gate.clone());
    let output = if incremental {
        bounded(stream(provider.as_ref())).await
    } else {
        match bounded(provider.chat_stream(request())).await {
            Ok(_) => Transcript {
                done: 1,
                ..Default::default()
            },
            Err(DomainError::Provider(error)) => Transcript {
                errors: vec![error],
                ..Default::default()
            },
            Err(error) => panic!("unexpected non-provider error: {error}"),
        }
    };
    verify(checks(server.starts(), &output, &gate.snapshot(), error));
}
#[tokio::test]
async fn assembled_malformed_stop_ignores_later_throttle() {
    check(false, false).await;
}
#[tokio::test]
async fn assembled_malformed_error_ignores_later_throttle() {
    check(false, true).await;
}
#[tokio::test]
async fn incremental_malformed_stop_ignores_later_throttle() {
    check(true, false).await;
}
#[tokio::test]
async fn incremental_malformed_error_ignores_later_throttle() {
    check(true, true).await;
}
#[test]
fn every_acceptance_predicate_rejects_its_counterexample() {
    for error in [false, true] {
        let output = || Transcript {
            done: usize::from(!error),
            errors: if error { vec![ERROR.into()] } else { vec![] },
            ..Default::default()
        };
        let state = || {
            let mut state = Snapshot::default();
            state.grants = 1;
            state.finishes = vec![if error {
                Feedback::Failure
            } else {
                Feedback::Success
            }];
            state
        };
        verify(checks(1, &output(), &state(), error));
        for index in 0..7 {
            let mut sends = 1;
            let mut out = output();
            let mut state = state();
            match index {
                0 => sends = 2,
                1 => out.done += 1,
                2 => out.errors.push("changed".into()),
                3 => state.receipts.push(ThrottleFeedback::Unavailable),
                4 => state.finishes.clear(),
                5 => state.grants = 2,
                6 => state.abandoned = 1,
                _ => unreachable!(),
            }
            assert!(
                !checks(sends, &out, &state, error)[index],
                "predicate {index} accepted mutation"
            );
        }
    }
}
