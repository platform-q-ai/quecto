//! #2116: every OpenAI Chat Completions path carries `finish_reason` into
//! `LlmResponse::stop_reason`. One table runs through all three builders —
//! non-streaming bodies, whole SSE bodies and the live streaming handler —
//! so none of them can silently drop it again.
use super::*;
use crate::domain::message::StopReason;

/// `(finish_reason on the wire, expected stop reason)`.
fn cases() -> Vec<(Option<&'static str>, Option<StopReason>)> {
    vec![
        (Some("stop"), Some(StopReason::EndTurn)),
        (Some("length"), Some(StopReason::MaxTokens)),
        (Some("tool_calls"), Some(StopReason::ToolUse)),
        (Some("function_call"), Some(StopReason::ToolUse)),
        (Some("content_filter"), Some(StopReason::Refusal)),
        (
            Some("vendor_specific"),
            Some(StopReason::Unknown("vendor_specific".into())),
        ),
        (None, None),
    ]
}

fn finish(reason: Option<&str>) -> serde_json::Value {
    reason.map_or(serde_json::Value::Null, |r| serde_json::json!(r))
}

#[test]
fn non_streaming_bodies_carry_finish_reason() {
    for (reason, expected) in cases() {
        let body = serde_json::json!({
            "choices": [{"message": {"role": "assistant", "content": "hi"}, "finish_reason": finish(reason)}]
        });
        let response = OpenAiProvider::parse_response(&body).unwrap();
        assert_eq!(response.stop_reason, expected, "{reason:?}");
    }
}

#[test]
fn whole_sse_bodies_carry_finish_reason_from_any_chunk() {
    for (reason, expected) in cases() {
        // Content first, then the finish reason on its own empty-delta
        // chunk, then a usage-only chunk that must not reset it.
        let sse = format!(
            "data: {}\n\ndata: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
            serde_json::json!({"choices": [{"delta": {"content": "hi"}, "finish_reason": null}]}),
            serde_json::json!({"choices": [{"delta": {}, "finish_reason": finish(reason)}]}),
            serde_json::json!({"choices": [], "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}}),
        );
        let response = super::super::openai_sse_parser::parse_sse_response(&sse).unwrap();
        assert_eq!(response.stop_reason, expected, "{reason:?}");
        assert_eq!(response.content.as_deref(), Some("hi"));
    }
}

#[tokio::test]
async fn the_streaming_handler_carries_finish_reason_to_done_and_eof() {
    for (reason, expected) in cases() {
        for ends_with_done in [true, false] {
            let (tx, mut rx) = tokio::sync::mpsc::channel(16);
            let mut handler = OpenAiSseHandler::new();
            for chunk in [
                serde_json::json!({"choices": [{"delta": {"content": "hi"}, "finish_reason": null}]}),
                serde_json::json!({"choices": [{"delta": {}, "finish_reason": finish(reason)}]}),
                serde_json::json!({"choices": [], "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}}),
            ] {
                handler.process_line(&format!("data: {chunk}"), &tx).await;
            }
            if ends_with_done {
                handler.process_line("data: [DONE]", &tx).await;
            } else {
                handler.on_eof(&tx).await;
            }
            drop(tx);
            let mut done = None;
            while let Some(event) = rx.recv().await {
                if let StreamEvent::Done(response) = event {
                    done = Some(response);
                }
            }
            let response = done.expect("a Done response");
            assert_eq!(
                response.stop_reason, expected,
                "{reason:?}, [DONE]: {ends_with_done}"
            );
        }
    }
}

#[test]
fn a_chunk_without_a_finish_reason_does_not_erase_an_earlier_one() {
    let sse = format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        serde_json::json!({"choices": [{"delta": {"content": "hi"}, "finish_reason": "length"}]}),
        serde_json::json!({"choices": [{"delta": {}}]}),
    );
    let response = super::super::openai_sse_parser::parse_sse_response(&sse).unwrap();
    assert_eq!(response.stop_reason, Some(StopReason::MaxTokens));
}
