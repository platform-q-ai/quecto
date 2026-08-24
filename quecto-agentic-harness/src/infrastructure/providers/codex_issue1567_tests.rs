use super::*;

#[tokio::test]
async fn test_codex_provider_success() {
    let server = wiremock::MockServer::start().await;
    let sse_body = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hi!\"}\n\
                         data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":10,\"output_tokens\":5,\"input_tokens_details\":{\"cached_tokens\":7}}}}\n\
                         data: [DONE]\n";
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(sse_body))
        .mount(&server)
        .await;

    let provider = CodexProvider::new(
        "test-token".to_string(),
        "acct-123".to_string(),
        Some(server.uri()),
    );
    let messages = vec![Message::system("You are helpful."), Message::user("hello")];
    let result = provider
        .chat(ChatRequest {
            messages: &messages,
            tools: &[],
            model: "gpt-5.6-luna",
            max_tokens: 1024,
            temperature: 0.7,
            session_id: None,
            tool_choice: None,
            metadata: None,
            thinking_level: None,
            cancel_flag: None,
            effort: None,
        })
        .await;
    let resp = result.unwrap();
    assert_eq!(resp.content.as_deref(), Some("Hi!"));
    let usage = resp.usage.expect("usage");
    assert_eq!(usage.prompt_tokens, 3);
    assert_eq!(usage.completion_tokens, 5);
    assert_eq!(usage.cache_read_tokens, Some(7));
    assert_eq!(
        usage.cost.expect("gpt-5.6 pricing").total_cost_micro_usd,
        33
    );
}
