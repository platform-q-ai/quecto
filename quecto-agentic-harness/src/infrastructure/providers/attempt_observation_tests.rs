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
