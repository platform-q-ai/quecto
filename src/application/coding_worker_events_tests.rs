use super::*;

#[test]
fn test_build_tool_start_minimal() {
    let payload = build_tool_start(ToolStartInput {
        tool: "read_file".into(),
        call_id: "c1".into(),
        args_preview: None,
    });
    let json = payload_to_json(&payload);
    assert_eq!(json["tool"], "read_file");
    assert_eq!(json["call_id"], "c1");
    assert!(json.get("args_preview").is_none());
}

#[test]
fn test_build_tool_start_with_preview() {
    let payload = build_tool_start(ToolStartInput {
        tool: "edit_file".into(),
        call_id: "c4".into(),
        args_preview: Some("src/parser.rs".into()),
    });
    let json = payload_to_json(&payload);
    assert_eq!(json["tool"], "edit_file");
    let preview = json["args_preview"].as_str().unwrap();
    assert!(preview.contains("src/parser.rs"));
}

#[test]
fn test_build_tool_result_success() {
    let payload = build_tool_result(ToolResultInput {
        tool: "read_file".into(),
        call_id: "c1".into(),
        ok: true,
        duration_ms: Some(12),
        diff_ref: None,
        stderr_ref: None,
        stdout_ref: None,
        truncated: None,
    });
    let json = payload_to_json(&payload);
    assert_eq!(json["ok"], true);
    assert_eq!(json["duration_ms"], 12);
}

#[test]
fn test_build_tool_result_failure_with_stderr() {
    let payload = build_tool_result(ToolResultInput {
        tool: "exec".into(),
        call_id: "c2".into(),
        ok: false,
        duration_ms: None,
        diff_ref: None,
        stderr_ref: Some("artifact:stderr-c2".into()),
        stdout_ref: None,
        truncated: None,
    });
    let json = payload_to_json(&payload);
    assert_eq!(json["ok"], false);
    let s = json["stderr_ref"].as_str().unwrap();
    assert!(s.starts_with("artifact:"));
}

#[test]
fn test_build_tool_result_with_diff_ref() {
    let payload = build_tool_result(ToolResultInput {
        tool: "edit_file".into(),
        call_id: "c3".into(),
        ok: true,
        duration_ms: None,
        diff_ref: Some("artifact:diff-c3".into()),
        stderr_ref: None,
        stdout_ref: None,
        truncated: None,
    });
    let json = payload_to_json(&payload);
    assert!(json.get("diff_ref").is_some());
}

#[test]
fn test_build_tool_result_truncated() {
    let payload = build_tool_result(ToolResultInput {
        tool: "exec".into(),
        call_id: "c10".into(),
        ok: true,
        duration_ms: None,
        diff_ref: None,
        stderr_ref: None,
        stdout_ref: Some("artifact:stdout-c10".into()),
        truncated: Some(true),
    });
    let json = payload_to_json(&payload);
    assert_eq!(json["truncated"], true);
    let s = json["stdout_ref"].as_str().unwrap();
    assert!(s.starts_with("artifact:"));
}

#[test]
fn test_build_tool_result_with_stdout_ref() {
    let payload = build_tool_result(ToolResultInput {
        tool: "exec".into(),
        call_id: "c5".into(),
        ok: true,
        duration_ms: None,
        diff_ref: None,
        stderr_ref: None,
        stdout_ref: Some("artifact:stdout-c5".into()),
        truncated: None,
    });
    let json = payload_to_json(&payload);
    let s = json["stdout_ref"].as_str().unwrap();
    assert!(s.starts_with("artifact:"));
}

#[test]
fn test_build_artifact_patch() {
    let payload = build_artifact(ArtifactInput {
        artifact_id: "artifact:patch-1".into(),
        artifact_type: "patch".into(),
        path: "artifacts/patch.diff".into(),
        size_bytes: None,
        description: None,
    });
    let json = payload_to_json(&payload);
    assert_eq!(json["artifact_id"], "artifact:patch-1");
    assert_eq!(json["artifact_type"], "patch");
    assert!(json.get("path").is_some());
}

#[test]
fn test_build_artifact_with_size() {
    let payload = build_artifact(ArtifactInput {
        artifact_id: "artifact:test-1".into(),
        artifact_type: "test_output".into(),
        path: "artifacts/test.log".into(),
        size_bytes: Some(2048),
        description: None,
    });
    let json = payload_to_json(&payload);
    assert_eq!(json["size_bytes"], 2048);
}

#[test]
fn test_build_artifact_with_description() {
    let payload = build_artifact(ArtifactInput {
        artifact_id: "artifact:desc-1".into(),
        artifact_type: "patch".into(),
        path: "artifacts/patch.diff".into(),
        size_bytes: None,
        description: Some("final patch for parser refactor".into()),
    });
    let json = payload_to_json(&payload);
    assert_eq!(json["description"], "final patch for parser refactor");
}

#[test]
fn test_build_log_info() {
    let payload = build_log(LogInput {
        level: "info".into(),
        message: "starting test suite".into(),
        context: None,
    });
    let json = payload_to_json(&payload);
    assert_eq!(json["level"], "info");
    assert_eq!(json["message"], "starting test suite");
}

#[test]
fn test_build_log_with_context() {
    let payload = build_log(LogInput {
        level: "warn".into(),
        message: "warning".into(),
        context: Some(serde_json::json!({"file": "src/parser.rs"})),
    });
    let json = payload_to_json(&payload);
    assert!(json.get("context").is_some());
}

#[test]
fn test_build_log_levels() {
    for level in ["error", "warn", "debug", "info"] {
        let payload = build_log(LogInput {
            level: level.into(),
            message: "test message".into(),
            context: None,
        });
        let json = payload_to_json(&payload);
        assert_eq!(json["level"], level);
    }
}

#[test]
fn test_is_payload_oversized_within_limit() {
    let payload = build_log(LogInput {
        level: "info".into(),
        message: "small".into(),
        context: None,
    });
    let (_, oversized) = is_payload_oversized(&payload);
    assert!(!oversized);
}

#[test]
fn test_payload_to_json_strips_type() {
    let payload = build_log(LogInput {
        level: "info".into(),
        message: "test".into(),
        context: None,
    });
    let json = payload_to_json(&payload);
    assert!(json.get("type").is_none());
}
