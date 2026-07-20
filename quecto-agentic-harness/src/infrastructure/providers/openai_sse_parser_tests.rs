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
        "data: {\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5,\"total_tokens\":15}}\n\n",
        "data: [DONE]\n\n",
    );
    let response = parse_sse_response(sse).unwrap();
    assert_eq!(response.content.as_deref(), Some("Hello"));
    let usage = response.usage.expect("usage chunk should be captured");
    assert_eq!(usage.prompt_tokens, 10);
    assert_eq!(usage.completion_tokens, 5);
}
