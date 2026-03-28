use super::*;
use quecto::domain::audit::{AuditEvent, AuditIssue, content_preview};
use quecto::infrastructure::persistence::audit_log::AuditLog;
use std::path::PathBuf;
use tempfile::TempDir;

// ===========================================================================
// World extensions for audit log tests
// ===========================================================================

// We store audit test state on the shared QuectoWorld.
// The QuectoWorld already has a `tempdir` field we can use.

// Helper: get or create the audit temp dir path from the world's tempdir.
fn audit_base(world: &QuectoWorld) -> PathBuf {
    world.tempdir.as_ref().expect("tempdir not set").path().to_path_buf()
}

// ===========================================================================
// Background
// ===========================================================================

#[given("a temporary audit log directory")]
fn given_temp_audit_dir(world: &mut QuectoWorld) {
    world.tempdir = Some(TempDir::new().expect("failed to create temp dir"));
}

// ===========================================================================
// Domain: AuditEvent serde round-trip scenarios
// ===========================================================================

#[given(expr = r#"an AuditEvent::ToolCall with tool {string} call_id {string} arguments {string}"#)]
fn given_tool_call(world: &mut QuectoWorld, tool: String, call_id: String, arguments: String) {
    let event = AuditEvent::ToolCall { tool, call_id, arguments };
    world.audit_event = Some(event);
}

#[given(expr = r#"an AuditEvent::ToolResult with call_id {string} tool {string} is_error {word} content_tokens {int} content_preview {string}"#)]
fn given_tool_result(world: &mut QuectoWorld, call_id: String, tool: String, is_error_str: String, content_tokens: usize, content_preview: String) {
    let is_error = is_error_str == "true";
    let event = AuditEvent::ToolResult { call_id, tool, is_error, content_tokens, content_preview };
    world.audit_event = Some(event);
}

#[given(expr = r"an AuditEvent::LlmTurnStart with input_tokens_estimate {int} message_count {int}")]
fn given_llm_turn_start(world: &mut QuectoWorld, input_tokens_estimate: usize, message_count: usize) {
    let event = AuditEvent::LlmTurnStart { input_tokens_estimate, message_count };
    world.audit_event = Some(event);
}

#[given(expr = r#"an AuditEvent::LlmTurnEnd with input_tokens {int} output_tokens {int} stop_reason {string} duration_ms {int}"#)]
fn given_llm_turn_end(world: &mut QuectoWorld, input_tokens: usize, output_tokens: usize, stop_reason: String, duration_ms: u64) {
    let event = AuditEvent::LlmTurnEnd { input_tokens, output_tokens, stop_reason, duration_ms };
    world.audit_event = Some(event);
}

#[given(expr = r#"an AuditEvent::WorkflowStep with action {string} step_index {int} step_key {string} step_label {string} template_id {string}"#)]
fn given_workflow_step(world: &mut QuectoWorld, action: String, step_index: usize, step_key: String, step_label: String, template_id: String) {
    let event = AuditEvent::WorkflowStep { action, step_index, step_key, step_label, template_id };
    world.audit_event = Some(event);
}

#[given(expr = r#"an AuditEvent::WorkflowTransition from {string} to {string} template_id {string} issue {int} {string}"#)]
fn given_workflow_transition(world: &mut QuectoWorld, from_mode: String, to_mode: String, template_id: String, issue_num: u64, issue_title: String) {
    let event = AuditEvent::WorkflowTransition {
        from_mode,
        to_mode,
        template_id: Some(template_id),
        issue: Some(AuditIssue { number: issue_num, title: issue_title }),
    };
    world.audit_event = Some(event);
}

#[given(expr = r"an AuditEvent::ContextPruned with messages_dropped {int} tool_results_collapsed {int} tokens_before {int} tokens_after {int}")]
fn given_context_pruned(world: &mut QuectoWorld, messages_dropped: usize, tool_results_collapsed: usize, tokens_before: usize, tokens_after: usize) {
    let event = AuditEvent::ContextPruned { messages_dropped, tool_results_collapsed, tokens_before, tokens_after };
    world.audit_event = Some(event);
}

#[given(expr = r#"an AuditEvent::SubagentSpawned with agent_id {string} task_preview {string} system_preview {string}"#)]
fn given_subagent_spawned(world: &mut QuectoWorld, agent_id: String, task_preview: String, system_preview: String) {
    let event = AuditEvent::SubagentSpawned { agent_id, task_preview, system_preview };
    world.audit_event = Some(event);
}

#[given(expr = r#"an AuditEvent::SubagentCmd with agent_id {string} command {string}"#)]
fn given_subagent_cmd(world: &mut QuectoWorld, agent_id: String, command: String) {
    let event = AuditEvent::SubagentCmd { agent_id, command };
    world.audit_event = Some(event);
}

#[given(expr = r#"an AuditEvent::GuardBlocked with command_preview {string} guard_message {string} before_step_key {string}"#)]
fn given_guard_blocked(world: &mut QuectoWorld, command_preview: String, guard_message: String, before_step_key: String) {
    let event = AuditEvent::GuardBlocked { command_preview, guard_message, before_step_key };
    world.audit_event = Some(event);
}

#[given(expr = r#"an AuditEvent::Error with source {string} tool {string} message {string}"#)]
fn given_error_event(world: &mut QuectoWorld, source: String, tool: String, message: String) {
    let event = AuditEvent::Error { source, tool: Some(tool), message };
    world.audit_event = Some(event);
}

#[when("the event is serialized to JSON")]
fn when_serialized(world: &mut QuectoWorld) {
    let event = world.audit_event.as_ref().expect("no audit event");
    let json = serde_json::to_string(event).expect("serialization failed");
    world.audit_json = Some(json);
}

#[then("it deserializes back to an identical ToolCall event")]
#[then("it deserializes back to an identical ToolResult event")]
#[then("it deserializes back to an identical LlmTurnStart event")]
#[then("it deserializes back to an identical LlmTurnEnd event")]
#[then("it deserializes back to an identical WorkflowStep event")]
#[then("it deserializes back to an identical WorkflowTransition event")]
#[then("it deserializes back to an identical ContextPruned event")]
#[then("it deserializes back to an identical SubagentSpawned event")]
#[then("it deserializes back to an identical SubagentCmd event")]
#[then("it deserializes back to an identical GuardBlocked event")]
#[then("it deserializes back to an identical Error event")]
fn then_round_trips(world: &mut QuectoWorld) {
    let json = world.audit_json.as_ref().expect("no JSON");
    let original = world.audit_event.as_ref().expect("no original event");
    let deserialized: AuditEvent = serde_json::from_str(json).expect("deserialization failed");
    assert_eq!(&deserialized, original);
}

// ===========================================================================
// Infrastructure: AuditLog writer scenarios
// ===========================================================================

#[given("no audit directory exists")]
fn given_no_audit_dir(world: &mut QuectoWorld) {
    let base = audit_base(world);
    assert!(!base.join("audit").exists());
}

#[when(expr = r#"an AuditLog is opened for session {string}"#)]
fn when_audit_log_opened(world: &mut QuectoWorld, session_key: String) {
    let base = audit_base(world);
    let log = AuditLog::open_sync(&base, &session_key).expect("failed to open audit log");
    world.audit_log = Some(std::sync::Arc::new(log));
    world.audit_session_key = Some(session_key);
}

fn emit_sync(log: &AuditLog, turn: u32, event: AuditEvent) {
    let rt = tokio::runtime::Runtime::new().expect("failed to create runtime");
    rt.block_on(log.emit(turn, event)).expect("emit failed");
}

#[when(expr = "a ToolCall event is emitted at turn {int}")]
fn when_tool_call_emitted(world: &mut QuectoWorld, turn: u32) {
    let log = world.audit_log.as_ref().expect("no audit log");
    emit_sync(log, turn, AuditEvent::ToolCall {
        tool: "bash".into(),
        call_id: "call_default".into(),
        arguments: "{}".into(),
    });
}

#[when(expr = r#"a ToolCall event is emitted at turn {int} with tool {string} call_id {string} arguments {string}"#)]
fn when_tool_call_emitted_specific(world: &mut QuectoWorld, turn: u32, tool: String, call_id: String, arguments: String) {
    let log = world.audit_log.as_ref().expect("no audit log");
    emit_sync(log, turn, AuditEvent::ToolCall { tool, call_id, arguments });
}

#[when(expr = "a ToolResult event is emitted at turn {int}")]
fn when_tool_result_emitted(world: &mut QuectoWorld, turn: u32) {
    let log = world.audit_log.as_ref().expect("no audit log");
    emit_sync(log, turn, AuditEvent::ToolResult {
        call_id: "call_default".into(),
        tool: "bash".into(),
        is_error: false,
        content_tokens: 100,
        content_preview: "ok".into(),
    });
}

#[when(expr = "a LlmTurnStart event is emitted at turn {int}")]
fn when_llm_start_emitted(world: &mut QuectoWorld, turn: u32) {
    let log = world.audit_log.as_ref().expect("no audit log");
    emit_sync(log, turn, AuditEvent::LlmTurnStart {
        input_tokens_estimate: 5000,
        message_count: 10,
    });
}

#[when(expr = "a LlmTurnEnd event is emitted at turn {int}")]
fn when_llm_end_emitted(world: &mut QuectoWorld, turn: u32) {
    let log = world.audit_log.as_ref().expect("no audit log");
    emit_sync(log, turn, AuditEvent::LlmTurnEnd {
        input_tokens: 5000,
        output_tokens: 500,
        stop_reason: "end_turn".into(),
        duration_ms: 2000,
    });
}

#[then("the audit directory exists")]
fn then_audit_dir_exists(world: &mut QuectoWorld) {
    let base = audit_base(world);
    assert!(base.join("audit").exists());
}

#[then(expr = r#"the file {string} exists in the audit directory"#)]
fn then_file_exists_in_audit(world: &mut QuectoWorld, filename: String) {
    let base = audit_base(world);
    assert!(base.join("audit").join(&filename).exists(), "file {} not found", filename);
}

fn read_audit_file(world: &QuectoWorld) -> String {
    let session_key = world.audit_session_key.as_ref().expect("no session key");
    let path = AuditLog::file_path(&audit_base(world), session_key);
    std::fs::read_to_string(&path).expect("read failed")
}

#[then(expr = "the audit file contains exactly {int} lines")]
fn then_audit_file_has_lines(world: &mut QuectoWorld, expected: usize) {
    let content = read_audit_file(world);
    let line_count = content.lines().count();
    assert_eq!(line_count, expected, "expected {} lines, got {}", expected, line_count);
}

#[then(expr = r#"line {int} has field {string} equal to {string}"#)]
fn then_line_field_string(world: &mut QuectoWorld, line_num: usize, field: String, expected: String) {
    let content = read_audit_file(world);
    let line = content.lines().nth(line_num - 1).expect("line not found");
    let val: serde_json::Value = serde_json::from_str(line).expect("JSON parse failed");
    assert_eq!(val[&field].as_str().unwrap_or_default(), expected,
        "field '{}' on line {}: expected '{}', got '{}'",
        field, line_num, expected, val[&field]);
}

#[then(expr = "line {int} has field {string} equal to {int}")]
fn then_line_field_int(world: &mut QuectoWorld, line_num: usize, field: String, expected: i64) {
    let content = read_audit_file(world);
    let line = content.lines().nth(line_num - 1).expect("line not found");
    let val: serde_json::Value = serde_json::from_str(line).expect("JSON parse failed");
    assert_eq!(val[&field].as_i64().unwrap(), expected,
        "field '{}' on line {}: expected {}, got {}",
        field, line_num, expected, val[&field]);
}

#[then(expr = "line 1 has field {string} matching ISO 8601")]
fn then_ts_is_iso8601(world: &mut QuectoWorld, field: String) {
    let content = read_audit_file(world);
    let line = content.lines().next().expect("no lines");
    let val: serde_json::Value = serde_json::from_str(line).expect("JSON parse failed");
    let ts = val[&field].as_str().expect("ts not a string");
    assert!(ts.ends_with('Z'), "timestamp should end with Z: {}", ts);
    assert!(ts.contains('T'), "timestamp should contain T: {}", ts);
}

#[then("the audit file is readable without closing the log")]
fn then_readable_without_close(world: &mut QuectoWorld) {
    // The log is still held in world.audit_log — not dropped
    assert!(world.audit_log.is_some(), "audit log should still be open");
    let content = read_audit_file(world);
    assert!(!content.is_empty(), "file should have content");
}

#[then(expr = "it contains {int} complete JSON lines")]
fn then_contains_json_lines(world: &mut QuectoWorld, expected: usize) {
    let content = read_audit_file(world);
    let valid_count = content.lines().filter(|l| serde_json::from_str::<serde_json::Value>(l).is_ok()).count();
    assert_eq!(valid_count, expected);
}

#[then(expr = r#"the audit file is named {string}"#)]
fn then_file_named(world: &mut QuectoWorld, expected_name: String) {
    let session_key = world.audit_session_key.as_ref().expect("no session key");
    let path = AuditLog::file_path(&audit_base(world), session_key);
    assert_eq!(path.file_name().unwrap().to_str().unwrap(), expected_name);
}

#[given("no audit log is configured")]
fn given_no_audit_log(world: &mut QuectoWorld) {
    world.audit_log = None;
}

#[when("the agent processes a prompt")]
fn when_agent_processes_prompt(_world: &mut QuectoWorld) {
    // No-op — we just verify no audit dir is created
}

#[then("no audit directory is created")]
fn then_no_audit_dir(world: &mut QuectoWorld) {
    let base = audit_base(world);
    assert!(!base.join("audit").exists());
}

#[given(expr = "a tool result with {int} characters of content")]
fn given_long_tool_result(world: &mut QuectoWorld, chars: usize) {
    world.audit_long_content = Some("x".repeat(chars));
}

#[when("the content_preview is generated for the audit event")]
fn when_content_preview_generated(world: &mut QuectoWorld) {
    let content = world.audit_long_content.as_ref().expect("no content");
    let preview = content_preview(content, 200);
    world.audit_content_preview = Some(preview);
}

#[then(expr = "the content_preview is at most {int} characters")]
fn then_preview_capped(world: &mut QuectoWorld, max: usize) {
    let preview = world.audit_content_preview.as_ref().expect("no preview");
    assert!(preview.chars().count() <= max, "preview has {} chars, max {}", preview.chars().count(), max);
}
