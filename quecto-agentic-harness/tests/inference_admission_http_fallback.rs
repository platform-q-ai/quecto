//! Candidate HTTP status is typed throttle evidence unless structured terminal
//! fields override it. No display-text classification or leaf escalation.
use quecto::application::inference_attempt::{AttemptAdmission, AttemptPermit};
use quecto::domain::{
    error::DomainError,
    inference_admission::{Feedback, ThrottleFeedback},
    provider::{ChatRequest, LlmProvider},
};
use quecto::infrastructure::providers::openai::OpenAiProvider;
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
#[derive(Debug, Default, Clone)]
struct Gate(Arc<Mutex<Vec<ThrottleFeedback>>>);
impl AttemptAdmission for Gate {
    fn acquire(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Box<dyn AttemptPermit>, DomainError>> + Send + '_>>
    {
        Box::pin(async { Ok(Box::new(self.clone()) as Box<dyn AttemptPermit>) })
    }
}
impl AttemptPermit for Gate {
    fn feedback(&mut self, feedback: ThrottleFeedback) {
        self.0.lock().unwrap().push(feedback);
    }
    fn finish(self: Box<Self>, _: Feedback) {}
}
async fn check(status: u16, body: &str, expected: bool) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(status).set_body_string(body))
        .mount(&server)
        .await;
    let gate = Arc::new(Gate::default());
    let provider = OpenAiProvider::new("fixture".into(), Some(server.uri()))
        .with_attempt_admission(
            gate.clone(),
            quecto::infrastructure::providers::SingleAttemptClient::build(
                reqwest::Client::builder().no_proxy(),
            )
            .unwrap(),
        );
    let request = ChatRequest {
        trace: None,
        admission: None,
        model: "fixture",
        messages: &[],
        tools: &[],
        max_tokens: 1,
        temperature: 0.0,
        thinking_level: None,
        effort: None,
        tool_choice: None,
        metadata: None,
        session_id: None,
        cancel_flag: None,
    };
    assert!(provider.chat(request).await.is_err());
    let receipts = gate.0.lock().unwrap();
    assert_eq!(receipts.len(), usize::from(expected));
    if expected {
        assert!(matches!(receipts[0], ThrottleFeedback::NoHint { .. }));
    }
}
#[tokio::test]
async fn opaque_529_uses_authority_fallback() {
    check(529, "busy", true).await;
}
#[tokio::test]
async fn opaque_429_uses_authority_fallback() {
    check(429, "unknown plain", true).await;
}
#[tokio::test]
async fn billing_429_does_not_use_fallback() {
    check(
        429,
        r#"{"error":{"type":"rate_limit_error","code":"insufficient_quota"}}"#,
        false,
    )
    .await;
}
#[tokio::test]
async fn authentication_429_does_not_use_fallback() {
    check(429, r#"{"error":{"type":"authentication_error"}}"#, false).await;
}

#[tokio::test]
async fn assembled_responses_reports_sse_throttle_before_body_eof() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (sent, received) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut headers = Vec::new();
        while !headers.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            socket.read_exact(&mut byte).await.unwrap();
            headers.push(byte[0]);
        }
        let headers = String::from_utf8(headers).unwrap();
        let length: usize = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse().unwrap())
            })
            .unwrap();
        socket.read_exact(&mut vec![0; length]).await.unwrap();
        let body = "data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"type\":\"rate_limit_error\",\"message\":\"opaque\"}}}\n\n";
        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n{body}", body.len()+16).as_bytes()).await.unwrap();
        sent.send(()).unwrap();
        std::future::pending::<()>().await;
    });
    let gate = Arc::new(Gate::default());
    let provider = quecto::infrastructure::providers::codex::CodexProvider::with_api_key(
        "fixture".into(),
        Some(url),
        reqwest::Client::new(),
    )
    .with_attempt_admission(
        gate.clone(),
        quecto::infrastructure::providers::SingleAttemptClient::build(
            reqwest::Client::builder().no_proxy(),
        )
        .unwrap(),
    );
    let call = tokio::spawn(async move {
        let request = ChatRequest {
            trace: None,
            admission: None,
            model: "fixture",
            messages: &[],
            tools: &[],
            max_tokens: 1,
            temperature: 0.0,
            thinking_level: None,
            effort: None,
            tool_choice: None,
            metadata: None,
            session_id: None,
            cancel_flag: None,
        };
        provider.chat(request).await
    });
    received.await.unwrap();
    let observed = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while gate.0.lock().unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await;
    call.abort();
    server.abort();
    let _ = call.await;
    let _ = server.await;
    assert!(
        observed.is_ok(),
        "typed SSE receipt must not wait for HTTP body EOF"
    );
}
