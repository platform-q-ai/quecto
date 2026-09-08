//! Receipt observation must respect the provider parser's dispatch and terminal
//! state, even while assembled calls keep consuming the HTTP body to EOF.
#[path = "common/admission_feedback_fixture.rs"]
pub mod fixture;
use fixture::*;
use quecto::domain::inference_admission::Feedback;
use std::sync::Arc;
async fn check(leaf: Leaf, terminal: bool, incremental: bool) {
    let body = match leaf {
        Leaf::Anthropic => {
            let prefix = if terminal {"event: message_stop\ndata: {}\n\n"} else {"event: ignored\n"};
            format!("{prefix}data: {{\"type\":\"error\",\"error\":{{\"type\":\"overloaded_error\"}}}}\n\nevent: message_stop\ndata: {{}}\n\n")
        },
        _ => "data: {\"type\":\"response.completed\",\"response\":{}}\n\ndata: {\"type\":\"error\",\"code\":\"rate_limit_exceeded\"}\n\n".into(),
    };
    let server = Server::start(Reply {
        status: 200,
        headers: "Content-Type: text/event-stream\r\n".into(),
        body,
        stalled: false,
    })
    .await;
    let gate = Arc::new(Gate::default());
    let provider = leaf.provider(&server.url, gate.clone());
    let success = if incremental {
        let output = bounded(stream(provider.as_ref())).await;
        output.done == 1 && output.errors.is_empty()
    } else {
        bounded(provider.chat_stream(request())).await.is_ok()
    };
    verify(observer_checks(success, &gate.snapshot()));
}
#[tokio::test]
async fn responses_assembled_ignores_error_after_completed() {
    check(Leaf::Responses, true, false).await;
}
#[tokio::test]
async fn oauth_assembled_ignores_error_after_completed() {
    check(Leaf::OAuth, true, false).await;
}
#[tokio::test]
async fn anthropic_assembled_ignores_error_after_stop() {
    check(Leaf::Anthropic, true, false).await;
}
#[tokio::test]
async fn anthropic_assembled_ignores_error_under_unknown_event() {
    check(Leaf::Anthropic, false, false).await;
}
#[tokio::test]
async fn anthropic_incremental_ignores_error_under_unknown_event() {
    check(Leaf::Anthropic, false, true).await;
}

fn observer_checks(success: bool, state: &Snapshot) -> [bool; 3] {
    [
        success,
        state.receipts.is_empty(),
        state.finishes == [Feedback::Success],
    ]
}
fn verify(checks: [bool; 3]) {
    assert!(
        checks.into_iter().all(|ok| ok),
        "observer acceptance failed: {checks:?}"
    );
}
#[test]
fn observer_assertions_reject_observed_counterexamples() {
    let mut state = Snapshot::default();
    state.finishes = vec![Feedback::Success];
    verify(observer_checks(true, &state));
    assert!(!observer_checks(false, &state)[0]);
    state
        .receipts
        .push(quecto::domain::inference_admission::ThrottleFeedback::Unavailable);
    assert!(!observer_checks(true, &state)[1]);
    state.receipts.clear();
    state.finishes = vec![Feedback::Failure];
    assert!(!observer_checks(true, &state)[2]);
}

async fn extension(leaf: Leaf, incremental: bool, throttle: bool) {
    let extension_code = if throttle {
        "invalid_request_error"
    } else {
        "rate_limit_exceeded"
    };
    let ending = if throttle {
        "data: {\"type\":\"error\",\"code\":\"rate_limit_exceeded\"}\n\n"
    } else {
        "data: {\"type\":\"response.completed\",\"response\":{}}\n\n"
    };
    let server = Server::start(Reply {status:200, headers:"Content-Type: text/event-stream\r\n".into(),body:format!("data: {{\"type\":\"extension\",\"error\":{{\"code\":\"{extension_code}\"}}}}\n\n{ending}"),stalled:false}).await;
    let gate = Arc::new(Gate::default());
    let provider = leaf.provider(&server.url, gate.clone());
    let success = if incremental {
        let output = bounded(stream(provider.as_ref())).await;
        output.done == 1 && output.errors.is_empty()
    } else {
        bounded(provider.chat_stream(request())).await.is_ok()
    };
    let state = gate.snapshot();
    verify(extension_checks(success, &state, throttle));
}
fn extension_checks(success: bool, state: &Snapshot, throttle: bool) -> [bool; 3] {
    [
        success != throttle,
        state.receipts.len() == usize::from(throttle),
        state.finishes
            == [if throttle {
                Feedback::Failure
            } else {
                Feedback::Success
            }],
    ]
}
macro_rules! extension_cases {
    ($module:ident,$leaf:expr,$incremental:expr) => {
        mod $module {
            use super::*;
            #[tokio::test]
            async fn unknown_error_shape_then_success_has_no_feedback() {
                extension($leaf, $incremental, false).await;
            }
            #[tokio::test]
            async fn unknown_error_shape_does_not_hide_later_throttle() {
                extension($leaf, $incremental, true).await;
            }
        }
    };
}
extension_cases!(responses_incremental, Leaf::Responses, true);
extension_cases!(responses_assembled, Leaf::Responses, false);
extension_cases!(oauth_incremental, Leaf::OAuth, true);
extension_cases!(oauth_assembled, Leaf::OAuth, false);
#[test]
fn extension_assertions_reject_observed_counterexamples() {
    for throttle in [false, true] {
        let mut state = Snapshot::default();
        state.finishes = vec![if throttle {
            Feedback::Failure
        } else {
            Feedback::Success
        }];
        if throttle {
            state.receipts =
                vec![quecto::domain::inference_admission::ThrottleFeedback::NoHint { jitter: 0 }];
        }
        verify(extension_checks(!throttle, &state, throttle));
        assert!(!extension_checks(throttle, &state, throttle)[0]);
        state
            .receipts
            .push(quecto::domain::inference_admission::ThrottleFeedback::Unavailable);
        assert!(!extension_checks(!throttle, &state, throttle)[1]);
        state.finishes.clear();
        assert!(!extension_checks(!throttle, &state, throttle)[2]);
    }
}
