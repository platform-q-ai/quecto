//! The Responses API also emits top-level error fields, not only nested
//! response.failed payloads. Preserve its established display while forwarding
//! the structured throttle receipt from the real wire event.
#[path = "common/admission_feedback_fixture.rs"]
pub mod fixture;
#[path = "common/admission_leaf_error_oracle.rs"]
mod oracle;
use fixture::*;
use std::sync::Arc;

async fn check(leaf: Leaf, post_text: bool) {
    let prefix = if post_text {
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"visible once\"}\n\n"
    } else {
        ""
    };
    let server = Server::start(Reply {
        status: 200,
        headers: "Content-Type: text/event-stream\r\n".into(),
        body: format!("{prefix}data: {{\"type\":\"error\",\"code\":\"rate_limit_exceeded\",\"message\":\"opaque fixture\"}}\n\ndata: [DONE]\n\n"),
        stalled: false,
    }).await;
    let gate = Arc::new(Gate::default());
    let provider = leaf.provider(&server.url, gate.clone());
    let output = bounded(stream(provider.as_ref())).await;
    oracle::verify(oracle::checks(
        server.starts(),
        &output,
        &gate.snapshot(),
        oracle::Expected {
            post: post_text,
            enabled: true,
            throttle: true,
            error: "Responses stream error",
        },
    ));
}
#[tokio::test]
async fn responses_root_error_before_text_forwards_throttle_without_done() {
    check(Leaf::Responses, false).await;
}
#[tokio::test]
async fn responses_root_error_after_text_preserves_output_without_replay() {
    check(Leaf::Responses, true).await;
}
#[tokio::test]
async fn oauth_root_error_before_text_forwards_throttle_without_done() {
    check(Leaf::OAuth, false).await;
}
#[tokio::test]
async fn oauth_root_error_after_text_preserves_output_without_replay() {
    check(Leaf::OAuth, true).await;
}
