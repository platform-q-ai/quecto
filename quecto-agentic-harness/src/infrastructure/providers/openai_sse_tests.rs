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
    let mut handler = OpenAiSseHandler::new();

    handler
        .process_line(r#"data: {"choices":[{"delta":{"content":"hi"}}]}"#, &tx)
        .await;
    let _ = rx.recv().await; // TextDelta

    let outcome = handler
            .process_line(
                r#"data: {"choices":[],"usage":{"prompt_tokens":1234,"completion_tokens":56,"total_tokens":1290}}"#,
                &tx,
            )
            .await;
    assert!(matches!(outcome, SseLineOutcome::Continue));

    handler.on_eof(&tx).await;
    match rx.recv().await.unwrap() {
        StreamEvent::Done(response) => {
            let usage = response.usage.expect("usage should be captured");
            assert_eq!(usage.prompt_tokens, 1234);
            assert_eq!(usage.completion_tokens, 56);
            assert_eq!(response.content.as_deref(), Some("hi"));
        }
        other => panic!("unexpected event: {other:?}"),
    }
}
