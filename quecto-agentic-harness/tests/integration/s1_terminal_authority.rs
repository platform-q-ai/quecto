//! Actual Codex consumers own terminal authority, including their existing
//! lossy whole-read decoding. No general malformed-framing acceptance claim.
use super::fixture::*;
use quecto::domain::admission::value_objects::inference_admission::{Feedback, ThrottleFeedback};
use quecto::domain::error::DomainError;
use quecto::domain::inference::events::request_observation::RequestTrace;
use quecto::domain::inference::services::provider_error::{
    ProviderErrorClass, classify_provider_error, provider_http_status,
};
use quecto::domain::inference::value_objects::attempt_diagnostics::{
    AttemptDiagnostics, ErrorCode, TerminalEvent, Termination,
};
use quecto::domain::inference::value_objects::provider::StreamEvent;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct RawServer {
    url: String,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for RawServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl RawServer {
    async fn start(body: Vec<u8>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut received = Vec::new();
            let mut chunk = [0; 4096];
            loop {
                let n = socket.read(&mut chunk).await.unwrap();
                assert!(n > 0, "request arrives before EOF");
                received.extend_from_slice(&chunk[..n]);
                if let Some(end) = received.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&received[..end]);
                    let length: usize = head
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse().unwrap())
                        })
                        .unwrap_or(0);
                    if received.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(head.as_bytes()).await.unwrap();
            socket.write_all(&body).await.unwrap();
            socket.shutdown().await.unwrap();
        });
        Self { url, task }
    }
}
fn first(size: usize, message: &str) -> Vec<u8> {
    format!("data: {{\"type\":\"response.failed\",\"response\":{{\"error\":{{\"type\":\"invalid_request_error\",\"message\":\"{message}{}\"}}}}}}\n\n", "p".repeat(size)).into_bytes()
}
fn throttle() -> &'static str {
    "data: {\"type\":\"error\",\"error\":{\"type\":\"rate_limit_error\",\"code\":\"rate_limit_exceeded\"}}\n\n"
}
fn success(size: usize) -> String {
    format!(
        "data: {{\"type\":\"response.output_text.delta\",\"delta\":\"{}\"}}\n\ndata: {{\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{{\"type\":\"function_call\",\"id\":\"item\",\"call_id\":\"call\",\"name\":\"fixture\",\"arguments\":\"\"}}}}\n\ndata: {{\"type\":\"response.function_call_arguments.delta\",\"output_index\":0,\"item_id\":\"item\",\"delta\":\"{{}}\"}}\n\ndata: {{\"type\":\"response.function_call_arguments.done\",\"output_index\":0,\"item_id\":\"item\",\"arguments\":\"{{}}\"}}\n\ndata: {{\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{{\"type\":\"function_call\",\"id\":\"item\",\"call_id\":\"call\",\"name\":\"fixture\",\"arguments\":\"{{}}\"}}}}\n\n{}{}",
        "t".repeat(size),
        Leaf::Responses.terminal(),
        throttle()
    )
}
#[derive(Clone, Copy, Debug)]
enum Scenario {
    RejectedThrottle,
    RejectedSuccess,
    LossyTerminal,
    LossyThrottle,
    InertShapeSuccess,
    ValidTerminal,
    NormalThrottle,
}
fn scenario_body(scenario: Scenario, padding: usize, text_size: usize) -> Vec<u8> {
    let mut body = match scenario {
        Scenario::RejectedThrottle | Scenario::RejectedSuccess => first(padding, "\\uD800"),
        Scenario::LossyTerminal | Scenario::LossyThrottle => {
            let mut raw = first(padding, "X");
            let offset = raw.iter().position(|b| *b == b'X').unwrap();
            raw[offset] = 0xff;
            if matches!(scenario, Scenario::LossyThrottle) {
                raw = String::from_utf8_lossy(&raw)
                    .replace("invalid_request_error", "rate_limit_error")
                    .into_bytes();
                let repaired = raw
                    .windows(3)
                    .position(|w| w == [0xef, 0xbf, 0xbd])
                    .unwrap();
                raw.splice(repaired..repaired + 3, [0xff]);
            }
            raw
        }
        Scenario::InertShapeSuccess => format!(
            "data: {{\"type\":[],\"error\":[],\"ignored\":\"{}\"}}\n\n",
            "p".repeat(padding)
        )
        .into_bytes(),
        Scenario::ValidTerminal => first(padding, "valid"),
        Scenario::NormalThrottle => Vec::new(),
    };
    body.extend_from_slice(
        if matches!(
            scenario,
            Scenario::RejectedSuccess | Scenario::InertShapeSuccess
        ) {
            success(text_size)
        } else {
            throttle().to_owned()
        }
        .as_bytes(),
    );
    body
}
async fn row(
    leaf: Leaf,
    surface: Surface,
    admitted: bool,
    padding: usize,
    scenario: Scenario,
) -> bool {
    let text_size = if padding == 0 { 7 } else { 6000 };
    let body = scenario_body(scenario, padding, text_size);
    let mut server = RawServer::start(body).await;
    let gate = Arc::new(Gate::default());
    let provider = leaf.provider(&server.url, admitted.then(|| gate.clone()));
    let trace = Arc::new(RequestTrace::default());
    trace.start();
    let mut req = request();
    req.trace = Some(trace.clone());
    let result = match surface {
        Surface::Chat => bounded(provider.chat(req)).await,
        Surface::Assembled => bounded(provider.chat_stream(req)).await,
        Surface::Incremental => {
            let mut rx = provider.chat_stream_incremental(req).await;
            let mut result = None;
            let mut terminals = 0;
            let mut visible = String::new();
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
                    StreamEvent::TextDelta(text) => visible.push_str(&text),
                    _ => {}
                }
            }
            assert_eq!(
                visible,
                if matches!(
                    scenario,
                    Scenario::RejectedSuccess | Scenario::InertShapeSuccess
                ) {
                    "t".repeat(text_size)
                } else {
                    String::new()
                },
                "actual text notifications exactly once"
            );
            assert_eq!(terminals, 1, "one outward terminal");
            result.unwrap()
        }
    };
    bounded(&mut server.task).await.unwrap();
    let success = matches!(
        scenario,
        Scenario::RejectedSuccess | Scenario::InertShapeSuccess
    );
    let throttled = matches!(
        scenario,
        Scenario::RejectedThrottle | Scenario::NormalThrottle
    );
    if success {
        let response = result.as_ref().unwrap();
        assert_eq!(
            response.content.as_deref(),
            Some("t".repeat(text_size).as_str())
        );
        assert_eq!(response.tool_calls.len(), 1, "never duplicate tools");
        assert_eq!(response.tool_calls[0].id, "call");
        assert_eq!(response.tool_calls[0].name, "fixture");
        assert_eq!(response.tool_calls[0].arguments, "{}");
    } else {
        let error = result.as_ref().unwrap_err();
        assert_eq!(
            classify_provider_error(error),
            if throttled || (matches!(scenario, Scenario::LossyThrottle) && padding == 0) {
                ProviderErrorClass::RateLimit
            } else if padding == 0 {
                ProviderErrorClass::Client
            } else {
                ProviderErrorClass::Server
            }
        );
        assert_eq!(
            provider_http_status(error),
            Some(
                if throttled || (matches!(scenario, Scenario::LossyThrottle) && padding == 0) {
                    429
                } else if padding == 0 {
                    400
                } else {
                    502
                }
            )
        );
        assert!(!error.to_string().contains("CANARY"));
    }
    let attempts = trace.attempt_diagnostics();
    assert_eq!(attempts.len(), 1);
    let actual = &attempts[0];
    let state = gate.snapshot();
    let expected_receipts = if admitted && throttled {
        vec![ThrottleFeedback::NoHint { jitter: u64::MAX }]
    } else {
        vec![]
    };
    let expected_finish = if admitted {
        vec![if success {
            Feedback::Success
        } else {
            Feedback::Failure
        }]
    } else {
        vec![]
    };
    let correct = diagnostics_match(actual, surface, scenario, padding, text_size)
        && state.grants == usize::from(admitted)
        && state.abandoned == 0
        && state.receipts == expected_receipts
        && state.finishes == expected_finish
        && state.blocked == (admitted && throttled);
    eprintln!(
        "AUTHORITY {leaf:?}/{surface:?}/{admitted}/{padding}/{scenario:?} correct={correct} actual={actual:?} permit={state:?}"
    );
    correct
}
// Keep the receipt oracle cohesive and separate from executing the provider.
fn diagnostics_match(
    actual: &AttemptDiagnostics,
    surface: Surface,
    scenario: Scenario,
    padding: usize,
    text_size: usize,
) -> bool {
    let success = matches!(
        scenario,
        Scenario::RejectedSuccess | Scenario::InertShapeSuccess
    );
    let throttled = matches!(
        scenario,
        Scenario::RejectedThrottle | Scenario::NormalThrottle
    );
    let expected_code = if throttled {
        Some(ErrorCode::RateLimitExceeded)
    } else if matches!(scenario, Scenario::ValidTerminal) && padding == 0 {
        Some(ErrorCode::InvalidRequestError)
    } else {
        None
    };
    let expected_terminal = if success {
        TerminalEvent::ResponseCompleted
    } else if throttled {
        TerminalEvent::Error
    } else {
        TerminalEvent::ResponseFailed
    };
    actual.error_code == expected_code
        && actual.terminal_event == Some(expected_terminal)
        && actual.termination
            == if success || matches!(surface, Surface::Incremental) {
                Termination::Completed
            } else {
                Termination::Rejected
            }
        && actual.event_count
            == if success {
                7
            } else if matches!(scenario, Scenario::RejectedThrottle) {
                2
            } else {
                1
            }
        && actual.parse_errors
            == u32::from(matches!(
                scenario,
                Scenario::RejectedThrottle
                    | Scenario::RejectedSuccess
                    | Scenario::LossyTerminal
                    | Scenario::LossyThrottle
            ))
        && actual.first_token_ms.is_some() == success
        && actual.generated_text == success
        && actual.generated_tool_call == success
        && !actual.generated_thinking
        && actual.output_bytes == if success { text_size as u64 + 2 } else { 0 }
}
async fn matrix(scenario: Scenario, incremental: bool) {
    let mut wrong = 0;
    let mut count = 0;
    for leaf in [Leaf::Responses, Leaf::OAuth] {
        let surfaces = if incremental {
            vec![Surface::Incremental]
        } else {
            vec![Surface::Chat, Surface::Assembled]
        };
        for surface in surfaces {
            for admitted in [false, true] {
                for padding in [0, 4200] {
                    count += 1;
                    match row(leaf, surface, admitted, padding, scenario).await {
                        true => {}
                        false => wrong += 1,
                    }
                }
            }
        }
    }
    eprintln!("AUTHORITY SUMMARY {scenario:?} selected={count} wrong={wrong}");
    assert_eq!(
        wrong, 0,
        "{scenario:?}: {wrong}/{count} consumer/receipt disagreements"
    );
}
#[tokio::test]
async fn s1_terminal_authority_rejected_then_throttle() {
    matrix(Scenario::RejectedThrottle, false).await;
}
#[tokio::test]
async fn s1_terminal_authority_rejected_then_success() {
    matrix(Scenario::RejectedSuccess, false).await;
}
#[tokio::test]
async fn s1_terminal_authority_lossy_consumed_first() {
    matrix(Scenario::LossyTerminal, false).await;
}
#[tokio::test]
async fn s1_terminal_authority_valid_first_control() {
    matrix(Scenario::ValidTerminal, false).await;
}
#[tokio::test]
async fn s1_terminal_authority_throttle_control() {
    matrix(Scenario::NormalThrottle, false).await;
}
#[tokio::test]
async fn s1_terminal_authority_incremental_rejected_control() {
    matrix(Scenario::RejectedThrottle, true).await;
}
#[tokio::test]
async fn s1_terminal_authority_incremental_success_control() {
    matrix(Scenario::RejectedSuccess, true).await;
}

#[tokio::test]
async fn s1_terminal_authority_lossy_throttle_no_semantics() {
    matrix(Scenario::LossyThrottle, false).await;
}
#[tokio::test]
async fn s1_terminal_authority_inert_shape_success_control() {
    matrix(Scenario::InertShapeSuccess, false).await;
}
