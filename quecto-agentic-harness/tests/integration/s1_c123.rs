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
struct Case {
    shape: usize,
    declaration: usize,
}
impl Case {
    fn selected(self) -> serde_json::Value {
        match self.declaration {
            0 => json!({"type":"authentication_error","code":"rate_limit_exceeded"}),
            1 => json!({"code":"authentication_error","type":"rate_limit_exceeded"}),
            2 => json!({"type":"invalid_request_error","code":"rate_limit_exceeded"}),
            3 => json!({"code":"invalid_request_error","type":"rate_limit_exceeded"}),
            4 => json!({"code":429,"status":400}),
            5 => json!({"code":529,"status":400}),
            6 => json!({"type":"rate_limit_error","code":"insufficient_quota"}),
            7 => json!({"type":"rate_limit_error"}),
            _ => unreachable!(),
        }
    }
    fn body(self) -> String {
        let mut selected = self.selected();
        selected["message"] = "CANARY_PROMPT CANARY_TOKEN https://CANARY_URL".into();
        let ignored = json!({"type":"insufficient_quota","message":"CANARY_IGNORED"});
        let value = match self.shape {
            0 => json!({"type":"error","error":selected,"response":{"error":ignored}}),
            1 => json!({"type":"response.failed","response":{"error":selected},"error":ignored}),
            2 => {
                // Root error event requires type:error, so metadata declarations
                // in root are represented by code only; mixed type cases use
                // direct/nested shapes below.
                selected["type"] = "error".into();
                selected
            }
            _ => unreachable!(),
        };
        format!(
            "data: {value}\n\ndata: {{\"type\":\"response.output_text.delta\",\"delta\":\"CANARY_TAIL\"}}\n\n{}",
            Leaf::Responses.terminal()
        )
    }
    fn expectation(self) -> (u16, Option<ErrorCode>, TerminalEvent) {
        let (status, code) = match self.declaration {
            0 => (401, Some(ErrorCode::AuthenticationError)),
            1 => (401, Some(ErrorCode::RateLimitExceeded)),
            2 => (400, Some(ErrorCode::InvalidRequestError)),
            3 => (400, Some(ErrorCode::RateLimitExceeded)),
            4 => (429, None),
            5 => (529, None),
            6 => (402, Some(ErrorCode::RateLimitExceeded)),
            7 => (429, Some(ErrorCode::RateLimitExceeded)),
            _ => unreachable!(),
        };
        (
            status,
            code,
            if self.shape == 1 {
                TerminalEvent::ResponseFailed
            } else {
                TerminalEvent::Error
            },
        )
    }
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
    let error = result.as_ref().unwrap_err();
    assert_eq!(provider_http_status(error), Some(status));
    assert_eq!(
        classify_provider_error(error),
        ProviderErrorClass::from_status(status)
    );
    assert!(notifications.is_empty(), "no model activity after error");
    assert!(!error.to_string().contains("CANARY"));
    let attempts = trace.attempt_diagnostics();
    assert_eq!(attempts.len(), 1, "one real provider send");
    let actual = &attempts[0];
    assert!(
        !format!("{actual:?}").contains("CANARY"),
        "finite passive/active diagnostics"
    );
    let state = gate.snapshot();
    let throttled = matches!(status, 429 | 529);
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
#[tokio::test]
async fn s1_c123_selected_finite_policy_matrix() {
    let mut wrong = 0;
    let mut selected = 0;
    for shape in 0..3 {
        for declaration in 0..8 {
            // Root type:error cannot also be a different named type. Numeric
            // status-conflict rows explicitly cover all three envelopes.
            if shape == 2 && !matches!(declaration, 4 | 5) {
                continue;
            }
            for leaf in [Leaf::Responses, Leaf::OAuth] {
                for surface in [Surface::Chat, Surface::Assembled, Surface::Incremental] {
                    for admitted in [false, true] {
                        selected += 1;
                        if !row(leaf, surface, admitted, Case { shape, declaration }).await {
                            wrong += 1;
                        }
                    }
                }
            }
        }
    }
    eprintln!(
        "C123 POLICY SUMMARY selected={selected} passed={} failed={wrong}",
        selected - wrong
    );
    assert_eq!(selected, 216);
    assert_eq!(
        wrong, 0,
        "{wrong}/{selected} selected finite policy disagreements"
    );
}
