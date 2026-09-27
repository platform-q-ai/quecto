//! Actual OpenAI SSE through the existing agent-loop initiation retry owner.
use quecto::application::agent_turn::ports::AgentLoop;
use quecto::{
    application::{
        agent_loop::{AgentLoopConfig, AgentLoopImpl},
        inference_attempt::{AttemptAdmission, AttemptPermit},
    },
    domain::{
        error::DomainError,
        inference_admission::{Feedback, ThrottleFeedback},
        message::Message,
        tool::ToolProfileContext,
    },
    infrastructure::{providers::openai::OpenAiProvider, tools::registry::ToolRegistryImpl},
};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
#[derive(Debug)]
struct Gate;
impl AttemptAdmission for Gate {
    fn acquire(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Box<dyn AttemptPermit>, DomainError>> + Send + '_>>
    {
        Box::pin(async { Ok(Box::new(Gate) as Box<dyn AttemptPermit>) })
    }
}
impl AttemptPermit for Gate {
    fn feedback(&mut self, _: ThrottleFeedback) {}
    fn finish(self: Box<Self>, _: Feedback) {}
}
async fn check(error: serde_json::Value, post: bool, retries: bool) {
    let server = MockServer::start().await;
    let count = Arc::new(AtomicUsize::new(0));
    let observed = count.clone();
    let prefix = if post {
        "data: {\"choices\":[{\"delta\":{\"content\":\"visible once\"}}]}\n\n"
    } else {
        ""
    };
    let failure = format!(
        "{prefix}data: {}\n\n",
        serde_json::json!({ "error": error })
    );
    Mock::given(method("POST"))
        .respond_with(move |_: &wiremock::Request| {
            let body = if observed.fetch_add(1, Ordering::SeqCst) == 0 {
                failure.clone()
            } else {
                "data: {\"choices\":[{\"delta\":{\"content\":\"recovered\"}}]}\n\ndata: [DONE]\n\n"
                    .into()
            };
            ResponseTemplate::new(200).set_body_string(body)
        })
        .mount(&server)
        .await;
    let provider = OpenAiProvider::with_client(
        "fixture".into(),
        Some(server.uri()),
        reqwest::Client::builder().no_proxy().build().unwrap(),
    )
    .with_attempt_admission(
        Arc::new(Gate),
        quecto::infrastructure::providers::SingleAttemptClient::build(
            reqwest::Client::builder().no_proxy(),
        )
        .unwrap(),
    );
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: Arc::new(provider),
        tool_registry: Box::new(ToolRegistryImpl::new()),
        model: "fixture".into(),
        max_tokens: 32,
        temperature: 0.0,
        retention: None,
        session_key: "fixture".into(),
        context_collapse_after_tool_calls: u32::MAX,
        max_context_tokens: 100_000,
        progress_callback: None,
        streaming: true,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
        tool_profile_context: ToolProfileContext::Parent,
    })
    .with_max_tool_iterations(1);
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        agent.process(&mut vec![Message::user("hello")]),
    )
    .await
    .unwrap();
    verify(owner_checks(
        count.load(Ordering::SeqCst),
        result.as_ref().ok().map(|value| value.response.as_str()),
        retries,
    ));
    // A retry resends the request unchanged; the malformed-request recovery
    // a client error goes to would change it (#2155).
    let bodies: Vec<_> = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .map(|request| request.body)
        .collect();
    assert!(
        bodies.windows(2).all(|pair| pair[0] == pair[1]),
        "a retry resends the request unchanged"
    );
}
fn rate_limit(code: Option<&str>) -> serde_json::Value {
    let message = if code.is_some() {
        "rate limit exceeded"
    } else {
        "opaque fixture"
    };
    serde_json::json!({"type":"rate_limit_error","code":code,"message":message})
}
#[tokio::test]
async fn opaque_typed_rate_limit_retries_before_output() {
    check(rate_limit(None), false, true).await;
}
#[tokio::test]
async fn billing_code_overrides_retryable_message() {
    check(rate_limit(Some("insufficient_quota")), false, false).await;
}
#[tokio::test]
async fn typed_rate_limit_after_text_never_replays() {
    check(rate_limit(None), true, false).await;
}
/// #2155: OpenRouter's numeric gateway status mid-stream is retried.
#[tokio::test]
async fn numeric_gateway_code_before_output_retries() {
    check(
        serde_json::json!({"code": 502, "message": "upstream went away"}),
        false,
        true,
    )
    .await;
}
/// #2155: OpenAI's `server_error` chunk is a server failure: retried.
#[tokio::test]
async fn server_error_type_before_output_retries() {
    check(
        serde_json::json!({"type": "server_error", "code": null, "message": "m"}),
        false,
        true,
    )
    .await;
}
/// #2155: an error type nobody knows is retried as a bad gateway.
#[tokio::test]
async fn unknown_error_type_before_output_retries() {
    check(
        serde_json::json!({"type": "brand_new_error", "message": "m"}),
        false,
        true,
    )
    .await;
}
/// #2155: an allowlisted client error is never resent as it was: resending
/// it fails again. (A malformed request, `invalid_request_error`, goes to the
/// loop's malformed-request recovery instead, which changes the request.)
#[tokio::test]
async fn known_client_error_before_output_never_retries() {
    check(
        serde_json::json!({"type": "authentication_error", "message": "m"}),
        false,
        false,
    )
    .await;
}
/// #2155 review: a malformed-request error after output is never repaired
/// and sent again: one request, unchanged.
#[tokio::test]
async fn malformed_request_error_after_output_never_resends() {
    check(
        serde_json::json!({"type": "invalid_request_error", "message": "m"}),
        true,
        false,
    )
    .await;
}
/// #2155: a retryable server error after output never replays the reply.
#[tokio::test]
async fn server_error_after_output_never_replays() {
    check(
        serde_json::json!({"type": "server_error", "message": "m"}),
        true,
        false,
    )
    .await;
}

fn owner_checks(count: usize, response: Option<&str>, retries: bool) -> [bool; 3] {
    [
        count == if retries { 2 } else { 1 },
        response.is_some() == retries,
        !retries || response == Some("recovered"),
    ]
}
fn verify(checks: [bool; 3]) {
    assert!(
        checks.into_iter().all(|ok| ok),
        "retry-owner acceptance failed: {checks:?}"
    );
}
#[test]
fn owner_assertions_reject_observed_counterexamples() {
    verify(owner_checks(2, Some("recovered"), true));
    verify(owner_checks(1, None, false));
    assert!(!owner_checks(1, Some("recovered"), true)[0]);
    assert!(!owner_checks(2, None, true)[1]);
    assert!(!owner_checks(1, Some("unexpected"), false)[1]);
    assert!(!owner_checks(2, Some("changed"), true)[2]);
}
