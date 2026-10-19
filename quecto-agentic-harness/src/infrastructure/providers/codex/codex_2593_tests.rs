//! #2593: a Responses reply cut off at its output limit
//! (`response.incomplete`, `max_output_tokens`) finishes as `MaxTokens`
//! with what it got; another incomplete reason stays a failure.
use super::*;

/// #2593: a reply cut off at its output limit finishes, as `MaxTokens`.
#[tokio::test]
async fn handler_response_incomplete_at_the_output_limit_finishes_the_reply() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let mut handler = CodexSseHandler::new();
    let out = handler
        .process_line(
            r#"data: {"type":"response.incomplete","response":{"status":"incomplete","incomplete_details":{"reason":"max_output_tokens"}}}"#,
            &tx,
        )
        .await;
    assert!(matches!(out, SseLineOutcome::Done));
    match rx.recv().await.unwrap() {
        StreamEvent::Done(response) => {
            assert_eq!(
                response.stop_reason,
                Some(crate::domain::conversation::value_objects::message::StopReason::MaxTokens)
            );
        }
        other => panic!("unexpected: {other:?}"),
    }
}

/// Review of #2598: a call the limit stopped before any argument text
/// finishes with no arguments, which the domain reads as cut off.
#[test]
fn a_call_cut_off_before_its_arguments_finishes_with_none() {
    let sse = r#"data: {"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","call_id":"c1","name":"write","arguments":""}}
data: {"type":"response.incomplete","response":{"status":"incomplete","incomplete_details":{"reason":"max_output_tokens"}}}
"#;
    let response = CodexProvider::parse_sse_response(sse).unwrap();
    assert_eq!(
        response.stop_reason,
        Some(crate::domain::conversation::value_objects::message::StopReason::MaxTokens)
    );
    assert_eq!(response.tool_calls[0].arguments, "");
    let cut = crate::domain::inference::services::output_limit::cut_off_tool_call(&response);
    assert_eq!(cut.map(|c| c.tool), Some("write"));
}

/// An incomplete response that names no reason is no output-limit stop:
/// the allowlist holds, and it stays a stream failure.
#[test]
fn an_incomplete_response_without_a_reason_stays_a_failure() {
    let sse = r#"data: {"type":"response.incomplete","response":{"status":"incomplete"}}
"#;
    let error = CodexProvider::parse_sse_response(sse).unwrap_err();
    assert!(error.to_string().contains("response.incomplete"), "{error}");
}
