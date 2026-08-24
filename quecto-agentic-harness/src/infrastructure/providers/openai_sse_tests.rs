use super::*;

#[tokio::test]
async fn handler_emits_text_delta_and_done_response() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let mut handler = OpenAiSseHandler::new();

    let outcome = handler
        .process_line(r#"data: {"choices":[{"delta":{"content":"hello"}}]}"#, &tx)
        .await;
    assert!(matches!(outcome, SseLineOutcome::Continue));
    match rx.recv().await.unwrap() {
        StreamEvent::TextDelta(text) => assert_eq!(text, "hello"),
        other => panic!("unexpected event: {other:?}"),
    }

    let outcome = handler.process_line("data: [DONE]", &tx).await;
    assert!(matches!(outcome, SseLineOutcome::Done));
    match rx.recv().await.unwrap() {
        StreamEvent::Done(response) => {
            assert_eq!(response.content.as_deref(), Some("hello"));
            assert!(response.tool_calls.is_empty());
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

#[tokio::test]
async fn handler_emits_explicit_reasoning_as_thinking_without_token_leakage() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let mut handler = OpenAiSseHandler::new();

    let outcome = handler
        .process_line(
            r#"data: {"choices":[{"delta":{"reasoning":"visible rationale","content":"answer"}}]}"#,
            &tx,
        )
        .await;
    assert!(matches!(outcome, SseLineOutcome::Continue));
    match rx.recv().await.unwrap() {
        StreamEvent::ThinkingDelta(text) => assert_eq!(text, "visible rationale"),
        other => panic!("unexpected event: {other:?}"),
    }
    match rx.recv().await.unwrap() {
        StreamEvent::TextDelta(text) => assert_eq!(text, "answer"),
        other => panic!("unexpected event: {other:?}"),
    }
    handler.on_eof(&tx).await;
    match rx.recv().await.unwrap() {
        StreamEvent::Done(response) => {
            assert_eq!(response.content.as_deref(), Some("answer"));
            assert_eq!(response.thinking_blocks.len(), 1);
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

#[tokio::test]
async fn handler_ignores_non_data_and_malformed_json_then_finishes_on_eof() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(2);
    let mut handler = OpenAiSseHandler::new();

    assert!(matches!(
        handler.process_line(": keepalive", &tx).await,
        SseLineOutcome::Continue
    ));
    assert!(matches!(
        handler.process_line("data: not-json", &tx).await,
        SseLineOutcome::Continue
    ));
    assert!(rx.try_recv().is_err());

    handler.on_eof(&tx).await;
    match rx.recv().await.unwrap() {
        StreamEvent::Done(response) => assert!(response.content.is_none()),
        other => panic!("unexpected event: {other:?}"),
    }
}

#[tokio::test]
async fn handler_captures_usage_chunk_into_response() {
    // With stream_options.include_usage, OpenAI-compatible providers emit
    // a final chunk with empty choices and a populated usage object.
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let mut handler = OpenAiSseHandler::with_model("gpt-5.6-luna");

    handler
        .process_line(r#"data: {"choices":[{"delta":{"content":"hi"}}]}"#, &tx)
        .await;
    let _ = rx.recv().await; // TextDelta

    let outcome = handler
            .process_line(
                r#"data: {"choices":[],"usage":{"prompt_tokens":1234,"completion_tokens":56,"total_tokens":1290,"prompt_tokens_details":{"cached_tokens":34}}}"#,
                &tx,
            )
            .await;
    assert!(matches!(outcome, SseLineOutcome::Continue));

    handler.on_eof(&tx).await;
    match rx.recv().await.unwrap() {
        StreamEvent::Done(response) => {
            let usage = response.usage.expect("usage should be captured");
            assert_eq!(usage.prompt_tokens, 1200);
            assert_eq!(usage.completion_tokens, 56);
            assert_eq!(usage.cache_read_tokens, Some(34));
            assert_eq!(usage.context_tokens, Some(1234));
            assert_eq!(usage.cost.expect("stream cost").total_cost_micro_usd, 1539);
            assert_eq!(response.content.as_deref(), Some("hi"));
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

#[tokio::test]
async fn handler_rejects_over_limit_content_without_done() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let mut handler = OpenAiSseHandler::new();
    let exact = "a".repeat(MAX_OPENAI_SSE_CONTENT_BYTES);
    handler
        .process_line(
            &format!(
                "data: {{\"choices\":[{{\"delta\":{{\"content\":{}}}}}]}}",
                serde_json::to_string(&exact).unwrap()
            ),
            &tx,
        )
        .await;
    assert!(matches!(
        rx.recv().await.unwrap(),
        StreamEvent::TextDelta(_)
    ));

    let outcome = handler
        .process_line(r#"data: {"choices":[{"delta":{"content":"b"}}]}"#, &tx)
        .await;
    assert!(matches!(outcome, SseLineOutcome::Done));
    match rx.recv().await.unwrap() {
        StreamEvent::Error(err) => assert!(err.contains("assistant content exceeds")),
        other => panic!("unexpected event: {other:?}"),
    }
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn handler_rejects_over_limit_tool_arguments_without_done() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(2);
    let mut handler = OpenAiSseHandler::new();
    let exact = "a".repeat(super::super::openai_sse_parser::MAX_OPENAI_SSE_TOOL_ARGUMENT_BYTES);
    handler.process_line(&format!("data: {{\"choices\":[{{\"delta\":{{\"tool_calls\":[{{\"index\":0,\"id\":\"c\",\"function\":{{\"name\":\"bash\",\"arguments\":{}}}}}]}}}}]}}", serde_json::to_string(&exact).unwrap()), &tx).await;
    assert!(rx.try_recv().is_err());

    let outcome = handler.process_line(r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"b"}}]}}]}"#, &tx).await;
    assert!(matches!(outcome, SseLineOutcome::Done));
    match rx.recv().await.unwrap() {
        StreamEvent::Error(err) => assert!(err.contains("tool-call arguments exceeds")),
        other => panic!("unexpected event: {other:?}"),
    }
}

#[tokio::test]
async fn handler_rejects_over_limit_reasoning_without_done() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let mut handler = OpenAiSseHandler::new();
    let exact = "a".repeat(MAX_OPENAI_SSE_CONTENT_BYTES);
    handler
        .process_line(
            &format!(
                "data: {{\"choices\":[{{\"delta\":{{\"reasoning\":{}}}}}]}}",
                serde_json::to_string(&exact).unwrap()
            ),
            &tx,
        )
        .await;
    let _ = rx.recv().await;
    let outcome = handler
        .process_line(r#"data: {"choices":[{"delta":{"reasoning":"b"}}]}"#, &tx)
        .await;
    assert!(matches!(outcome, SseLineOutcome::Done));
    match rx.recv().await.unwrap() {
        StreamEvent::Error(err) => assert!(err.contains("reasoning")),
        other => panic!("unexpected event: {other:?}"),
    }
    assert!(rx.try_recv().is_err());
}
