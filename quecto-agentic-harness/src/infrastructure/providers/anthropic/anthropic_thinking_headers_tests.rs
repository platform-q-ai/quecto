use super::*;

#[tokio::test]
async fn test_api_key_auth_sends_interleaved_thinking_for_non_46_models() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let response_body = serde_json::json!({
        "id": "msg_ga2",
        "type": "message",
        "role": "assistant",
        "content": [{"type": "text", "text": "ok"}],
        "stop_reason": "end_turn",
        "usage": {"input_tokens": 5, "output_tokens": 2}
    });
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
        .expect(1)
        .mount(&server)
        .await;

    let provider = AnthropicProvider::new("sk-ant-test".to_string(), Some(server.uri()));
    let messages = vec![Message::user("Hi")];
    let req = ChatRequest {
        trace: None,
        admission: None,
        messages: &messages,
        tools: &[],
        // Non-4.6 model: interleaved-thinking should be present
        model: "claude-sonnet-4-5",
        max_tokens: 1024,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    let result = provider.chat(req).await;
    assert!(result.is_ok(), "chat should succeed: {:?}", result);

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let req = &requests[0];
    let beta = req
        .headers
        .get("anthropic-beta")
        .map(|v| v.to_str().unwrap_or(""))
        .unwrap_or("");

    assert!(
        beta.contains("fine-grained-tool-streaming-2025-05-14"),
        "fine-grained-tool-streaming should be present, got: {:?}",
        beta
    );
    assert!(
        beta.contains("interleaved-thinking-2025-05-14"),
        "interleaved-thinking should be present for non-4.6 models, got: {:?}",
        beta
    );
    // Should NOT have identity or oauth betas (API key auth)
    assert!(
        !beta.contains("claude-code"),
        "identity beta should only appear for OAuth, got: {:?}",
        beta
    );
}

// ===========================================================================
