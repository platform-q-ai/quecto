//! #2151: a provider with no admission binding still records its attempts,
//! with the time to its first token.
use std::sync::Arc;
use std::time::Duration;

use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::domain::request_observation::RequestTrace;

#[tokio::test]
async fn an_unbound_provider_records_attempts_and_time_to_first_token() {
    let server = MockServer::start().await;
    let chunk = r#"{"choices":[{"index":0,"delta":{"content":"hi"},"finish_reason":"stop"}]}"#;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(
                    format!("data: {chunk}\n\ndata: [DONE]\n\n"),
                    "text/event-stream",
                )
                .set_delay(Duration::from_millis(150)),
        )
        .mount(&server)
        .await;
    let provider = super::create_provider_with_client(
        "openai",
        "sk-test".into(),
        Some(server.uri()),
        reqwest::Client::new(),
    )
    .unwrap();
    let trace = Arc::new(RequestTrace::default());
    trace.start();
    let messages = vec![crate::domain::message::Message::user("hi")];
    let request = crate::application::providers::ports::ChatRequest {
        trace: Some(trace.clone()),
        admission: None,
        model: "gpt-4o-mini",
        messages: &messages,
        tools: &[],
        max_tokens: 16,
        temperature: 0.0,
        thinking_level: None,
        effort: None,
        tool_choice: None,
        metadata: None,
        session_id: None,
        cancel_flag: None,
    };
    let mut rx = provider.chat_stream_incremental(request).await;
    while rx.recv().await.is_some() {}
    let attempts = trace.attempt_diagnostics();
    assert_eq!(attempts.len(), 1, "{attempts:?}");
    assert!(attempts[0].generated_text, "{attempts:?}");
    let first = attempts[0].first_token_ms.expect("time to first token");
    assert!(first >= 150, "{first}");
    assert!(
        trace.first_token().is_some(),
        "the request's first token is marked"
    );
    assert!(attempts[0].elapsed_ms >= first, "{attempts:?}");
}

async fn served(
    body: &str,
) -> (
    MockServer,
    Arc<dyn crate::application::providers::ports::LlmProvider>,
) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body.to_owned(), "text/event-stream"))
        .mount(&server)
        .await;
    let provider = super::create_provider_with_client(
        "openai",
        "sk-test".into(),
        Some(server.uri()),
        reqwest::Client::new(),
    )
    .unwrap();
    (server, provider)
}

/// Every path is observed: a whole reply at once (`chat`, no first token
/// to time) and a streamed one assembled (`chat_stream`).
#[tokio::test]
async fn an_unbound_providers_chat_and_chat_stream_are_observed() {
    let whole = r#"{"id":"x","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"hi"},"finish_reason":"stop"}]}"#;
    let chunk = r#"{"choices":[{"index":0,"delta":{"content":"hi"},"finish_reason":"stop"}]}"#;
    for (body, streamed) in [
        (whole.to_owned(), false),
        (format!("data: {chunk}\n\ndata: [DONE]\n\n"), true),
    ] {
        let (_server, provider) = served(&body).await;
        let trace = Arc::new(RequestTrace::default());
        trace.start();
        let messages = vec![crate::domain::message::Message::user("hi")];
        let request = crate::application::providers::ports::ChatRequest {
            trace: Some(trace.clone()),
            admission: None,
            model: "gpt-4o-mini",
            messages: &messages,
            tools: &[],
            max_tokens: 16,
            temperature: 0.0,
            thinking_level: None,
            effort: None,
            tool_choice: None,
            metadata: None,
            session_id: None,
            cancel_flag: None,
        };
        match streamed {
            true => provider.chat_stream(request).await.unwrap(),
            false => provider.chat(request).await.unwrap(),
        };
        let attempts = trace.attempt_diagnostics();
        assert_eq!(attempts.len(), 1, "{streamed}: {attempts:?}");
        assert_eq!(attempts[0].wire_status, Some(200));
        assert_eq!(
            attempts[0].first_token_ms.is_some(),
            streamed,
            "{attempts:?}"
        );
    }
}

/// A reasoning model's first token is its first reasoning delta.
#[tokio::test]
async fn a_reasoning_delta_is_a_first_token() {
    let chunk =
        r#"{"choices":[{"index":0,"delta":{"reasoning_content":"think"},"finish_reason":null}]}"#;
    let (_server, provider) = served(&format!("data: {chunk}\n\ndata: [DONE]\n\n")).await;
    let trace = Arc::new(RequestTrace::default());
    trace.start();
    let messages = vec![crate::domain::message::Message::user("hi")];
    let request = crate::application::providers::ports::ChatRequest {
        trace: Some(trace.clone()),
        admission: None,
        model: "gpt-4o-mini",
        messages: &messages,
        tools: &[],
        max_tokens: 16,
        temperature: 0.0,
        thinking_level: None,
        effort: None,
        tool_choice: None,
        metadata: None,
        session_id: None,
        cancel_flag: None,
    };
    let mut rx = provider.chat_stream_incremental(request).await;
    while rx.recv().await.is_some() {}
    let attempts = trace.attempt_diagnostics();
    assert!(attempts[0].generated_thinking, "{attempts:?}");
    assert!(attempts[0].first_token_ms.is_some(), "{attempts:?}");
}

async fn incremental(
    provider: &Arc<dyn crate::application::providers::ports::LlmProvider>,
) -> (
    Vec<crate::domain::provider::StreamEvent>,
    Vec<crate::domain::attempt_diagnostics::AttemptDiagnostics>,
) {
    let trace = Arc::new(RequestTrace::default());
    trace.start();
    let messages = vec![crate::domain::message::Message::user("hi")];
    let request = crate::application::providers::ports::ChatRequest {
        trace: Some(trace.clone()),
        admission: None,
        model: "gpt-4o-mini",
        messages: &messages,
        tools: &[],
        max_tokens: 16,
        temperature: 0.0,
        thinking_level: None,
        effort: None,
        tool_choice: None,
        metadata: None,
        session_id: None,
        cancel_flag: None,
    };
    let mut rx = provider.chat_stream_incremental(request).await;
    let mut events = Vec::new();
    while let Some(event) = rx.recv().await {
        events.push(event);
    }
    (events, trace.attempt_diagnostics())
}

/// Review #2151: a rate-limit error chunk mid-stream is observed without a
/// permit to report to: no panic, and the old path's handling (its error
/// event) is exactly as before.
#[tokio::test]
async fn a_mid_stream_rate_limit_chunk_is_observed_without_a_permit() {
    let error = r#"{"error":{"type":"rate_limit_exceeded","message":"slow down"}}"#;
    let (_server, provider) = served(&format!("data: {error}\n\n")).await;
    let (events, attempts) = incremental(&provider).await;
    assert_eq!(attempts.len(), 1, "{attempts:?}");
    assert!(
        attempts[0].error_code.is_some() || attempts[0].terminal_event.is_some(),
        "{attempts:?}"
    );
    let before = {
        let (_server, provider) = served(&format!("data: {error}\n\n")).await;
        let messages = vec![crate::domain::message::Message::user("hi")];
        let request = crate::application::providers::ports::ChatRequest {
            trace: None,
            admission: None,
            model: "gpt-4o-mini",
            messages: &messages,
            tools: &[],
            max_tokens: 16,
            temperature: 0.0,
            thinking_level: None,
            effort: None,
            tool_choice: None,
            metadata: None,
            session_id: None,
            cancel_flag: None,
        };
        let mut rx = provider.chat_stream_incremental(request).await;
        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            events.push(event);
        }
        events
    };
    assert_eq!(
        format!("{events:?}"),
        format!("{before:?}"),
        "observation changed handling"
    );
}

/// Each way an attempt ends is recorded: an error status (its body typed),
/// a request that never arrived, a stream ending without `[DONE]`, and one
/// completed.
#[tokio::test]
async fn how_each_attempt_ended_is_recorded() {
    use crate::domain::attempt_diagnostics::Termination;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(500).set_body_string(r#"{"error":{"type":"server_error"}}"#),
        )
        .mount(&server)
        .await;
    let provider = super::create_provider_with_client(
        "openai",
        "sk".into(),
        Some(server.uri()),
        reqwest::Client::new(),
    )
    .unwrap();
    let (_, attempts) = incremental(&provider).await;
    assert_eq!(
        attempts[0].termination,
        Termination::HttpError,
        "{attempts:?}"
    );
    assert_eq!(attempts[0].wire_status, Some(500));

    let unreachable = super::create_provider_with_client(
        "openai",
        "sk".into(),
        Some("http://127.0.0.1:9".into()),
        reqwest::Client::new(),
    )
    .unwrap();
    let (_, attempts) = incremental(&unreachable).await;
    assert_eq!(
        attempts[0].termination,
        Termination::SendError,
        "{attempts:?}"
    );

    let chunk = r#"{"choices":[{"index":0,"delta":{"content":"hi"},"finish_reason":null}]}"#;
    let (_server, provider) = served(&format!("data: {chunk}\n\n")).await;
    let (_, attempts) = incremental(&provider).await;
    assert_eq!(attempts[0].termination, Termination::Eof, "{attempts:?}");

    let (_server, provider) = served(&format!("data: {chunk}\n\ndata: [DONE]\n\n")).await;
    let (_, attempts) = incremental(&provider).await;
    assert_eq!(
        attempts[0].termination,
        Termination::Completed,
        "{attempts:?}"
    );
}

/// A tool call alone is a first token.
#[tokio::test]
async fn a_tool_call_alone_is_a_first_token() {
    let chunk = r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"c","type":"function","function":{"name":"grep","arguments":"{}"}}]},"finish_reason":"tool_calls"}]}"#;
    let (_server, provider) = served(&format!("data: {chunk}\n\ndata: [DONE]\n\n")).await;
    let (_, attempts) = incremental(&provider).await;
    assert!(attempts[0].generated_tool_call, "{attempts:?}");
    assert!(attempts[0].first_token_ms.is_some(), "{attempts:?}");
}

/// Review #2156: a stream the transport cut short, and one whose line
/// outgrew the pump's limit, end as read errors, never as dropped; and the
/// events the caller sees are exactly as before.
#[tokio::test]
async fn a_stream_that_fails_mid_body_ends_as_a_read_error() {
    use crate::domain::attempt_diagnostics::Termination;
    use tokio::io::AsyncWriteExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        // Two connections: the traced request and the untraced one.
        for _ in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 8192];
            let _ = tokio::io::AsyncReadExt::read(&mut socket, &mut request).await;
            let chunk = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hi\"}}]}\n\n";
            let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\
                        transfer-encoding: chunked\r\n\r\n";
            let body = format!("{head}{:x}\r\n{chunk}\r\n", chunk.len());
            socket.write_all(body.as_bytes()).await.unwrap();
            // The connection closes without the terminating chunk.
        }
    });
    let provider = super::create_provider_with_client(
        "openai",
        "sk".into(),
        Some(format!("http://{address}")),
        reqwest::Client::new(),
    )
    .unwrap();
    let (events, attempts) = incremental(&provider).await;
    assert_eq!(attempts.len(), 1, "{attempts:?}");
    assert_eq!(
        attempts[0].termination,
        Termination::ReadError,
        "{attempts:?}"
    );
    assert!(
        matches!(events.last(), Some(crate::domain::provider::StreamEvent::Error(e)) if e.contains("stream read error")),
        "{events:?}"
    );
    assert_eq!(
        format!("{events:?}"),
        format!("{:?}", untraced(&provider).await),
        "observation changed handling"
    );

    let (_server, provider) =
        served(&"x".repeat(crate::infrastructure::providers::sse_common::MAX_SSE_LINE_BYTES + 1))
            .await;
    let (events, attempts) = incremental(&provider).await;
    assert_eq!(
        attempts[0].termination,
        Termination::ReadError,
        "{attempts:?}"
    );
    assert_eq!(attempts[0].oversized_lines, 1, "{attempts:?}");
    assert_eq!(
        format!("{events:?}"),
        format!("{:?}", untraced(&provider).await),
        "observation changed handling"
    );
}

/// Review #2156, the same class: a reply the harness refuses part way (here
/// tool-call arguments over their limit) ends as rejected, never as dropped.
#[tokio::test]
async fn a_reply_refused_mid_stream_ends_as_rejected() {
    use crate::domain::attempt_diagnostics::Termination;
    let piece = "a".repeat(600 * 1024);
    let line = |arguments: &str| {
        format!(
            "data: {}\n\n",
            serde_json::json!({"choices":[{"index":0,"delta":{"tool_calls":[
                {"index":0,"function":{"arguments":arguments}}]}}]})
        )
    };
    let mut body = line("{\"x\":\"");
    for _ in 0..4 {
        body.push_str(&line(&piece));
    }
    body.push_str("data: [DONE]\n\n");
    let (_server, provider) = served(&body).await;
    let (events, attempts) = incremental(&provider).await;
    assert!(
        matches!(
            events.last(),
            Some(crate::domain::provider::StreamEvent::Error(_))
        ),
        "{events:?}"
    );
    assert_eq!(
        attempts[0].termination,
        Termination::Rejected,
        "{attempts:?}"
    );
    assert_eq!(
        format!("{events:?}"),
        format!("{:?}", untraced(&provider).await),
        "observation changed handling"
    );
}

/// The same request with no trace: the old path's events, to compare.
async fn untraced(
    provider: &Arc<dyn crate::application::providers::ports::LlmProvider>,
) -> Vec<crate::domain::provider::StreamEvent> {
    let messages = vec![crate::domain::message::Message::user("hi")];
    let request = crate::application::providers::ports::ChatRequest {
        trace: None,
        admission: None,
        model: "gpt-4o-mini",
        messages: &messages,
        tools: &[],
        max_tokens: 16,
        temperature: 0.0,
        thinking_level: None,
        effort: None,
        tool_choice: None,
        metadata: None,
        session_id: None,
        cancel_flag: None,
    };
    let mut rx = provider.chat_stream_incremental(request).await;
    let mut events = Vec::new();
    while let Some(event) = rx.recv().await {
        events.push(event);
    }
    events
}

/// Review #2156: a whole reply `chat` cannot accept ends as rejected.
#[tokio::test]
async fn a_whole_reply_that_cannot_be_parsed_ends_as_rejected() {
    use crate::domain::attempt_diagnostics::Termination;
    for body in ["not json", r#"{"choices":"not a list"}"#] {
        let (_server, provider) = served(body).await;
        let trace = Arc::new(RequestTrace::default());
        trace.start();
        let messages = vec![crate::domain::message::Message::user("hi")];
        let request = crate::application::providers::ports::ChatRequest {
            trace: Some(trace.clone()),
            admission: None,
            model: "gpt-4o-mini",
            messages: &messages,
            tools: &[],
            max_tokens: 16,
            temperature: 0.0,
            thinking_level: None,
            effort: None,
            tool_choice: None,
            metadata: None,
            session_id: None,
            cancel_flag: None,
        };
        assert!(provider.chat(request).await.is_err(), "{body}");
        let attempts = trace.attempt_diagnostics();
        assert_eq!(
            attempts[0].termination,
            Termination::Rejected,
            "{body}: {attempts:?}"
        );
    }
}
