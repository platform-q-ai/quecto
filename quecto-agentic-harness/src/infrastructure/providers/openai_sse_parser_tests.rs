use super::*;

#[test]
fn test_parse_sse_text_response() {
    let sse = "\
data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\
data: {\"choices\":[{\"delta\":{\"content\":\" world\"}}]}\n\
data: [DONE]\n";
    let result = parse_sse_response(sse).unwrap();
    assert_eq!(result.content.as_deref(), Some("Hello world"));
    assert!(result.tool_calls.is_empty());
}

#[test]
fn test_parse_sse_tool_call_response() {
    let sse = "\
data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"bash\",\"arguments\":\"\"}}]}}]}\n\
data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"cmd\\\"\"}}]}}]}\n\
data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\": \\\"ls\\\"}\"}}]}}]}\n\
data: [DONE]\n";
    let result = parse_sse_response(sse).unwrap();
    assert!(result.content.is_none());
    assert_eq!(result.tool_calls.len(), 1);
    assert_eq!(result.tool_calls[0].id, "call_1");
    assert_eq!(result.tool_calls[0].name, "bash");
    assert!(result.tool_calls[0].arguments.contains("ls"));
}

#[test]
fn test_parse_sse_empty() {
    let sse = "data: [DONE]\n";
    let result = parse_sse_response(sse).unwrap();
    assert!(result.content.is_none());
    assert!(result.tool_calls.is_empty());
}

#[test]
fn parse_sse_response_extracts_usage_chunk() {
    let sse = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n",
        "data: {\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5,\"total_tokens\":15,\"prompt_tokens_details\":{\"cached_tokens\":3}}}\n\n",
        "data: [DONE]\n\n",
    );
    let response = parse_sse_response(sse).unwrap();
    assert_eq!(response.content.as_deref(), Some("Hello"));
    let usage = response.usage.expect("usage chunk should be captured");
    assert_eq!(usage.prompt_tokens, 7);
    assert_eq!(usage.completion_tokens, 5);
    assert_eq!(usage.cache_read_tokens, Some(3));
    assert_eq!(usage.context_tokens, Some(10));
}

fn content_sse(fragment: &str) -> String {
    format!(
        "data: {{\"choices\":[{{\"delta\":{{\"content\":{}}}}}]}}\n",
        serde_json::to_string(fragment).unwrap()
    )
}

#[test]
fn parse_sse_content_accepts_exact_limit_and_rejects_over_limit() {
    let exact = "a".repeat(MAX_OPENAI_SSE_CONTENT_BYTES);
    let result = parse_sse_response(&content_sse(&exact)).unwrap();
    assert_eq!(
        result.content.as_ref().unwrap().len(),
        MAX_OPENAI_SSE_CONTENT_BYTES
    );

    let over = format!("{}{}", content_sse(&exact), content_sse("b"));
    let err = parse_sse_response(&over).unwrap_err().to_string();
    assert!(err.contains("assistant content exceeds"));
}

fn tool_args_sse(fragment: &str) -> String {
    format!(
        "data: {{\"choices\":[{{\"delta\":{{\"tool_calls\":[{{\"index\":0,\"id\":\"c\",\"function\":{{\"name\":\"bash\",\"arguments\":{}}}}}]}}}}]}}\n",
        serde_json::to_string(fragment).unwrap()
    )
}

#[test]
fn parse_sse_tool_arguments_accept_exact_limit_and_reject_over_limit() {
    let exact = "a".repeat(MAX_OPENAI_SSE_TOOL_ARGUMENT_BYTES);
    let result = parse_sse_response(&tool_args_sse(&exact)).unwrap();
    assert_eq!(
        result.tool_calls[0].arguments.len(),
        MAX_OPENAI_SSE_TOOL_ARGUMENT_BYTES
    );

    let over = format!("{}{}", tool_args_sse(&exact), tool_args_sse("b"));
    let err = parse_sse_response(&over).unwrap_err().to_string();
    assert!(err.contains("tool-call arguments exceeds"));
}
