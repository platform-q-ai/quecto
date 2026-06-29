use super::*;

#[test]
fn intercepts_vanilla_tool_result() {
    let line = r#"{"type":"tool_result","toolCallId":"uds-abc","content":"ok","isError":false}"#;
    let got = try_intercept_tool_result(line).expect("should intercept");
    assert_eq!(got.tool_call_id, "uds-abc");
    assert_eq!(got.content, "ok");
    assert!(!got.is_error);
}

#[test]
fn intercepts_tool_result_with_error_flag() {
    let line = r#"{"type":"tool_result","toolCallId":"uds-xyz","content":"oh no","isError":true}"#;
    let got = try_intercept_tool_result(line).expect("should intercept");
    assert!(got.is_error);
}

#[test]
fn intercepts_tool_result_with_unknown_future_field() {
    // Future-proofing: if `AgentCommand::ToolResult` gains, say, a
    // `metadata` field later, deserialisation keeps working; the
    // bypass path must not be more strict than the canonical one.
    let line = r#"{"type":"tool_result","toolCallId":"uds-1","content":"","isError":false,"metadata":{"elapsed_ms":42}}"#;
    let got = try_intercept_tool_result(line).expect("should intercept");
    assert_eq!(got.tool_call_id, "uds-1");
}

#[test]
fn does_not_intercept_prompt_command() {
    let line = r#"{"type":"prompt","message":"hello"}"#;
    assert!(try_intercept_tool_result(line).is_none());
}

#[test]
fn does_not_intercept_register_tools() {
    let line = r#"{"type":"register_tools","id":"r1","tools":[{"name":"t","description":"d"}]}"#;
    assert!(try_intercept_tool_result(line).is_none());
}

#[test]
fn does_not_intercept_garbage_or_empty() {
    assert!(try_intercept_tool_result("not json").is_none());
    assert!(try_intercept_tool_result("").is_none());
    assert!(try_intercept_tool_result("{}").is_none());
}

#[test]
fn does_not_intercept_line_that_only_mentions_the_literal_in_a_string() {
    // Cheap gate false-positive guard: a prompt whose message body
    // contains the substring `tool_result` should not intercept
    // (the full parse will reject it since the command's `type`
    // is `prompt`).
    let line = r#"{"type":"prompt","message":"explain what tool_result means"}"#;
    assert!(try_intercept_tool_result(line).is_none());
}
