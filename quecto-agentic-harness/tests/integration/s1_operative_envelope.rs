//! Actual-provider operative SSE authority, not formatter-only agreement.
use super::fixture::*;
use quecto::domain::admission::value_objects::inference_admission::{Feedback, ThrottleFeedback};
use quecto::domain::error::DomainError;
use quecto::domain::inference::events::request_observation::RequestTrace;
use quecto::domain::inference::services::provider_error::{
    ProviderErrorClass, classify_provider_error, provider_http_status,
};
use quecto::domain::inference::value_objects::attempt_diagnostics::{
    ErrorCode, TerminalEvent, Termination,
};
use quecto::domain::inference::value_objects::provider::StreamEvent;
use serde_json::json;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Server {
    url: String,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    async fn start(body: String) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                socket.read_exact(&mut byte).await.unwrap();
                head.push(byte[0]);
                assert!(head.len() < 32768);
            }
            let head = String::from_utf8(head).unwrap();
            let length: usize = head
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse().unwrap())
                })
                .unwrap();
            assert!(length < 1_048_576);
            socket.read_exact(&mut vec![0; length]).await.unwrap();
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(head.as_bytes()).await.unwrap();
            socket.write_all(body.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
        });
        Self { url, task }
    }
}
#[derive(Clone, Copy, Debug)]
enum Case {
    ClientIgnoredThrottle,
    ThrottleIgnoredBilling,
    UnknownIgnoredThrottle,
    DirectThrottle,
    NestedThrottle,
    RootUnknown,
    RootThrottle,
    SelectedBilling,
    SelectedBillingCode,
    Output,
}
impl Case {
    fn body(self) -> String {
        let throttle = json!({"type":"rate_limit_error","code":"rate_limit_exceeded","message":"CANARY_TOKEN https://CANARY_URL"});
        let billing = json!({"type":"insufficient_quota","code":"rate_limit_exceeded","message":"CANARY_BILLING"});
        let value = match self {
            Self::ClientIgnoredThrottle => {
                json!({"type":"response.failed","response":{"error":{"type":"invalid_request_error","message":"CANARY_PROMPT"}},"error":throttle})
            }
            Self::ThrottleIgnoredBilling => {
                json!({"type":"response.failed","response":{"error":throttle},"error":billing})
            }
            Self::UnknownIgnoredThrottle => {
                json!({"type":"response.failed","response":{"error":{"type":"CANARY_UNKNOWN"}},"error":throttle})
            }
            Self::DirectThrottle => json!({"type":"error","error":throttle}),
            Self::NestedThrottle => json!({"type":"response.failed","response":{"error":throttle}}),
            Self::RootUnknown => {
                json!({"type":"error","code":"CANARY_UNKNOWN","message":"CANARY_PROMPT"})
            }
            Self::RootThrottle => {
                json!({"type":"error","code":"rate_limit_exceeded","message":"CANARY_PROMPT"})
            }
            Self::SelectedBilling => {
                json!({"type":"response.failed","response":{"error":billing},"error":throttle})
            }
            Self::SelectedBillingCode => {
                json!({"type":"response.failed","response":{"error":{"type":"rate_limit_error","code":"insufficient_quota","message":"CANARY_PROMPT"}},"error":throttle})
            }
            Self::Output => return output(),
        };
        let raw = value.to_string();
        assert!(raw.len() <= 4096, "original raw event admitted");
        // A later conflicting terminal must never acquire authority.
        format!(
            "data: {raw}\n\ndata: {{\"type\":\"error\",\"error\":{{\"type\":\"rate_limit_error\"}}}}\n\n"
        )
    }
    fn expectation(self) -> (u16, Option<ErrorCode>, TerminalEvent) {
        match self {
            Self::ClientIgnoredThrottle => (
                400,
                Some(ErrorCode::InvalidRequestError),
                TerminalEvent::ResponseFailed,
            ),
            Self::ThrottleIgnoredBilling | Self::NestedThrottle => (
                429,
                Some(ErrorCode::RateLimitExceeded),
                TerminalEvent::ResponseFailed,
            ),
            Self::DirectThrottle | Self::RootThrottle => (
                429,
                Some(ErrorCode::RateLimitExceeded),
                TerminalEvent::Error,
            ),
            Self::RootUnknown => (502, None, TerminalEvent::Error),
            Self::UnknownIgnoredThrottle => (502, None, TerminalEvent::ResponseFailed),
            Self::SelectedBilling => (
                402,
                Some(ErrorCode::InsufficientQuota),
                TerminalEvent::ResponseFailed,
            ),
            // Preserve finite declared-type diagnostics; billing precedence
            // governs outward classification and feedback within this envelope.
            Self::SelectedBillingCode => (
                402,
                Some(ErrorCode::RateLimitExceeded),
                TerminalEvent::ResponseFailed,
            ),
            Self::Output => (200, None, TerminalEvent::ResponseCompleted),
        }
    }
}
fn output() -> String {
    format!(
        "data: {{\"type\":\"response.output_text.delta\",\"delta\":\"hello 世界\"}}\n\ndata: {{\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{{\"type\":\"function_call\",\"id\":\"item\",\"call_id\":\"call\",\"name\":\"fixture\",\"arguments\":\"\"}}}}\n\ndata: {{\"type\":\"response.function_call_arguments.delta\",\"output_index\":0,\"item_id\":\"item\",\"delta\":\"{{}}\"}}\n\ndata: {{\"type\":\"response.function_call_arguments.done\",\"output_index\":0,\"item_id\":\"item\",\"arguments\":\"{{}}\"}}\n\ndata: {{\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{{\"type\":\"function_call\",\"id\":\"item\",\"call_id\":\"call\",\"name\":\"fixture\",\"arguments\":\"{{}}\"}}}}\n\n{}data: {{\"type\":\"error\",\"error\":{{\"type\":\"rate_limit_error\"}}}}\n\n",
        Leaf::Responses.terminal()
    )
}
async fn row(leaf: Leaf, surface: Surface, admitted: bool, case: Case) -> bool {
    let mut server = Server::start(case.body()).await;
    let gate = Arc::new(Gate::default());
    let provider = leaf.provider(&server.url, admitted.then(|| gate.clone()));
    let trace = Arc::new(RequestTrace::default());
    trace.start();
    let mut req = request();
    req.trace = Some(trace.clone());
    let mut notifications = Vec::new();
    let result = match surface {
        Surface::Chat => bounded(provider.chat(req)).await,
        Surface::Assembled => bounded(provider.chat_stream(req)).await,
        Surface::Incremental => {
            let mut rx = provider.chat_stream_incremental(req).await;
            let mut result = None;
            let mut terminals = 0;
            while let Some(event) = bounded(rx.recv()).await {
                match event {
                    StreamEvent::Done(response) => {
                        terminals += 1;
                        result = Some(Ok(response));
                    }
                    StreamEvent::Error(error) => {
                        terminals += 1;
                        result = Some(Err(DomainError::Provider(error)));
                    }
                    other => notifications.push(other),
                }
            }
            assert_eq!(terminals, 1, "one actual consumer terminal");
            result.unwrap()
        }
    };
    bounded(&mut server.task).await.unwrap();
    let (status, code, terminal) = case.expectation();
    let success = status == 200;
    if success {
        let response = result.as_ref().unwrap();
        assert_eq!(response.content.as_deref(), Some("hello 世界"));
        assert_eq!(response.tool_calls.len(), 1);
        let tool = &response.tool_calls[0];
        assert_eq!(
            (&*tool.id, &*tool.name, &*tool.arguments),
            ("call", "fixture", "{}")
        );
        if matches!(surface, Surface::Incremental) {
            // Codex delivers tools in Done, not incremental tool notifications.
            assert_eq!(format!("{notifications:?}"), "[TextDelta(\"hello 世界\")]");
        }
    } else {
        let error = result.as_ref().unwrap_err();
        assert_eq!(provider_http_status(error), Some(status));
        assert_eq!(
            classify_provider_error(error),
            ProviderErrorClass::from_status(status)
        );
        assert!(
            notifications.is_empty(),
            "terminal error emits no text/tool activity"
        );
        assert!(
            !error.to_string().contains("CANARY"),
            "no raw provider fields"
        );
    }
    let attempts = trace.attempt_diagnostics();
    assert_eq!(attempts.len(), 1, "one real provider send");
    let actual = &attempts[0];
    assert!(
        !format!("{actual:?}").contains("CANARY"),
        "finite passive/active diagnostics"
    );
    let state = gate.snapshot();
    let throttled = status == 429;
    let receipts = if admitted && throttled {
        vec![ThrottleFeedback::NoHint { jitter: u64::MAX }]
    } else {
        vec![]
    };
    let finishes = if admitted {
        vec![if success {
            Feedback::Success
        } else {
            Feedback::Failure
        }]
    } else {
        vec![]
    };
    let correct = actual.error_code == code
        && actual.terminal_event == Some(terminal)
        && actual.termination
            == if success || matches!(surface, Surface::Incremental) {
                Termination::Completed
            } else {
                Termination::Rejected
            }
        && actual.event_count == if success { 6 } else { 1 }
        && actual.parse_errors == 0
        && actual.generated_text == success
        && actual.generated_tool_call == success
        && !actual.generated_thinking
        && actual.first_token_ms.is_some() == success
        && actual.output_bytes
            == if success {
                "hello 世界".len() as u64 + 2
            } else {
                0
            }
        && state.grants == usize::from(admitted)
        && state.abandoned == 0
        && state.receipts == receipts
        && state.finishes == finishes
        && state.blocked == (admitted && throttled);
    eprintln!(
        "OPERATIVE {case:?}/{leaf:?}/{surface:?}/admitted={admitted} status={status} expected_code={code:?} expected_receipts={receipts:?} correct={correct} actual={actual:?} permit={state:?}"
    );
    correct
}
async fn matrix(case: Case) {
    let mut wrong = 0;
    let mut selected = 0;
    for leaf in [Leaf::Responses, Leaf::OAuth] {
        for surface in [Surface::Chat, Surface::Assembled, Surface::Incremental] {
            for admitted in [false, true] {
                selected += 1;
                if row(leaf, surface, admitted, case).await {
                } else {
                    wrong += 1;
                }
            }
        }
    }
    eprintln!(
        "OPERATIVE SUMMARY {case:?} selected={selected} passed={} failed={wrong}",
        selected - wrong
    );
    assert_eq!(
        wrong, 0,
        "{case:?}: {wrong}/{selected} operative-envelope disagreements"
    );
}
#[tokio::test]
async fn s1_operative_envelope_client_ignored_throttle() {
    matrix(Case::ClientIgnoredThrottle).await;
}
#[tokio::test]
async fn s1_operative_envelope_throttle_ignored_billing() {
    matrix(Case::ThrottleIgnoredBilling).await;
}
#[tokio::test]
async fn s1_operative_envelope_unknown_ignored_throttle() {
    matrix(Case::UnknownIgnoredThrottle).await;
}
#[tokio::test]
async fn s1_operative_envelope_direct_control() {
    matrix(Case::DirectThrottle).await;
}
#[tokio::test]
async fn s1_operative_envelope_nested_control() {
    matrix(Case::NestedThrottle).await;
}
#[tokio::test]
async fn s1_operative_envelope_root_unknown_control() {
    matrix(Case::RootUnknown).await;
}
#[tokio::test]
async fn s1_operative_envelope_root_throttle() {
    matrix(Case::RootThrottle).await;
}
#[tokio::test]
async fn s1_operative_envelope_selected_billing_control() {
    matrix(Case::SelectedBilling).await;
}
#[tokio::test]
async fn s1_operative_envelope_selected_billing_code() {
    matrix(Case::SelectedBillingCode).await;
}
#[tokio::test]
async fn s1_operative_envelope_output_control() {
    matrix(Case::Output).await;
}

#[tokio::test]
async fn s1_c123_ignored_envelopes_billing_exact_output_controls() {
    for case in [
        Case::ClientIgnoredThrottle,
        Case::ThrottleIgnoredBilling,
        Case::UnknownIgnoredThrottle,
        Case::SelectedBilling,
        Case::SelectedBillingCode,
        Case::Output,
    ] {
        matrix(case).await;
    }
    eprintln!("C123 IGNORED BILLING OUTPUT SUMMARY selected=72 failed=0");
}
