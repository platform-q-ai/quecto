//! Enabled OpenAI SSE errors are terminal structured failures; disabled behavior
//! remains characterized separately. Provider message text is displayed, not classified.
#[path = "common/admission_feedback_fixture.rs"]
pub mod fixture;
#[path = "common/admission_leaf_error_oracle.rs"]
mod oracle;
use fixture::*;
use quecto::infrastructure::providers::openai::OpenAiProvider;
use serde_json::json;
use std::sync::Arc;

async fn check(post_text: bool, kind: &str, code: Option<&str>, throttle: bool, enabled: bool) {
    let error = json!({"type": kind, "code": code, "message": "opaque fixture"});
    let prefix = if post_text {
        "data: {\"choices\":[{\"delta\":{\"content\":\"visible once\"}}]}\n\n"
    } else {
        ""
    };
    let server = Server::start(Reply {
        status: 200,
        headers: "Content-Type: text/event-stream\r\n".into(),
        body: format!(
            "{prefix}data: {}\n\ndata: [DONE]\n\n",
            json!({"error": error})
        ),
        stalled: false,
    })
    .await;
    let gate = Arc::new(Gate::default());
    let provider = OpenAiProvider::with_client(
        "fixture".into(),
        Some(server.url.clone()),
        reqwest::Client::builder().no_proxy().build().unwrap(),
    );
    let provider = if enabled {
        provider.with_attempt_admission(gate.clone())
    } else {
        provider
    };
    let output = bounded(stream(&provider)).await;
    oracle::verify(oracle::checks(
        server.starts(),
        &output,
        &gate.snapshot(),
        oracle::Expected {
            post: post_text,
            enabled,
            throttle,
            error: &format!(
                "HTTP {} OpenAI stream error: {}",
                if throttle { 429 } else { 400 },
                json!({"error": error})
            ),
        },
    ));
}
#[tokio::test]
async fn disabled_error_payload_keeps_legacy_done_behavior() {
    check(true, "rate_limit_error", None, true, false).await;
}
#[tokio::test]
async fn typed_throttle_before_text_is_one_error_without_done() {
    check(false, "rate_limit_error", None, true, true).await;
}
#[tokio::test]
async fn typed_throttle_after_text_is_one_error_without_replay() {
    check(true, "rate_limit_error", None, true, true).await;
}
#[tokio::test]
async fn billing_overrides_throttle_but_remains_terminal() {
    check(
        true,
        "rate_limit_error",
        Some("insufficient_quota"),
        false,
        true,
    )
    .await;
}
#[tokio::test]
async fn untyped_structured_error_is_terminal_without_cooldown() {
    check(false, "invalid_request_error", None, false, true).await;
}
