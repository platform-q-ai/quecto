//! Enabled OpenAI SSE errors are terminal structured failures; disabled behavior
//! remains characterized separately. Provider message text is displayed, not classified.
// One copy per crate (clippy::duplicate_mod): the fixture is loaded by
// `inference_admission_feedback_transport`; the leaf-error oracle (which reads
// `super::fixture`) is owned here and imported by `inference_admission_responses_root_error`.
use crate::inference_admission_feedback_transport::fixture;
#[path = "../common/admission_leaf_error_oracle.rs"]
pub(crate) mod oracle;
use fixture::*;
use quecto::infrastructure::providers::openai::OpenAiProvider;
use serde_json::json;
use std::sync::Arc;

async fn check(post_text: bool, kind: &str, code: Option<&str>, throttle: bool, enabled: bool) {
    let status = match (throttle, code) {
        (true, _) => 429,
        (false, Some("insufficient_quota")) => 402,
        (false, _) => 400,
    };
    let error = json!({"type": kind, "code": code, "message": "opaque fixture"});
    check_error(post_text, error, status, throttle, enabled).await;
}
/// One OpenAI-compatible stream that fails with the error chunk `error`
/// (after one text delta when `post_text`): through admission it ends as one
/// error rendered with `status`; the text already streamed stays streamed
/// once, and no completed reply follows it (#2155).
async fn check_error(
    post_text: bool,
    error: serde_json::Value,
    status: u16,
    throttle: bool,
    enabled: bool,
) {
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
        provider.with_attempt_admission(
            gate.clone(),
            quecto::infrastructure::providers::SingleAttemptClient::build(
                reqwest::Client::builder().no_proxy(),
            )
            .unwrap(),
        )
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
                "HTTP {status} OpenAI stream error: {}",
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
/// #2155: every error-chunk shape keeps the partial text streamed once and
/// ends with one error carrying a retry-classifiable status.
#[tokio::test]
async fn numeric_gateway_code_is_the_status() {
    check_error(
        false,
        json!({"code": 502, "message": "m"}),
        502,
        false,
        true,
    )
    .await;
}
#[tokio::test]
async fn server_error_type_is_a_server_status() {
    check_error(
        false,
        json!({"type": "server_error", "code": null}),
        500,
        false,
        true,
    )
    .await;
}
#[tokio::test]
async fn unknown_error_type_is_a_bad_gateway() {
    check_error(false, json!({"type": "brand_new_error"}), 502, false, true).await;
}
#[tokio::test]
async fn server_error_after_partial_text_keeps_the_text_and_ends_in_error() {
    check_error(true, json!({"type": "server_error"}), 500, false, true).await;
}
#[tokio::test]
async fn numeric_gateway_code_after_partial_text_keeps_the_text_and_ends_in_error() {
    check_error(true, json!({"code": 502}), 502, false, true).await;
}
