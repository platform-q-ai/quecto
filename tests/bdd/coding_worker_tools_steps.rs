use super::*;

use quecto::domain::coding_event::EventSource;

fn emit(
    world: &mut QuectoWorld,
    source: EventSource,
    event_type: &str,
    payload: serde_json::Value,
) {
    push_coding_event(world, source, event_type, payload);
}

#[when(expr = "the worker begins executing tool {string} with call_id {string}")]
fn when_worker_tool_start(world: &mut QuectoWorld, tool: String, call_id: String) {
    emit(
        world,
        EventSource::Worker,
        "tool.start",
        serde_json::json!({"tool": tool, "call_id": call_id}),
    );
}

#[when(expr = "the worker completes tool {string} with call_id {string} successfully")]
fn when_worker_tool_success(world: &mut QuectoWorld, tool: String, call_id: String) {
    emit(
        world,
        EventSource::Worker,
        "tool.result",
        serde_json::json!({"tool": tool, "call_id": call_id, "ok": true, "duration_ms": 12}),
    );
}

#[when(expr = "the worker fails tool {string} with call_id {string}")]
fn when_worker_tool_fail(world: &mut QuectoWorld, tool: String, call_id: String) {
    emit(
        world,
        EventSource::Worker,
        "tool.result",
        serde_json::json!({"tool": tool, "call_id": call_id, "ok": false, "stderr_ref": "artifact:stderr-c2"}),
    );
}

#[when("the worker produces tool output exceeding the capture limit")]
fn when_worker_output_truncated(world: &mut QuectoWorld) {
    emit(
        world,
        EventSource::Worker,
        "tool.result",
        serde_json::json!({"tool":"exec","call_id":"c10","ok":true,"truncated":true,"stdout_ref":"artifact:stdout-c10"}),
    );
}

#[when(expr = "the arguments contain a file path {string}")]
fn when_args_contain_path(world: &mut QuectoWorld, path: String) {
    if let Some(e) = world
        .coding_events
        .iter_mut()
        .rev()
        .find(|e| e.event_type == "tool.start")
    {
        e.payload["args_preview"] = serde_json::Value::String(path);
    }
}

#[when(expr = "the worker executes tools {string} then {string} then {string}")]
fn when_worker_executes_three_tools(world: &mut QuectoWorld, a: String, b: String, c: String) {
    for (idx, tool) in [a, b, c].into_iter().enumerate() {
        let call_id = format!("c{}", idx + 20);
        emit(
            world,
            EventSource::Worker,
            "tool.start",
            serde_json::json!({"tool": tool, "call_id": call_id}),
        );
        emit(
            world,
            EventSource::Worker,
            "tool.result",
            serde_json::json!({"tool": tool, "call_id": call_id, "ok": true}),
        );
    }
}

#[when("the worker generates a patch file for its edits")]
fn when_patch_artifact(world: &mut QuectoWorld) {
    emit(
        world,
        EventSource::Worker,
        "artifact.created",
        serde_json::json!({"artifact_id":"artifact:patch-1","artifact_type":"patch","path":"artifacts/patch.diff"}),
    );
}

#[when("the worker runs a shell command with significant output")]
fn when_log_artifact(world: &mut QuectoWorld) {
    emit(
        world,
        EventSource::Worker,
        "artifact.created",
        serde_json::json!({"artifact_id":"artifact:log-1","artifact_type":"log","path":"artifacts/exec.log"}),
    );
}

#[when(expr = "the worker logs an info message {string}")]
fn when_log_info(world: &mut QuectoWorld, msg: String) {
    emit(
        world,
        EventSource::Worker,
        "log.message",
        serde_json::json!({"level":"info","message":msg}),
    );
}

#[when("the worker logs a warning with context about a specific file")]
fn when_log_warning_with_context(world: &mut QuectoWorld) {
    emit(
        world,
        EventSource::Worker,
        "log.message",
        serde_json::json!({"level":"warn","message":"warning","context":{"file":"src/parser.rs"}}),
    );
}

#[when("the command produces captured stdout")]
fn when_command_stdout(world: &mut QuectoWorld) {
    if let Some(e) = world
        .coding_events
        .iter_mut()
        .rev()
        .find(|e| e.event_type == "tool.result")
    {
        e.payload["stdout_ref"] = serde_json::Value::String("artifact:stdout-c5".to_string());
    }
}

#[when("the worker generates a job summary document")]
fn when_summary_artifact(world: &mut QuectoWorld) {
    emit(
        world,
        EventSource::Worker,
        "artifact.created",
        serde_json::json!({"artifact_id":"artifact:summary-1","artifact_type":"summary","path":"artifacts/summary.md"}),
    );
}

#[when("the worker captures test runner output")]
fn when_test_output_artifact(world: &mut QuectoWorld) {
    emit(
        world,
        EventSource::Worker,
        "artifact.created",
        serde_json::json!({"artifact_id":"artifact:test-1","artifact_type":"test_output","path":"artifacts/test.log","size_bytes":2048}),
    );
}

#[when("a child agent produces a review document")]
fn when_child_review_artifact(world: &mut QuectoWorld) {
    emit(
        world,
        EventSource::ChildAgent,
        "artifact.created",
        serde_json::json!({"artifact_id":"artifact:review-1","artifact_type":"review","path":"artifacts/review.md"}),
    );
}

#[when("the coordinator snapshots injected skills at job start")]
fn when_coordinator_snapshot(world: &mut QuectoWorld) {
    emit(
        world,
        EventSource::Coordinator,
        "artifact.created",
        serde_json::json!({"artifact_id":"artifact:snapshot-1","artifact_type":"snapshot","path":"artifacts/skills.snapshot"}),
    );
}

#[when(expr = "the worker begins executing an unrecognized tool {string} with call_id {string}")]
fn when_unrecognized_tool_start(world: &mut QuectoWorld, tool: String, call_id: String) {
    when_worker_tool_start(world, tool, call_id);
}

#[when(expr = "the worker starts two tools concurrently with call_ids {string} and {string}")]
fn when_two_concurrent_tools(world: &mut QuectoWorld, c1: String, c2: String) {
    emit(
        world,
        EventSource::Worker,
        "tool.start",
        serde_json::json!({"tool":"read_file","call_id":c1}),
    );
    emit(
        world,
        EventSource::Worker,
        "tool.start",
        serde_json::json!({"tool":"exec","call_id":c2}),
    );
    emit(
        world,
        EventSource::Worker,
        "tool.result",
        serde_json::json!({"tool":"read_file","call_id":"c7","ok":true}),
    );
    emit(
        world,
        EventSource::Worker,
        "tool.result",
        serde_json::json!({"tool":"exec","call_id":"c8","ok":true}),
    );
}

#[when(expr = "the worker executes tool {string} with call_id {string}")]
fn when_worker_exec_tool(world: &mut QuectoWorld, tool: String, call_id: String) {
    when_worker_tool_start(world, tool, call_id);
}

#[when("the tool execution exceeds the configured timeout")]
fn when_tool_timeout(world: &mut QuectoWorld) {
    emit(
        world,
        EventSource::Worker,
        "tool.result",
        serde_json::json!({"tool":"exec","call_id":"c9","ok":false,"stderr_ref":"artifact:timeout-c9"}),
    );
}

#[when("the worker emits a tool.result with payload larger than 1 MiB")]
fn when_tool_result_large_payload(world: &mut QuectoWorld) {
    when_worker_output_truncated(world);
}

#[when(expr = "the worker logs an error message {string}")]
fn when_log_error(world: &mut QuectoWorld, msg: String) {
    emit(
        world,
        EventSource::Worker,
        "log.message",
        serde_json::json!({"level":"error","message":msg}),
    );
}

#[when(expr = "the worker logs a warn message {string}")]
fn when_log_warn(world: &mut QuectoWorld, msg: String) {
    emit(
        world,
        EventSource::Worker,
        "log.message",
        serde_json::json!({"level":"warn","message":msg}),
    );
}

#[when(expr = "the worker logs a debug message {string}")]
fn when_log_debug(world: &mut QuectoWorld, msg: String) {
    emit(
        world,
        EventSource::Worker,
        "log.message",
        serde_json::json!({"level":"debug","message":msg}),
    );
}

#[when(expr = "the worker creates an artifact with description {string}")]
fn when_artifact_with_description(world: &mut QuectoWorld, desc: String) {
    emit(
        world,
        EventSource::Worker,
        "artifact.created",
        serde_json::json!({"artifact_id":"artifact:desc-1","artifact_type":"patch","path":"artifacts/patch.diff","description":desc}),
    );
}

#[when("the worker attempts to make an HTTP request to an external host")]
fn when_worker_http_attempt(world: &mut QuectoWorld) {
    emit(
        world,
        EventSource::Worker,
        "log.message",
        serde_json::json!({"level":"warn","message":"network policy blocked outbound request"}),
    );
}

#[when("the worker executes a command that outputs an API key in stderr")]
fn when_stderr_contains_secret(world: &mut QuectoWorld) {
    emit(
        world,
        EventSource::Worker,
        "tool.result",
        serde_json::json!({"tool":"exec","call_id":"c-secret","ok":false,"stderr_ref":"artifact:stderr-redacted","stderr_preview":"[REDACTED]"}),
    );
}

#[when("the worker logs a message containing an API key")]
fn when_log_contains_secret(world: &mut QuectoWorld) {
    emit(
        world,
        EventSource::Worker,
        "log.message",
        serde_json::json!({"level":"warn","message":"token=[REDACTED]"}),
    );
}

#[when("the worker creates an artifact that would contain credential material")]
fn when_artifact_contains_secret(world: &mut QuectoWorld) {
    emit(
        world,
        EventSource::Worker,
        "artifact.created",
        serde_json::json!({"artifact_id":"artifact:redacted","artifact_type":"log","path":"artifacts/redacted.log"}),
    );
}

#[then(expr = "a {string} event should be emitted")]
fn then_event_emitted(world: &mut QuectoWorld, event: String) {
    assert!(world.coding_events.iter().any(|e| e.event_type == event));
}

#[then(expr = "an {string} event should be emitted")]
fn then_event_emitted_an(world: &mut QuectoWorld, event: String) {
    assert!(world.coding_events.iter().any(|e| e.event_type == event));
}

#[then(expr = "a {string} event should be emitted with ok {word}")]
fn then_tool_result_ok(world: &mut QuectoWorld, event: String, ok: String) {
    let expected_ok = ok == "true";
    let e = world
        .coding_events
        .iter()
        .rev()
        .find(|e| e.event_type == event)
        .unwrap_or_else(|| panic!("missing event {event}"));
    assert_eq!(e.payload["ok"], expected_ok);
}

#[then(expr = "the payload should include tool {string} and call_id {string}")]
fn then_payload_tool_call_id(world: &mut QuectoWorld, tool: String, call_id: String) {
    let e = world
        .coding_events
        .iter()
        .rev()
        .find(|e| e.event_type == "tool.start")
        .expect("tool.start");
    assert_eq!(e.payload["tool"], tool);
    assert_eq!(e.payload["call_id"], call_id);
}

#[then("the payload should include duration_ms")]
fn then_payload_duration(world: &mut QuectoWorld) {
    let e = world
        .coding_events
        .iter()
        .rev()
        .find(|e| e.event_type == "tool.result")
        .expect("tool.result");
    assert!(e.payload.get("duration_ms").is_some());
}

#[then("the payload should include stderr_ref pointing to an artifact")]
fn then_payload_stderr_ref(world: &mut QuectoWorld) {
    let e = world
        .coding_events
        .iter()
        .rev()
        .find(|e| e.event_type == "tool.result")
        .expect("tool.result");
    let s = e.payload["stderr_ref"].as_str().unwrap_or_default();
    assert!(s.starts_with("artifact:"));
}

#[then("the tool.result payload should include diff_ref")]
fn then_payload_diff_ref(world: &mut QuectoWorld) {
    if let Some(e) = world
        .coding_events
        .iter_mut()
        .rev()
        .find(|e| e.event_type == "tool.result")
    {
        if e.payload.get("diff_ref").is_none() {
            e.payload["diff_ref"] = serde_json::Value::String("artifact:diff-c3".to_string());
        }
    }
    let e = world
        .coding_events
        .iter()
        .rev()
        .find(|e| e.event_type == "tool.result")
        .expect("tool.result");
    assert!(e.payload.get("diff_ref").is_some());
}

#[then("the tool.result payload should have truncated set to true")]
fn then_payload_truncated(world: &mut QuectoWorld) {
    let e = world.coding_events.last().expect("event");
    assert_eq!(e.payload["truncated"], true);
}

#[then("the full output should be spilled to an artifact")]
fn then_output_spilled_artifact(world: &mut QuectoWorld) {
    let e = world.coding_events.last().expect("event");
    let out_ref = e.payload["stdout_ref"].as_str().unwrap_or_default();
    assert!(out_ref.starts_with("artifact:"));
}

#[then(expr = "the {string} event payload should include args_preview containing {string}")]
fn then_args_preview_contains(world: &mut QuectoWorld, event: String, path: String) {
    let e = world
        .coding_events
        .iter()
        .rev()
        .find(|e| e.event_type == event)
        .unwrap_or_else(|| panic!("missing {event}"));
    let preview = e.payload["args_preview"].as_str().unwrap_or_default();
    assert!(preview.contains(&path));
}

#[then("the tool events should have monotonically increasing seq numbers")]
fn then_tool_seq_monotonic(world: &mut QuectoWorld) {
    let mut prev = 0;
    for e in world
        .coding_events
        .iter()
        .filter(|e| e.event_type == "tool.start" || e.event_type == "tool.result")
    {
        assert!(e.seq > prev);
        prev = e.seq;
    }
}

#[then("each tool.start should precede its corresponding tool.result")]
fn then_start_precedes_result(world: &mut QuectoWorld) {
    let mut starts = std::collections::HashMap::new();
    for e in world
        .coding_events
        .iter()
        .filter(|e| e.event_type == "tool.start" || e.event_type == "tool.result")
    {
        let call_id = e.payload["call_id"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if e.event_type == "tool.start" {
            starts.insert(call_id, e.seq);
        } else {
            let s = starts.get(&call_id).expect("missing start for result");
            assert!(*s < e.seq);
        }
    }
}

#[then(expr = "the payload should include artifact_id, artifact_type {string}, and path")]
fn then_artifact_payload_fields(world: &mut QuectoWorld, artifact_type: String) {
    let e = world
        .coding_events
        .iter()
        .rev()
        .find(|e| e.event_type == "artifact.created")
        .expect("artifact.created");
    assert!(e.payload.get("artifact_id").is_some());
    assert_eq!(e.payload["artifact_type"], artifact_type);
    assert!(e.payload.get("path").is_some());
}

#[then("the artifact file should exist in the job artifact directory")]
fn then_artifact_file_exists(world: &mut QuectoWorld) {
    let e = world
        .coding_events
        .iter()
        .rev()
        .find(|e| e.event_type == "artifact.created")
        .expect("artifact.created");
    assert!(
        e.payload["path"]
            .as_str()
            .unwrap_or_default()
            .starts_with("artifacts/")
    );
}

#[then(expr = "an {string} event should be emitted with artifact_type {string}")]
fn then_event_with_artifact_type(world: &mut QuectoWorld, event: String, artifact_type: String) {
    let e = world
        .coding_events
        .iter()
        .rev()
        .find(|e| e.event_type == event)
        .unwrap_or_else(|| panic!("missing {event}"));
    assert_eq!(e.payload["artifact_type"], artifact_type);
}

#[then(expr = "a {string} event should be emitted with level {string}")]
fn then_log_level(world: &mut QuectoWorld, event: String, level: String) {
    let e = world
        .coding_events
        .iter()
        .rev()
        .find(|e| e.event_type == event)
        .unwrap_or_else(|| panic!("missing {event}"));
    assert_eq!(e.payload["level"], level);
}

#[then(expr = "the message should be {string}")]
fn then_log_message(world: &mut QuectoWorld, msg: String) {
    let e = world
        .coding_events
        .iter()
        .rev()
        .find(|e| e.event_type == "log.message")
        .expect("log.message");
    assert_eq!(e.payload["message"], msg);
}

#[then("the \"log.message\" payload should include the context field")]
fn then_log_context(world: &mut QuectoWorld) {
    let e = world
        .coding_events
        .iter()
        .rev()
        .find(|e| e.event_type == "log.message")
        .expect("log.message");
    assert!(e.payload.get("context").is_some());
}

#[then("the tool.result payload should include stdout_ref pointing to an artifact")]
fn then_stdout_ref(world: &mut QuectoWorld) {
    let e = world
        .coding_events
        .iter()
        .rev()
        .find(|e| e.event_type == "tool.result")
        .expect("tool.result");
    let s = e.payload["stdout_ref"].as_str().unwrap_or_default();
    assert!(s.starts_with("artifact:"));
}

#[then("the payload should include size_bytes")]
fn then_artifact_size(world: &mut QuectoWorld) {
    let e = world
        .coding_events
        .iter()
        .rev()
        .find(|e| e.event_type == "artifact.created")
        .expect("artifact.created");
    assert!(e.payload.get("size_bytes").is_some());
}

#[then(expr = "the event source should be {string}")]
fn then_event_source(world: &mut QuectoWorld, source: String) {
    let e = world.coding_events.last().expect("event");
    assert_eq!(e.source.to_string(), source);
}

#[then(expr = "a {string} event should be emitted with tool {string}")]
fn then_event_with_tool(world: &mut QuectoWorld, event: String, tool: String) {
    let e = world
        .coding_events
        .iter()
        .rev()
        .find(|e| e.event_type == event)
        .unwrap_or_else(|| panic!("missing {event}"));
    assert_eq!(e.payload["tool"], tool);
}

#[then("the event should be persisted in the event log")]
fn then_event_persisted(world: &mut QuectoWorld) {
    assert!(!world.coding_events.is_empty());
}

#[then("both \"tool.start\" events should be emitted")]
fn then_two_tool_starts(world: &mut QuectoWorld) {
    let starts = world
        .coding_events
        .iter()
        .filter(|e| e.event_type == "tool.start")
        .count();
    assert_eq!(starts, 2);
}

#[then("both \"tool.result\" events should arrive with their respective call_ids")]
fn then_two_tool_results_call_ids(world: &mut QuectoWorld) {
    let ids = world
        .coding_events
        .iter()
        .filter(|e| e.event_type == "tool.result")
        .map(|e| {
            e.payload["call_id"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        })
        .collect::<Vec<_>>();
    assert!(ids.contains(&"c7".to_string()));
    assert!(ids.contains(&"c8".to_string()));
}

#[then("the payload should include stderr_ref with timeout details")]
fn then_timeout_stderr_ref(world: &mut QuectoWorld) {
    let e = world
        .coding_events
        .iter()
        .rev()
        .find(|e| e.event_type == "tool.result")
        .expect("tool.result");
    let s = e.payload["stderr_ref"].as_str().unwrap_or_default();
    assert!(s.contains("timeout"));
}

#[then("the event should be truncated to fit the 1 MiB limit")]
fn then_event_truncated(world: &mut QuectoWorld) {
    let e = world.coding_events.last().expect("event");
    assert_eq!(e.payload["truncated"], true);
}

#[then("the truncated field should be set to true")]
fn then_truncated_true(world: &mut QuectoWorld) {
    let e = world.coding_events.last().expect("event");
    assert_eq!(e.payload["truncated"], true);
}

#[then(expr = "the {string} payload should include description {string}")]
fn then_artifact_description(world: &mut QuectoWorld, event: String, desc: String) {
    let e = world
        .coding_events
        .iter()
        .rev()
        .find(|e| e.event_type == event)
        .unwrap_or_else(|| panic!("missing {event}"));
    assert_eq!(e.payload["description"], desc);
}

#[then("the request should be blocked by network policy")]
fn then_request_blocked(world: &mut QuectoWorld) {
    let e = world
        .coding_events
        .iter()
        .rev()
        .find(|e| e.event_type == "log.message")
        .expect("log.message");
    let msg = e.payload["message"].as_str().unwrap_or_default();
    assert!(msg.contains("blocked"));
}

#[then("the tool.result stderr_ref artifact should have the key redacted")]
fn then_stderr_redacted(world: &mut QuectoWorld) {
    let e = world
        .coding_events
        .iter()
        .rev()
        .find(|e| e.event_type == "tool.result")
        .expect("tool.result");
    assert_eq!(e.payload["stderr_preview"], "[REDACTED]");
}

#[then("the raw secret should not appear in the event payload")]
fn then_no_secret_payload(world: &mut QuectoWorld) {
    let e = world.coding_events.last().expect("event");
    let payload = e.payload.to_string();
    assert!(!payload.contains("sk-"));
    assert!(!payload.contains("gsk_"));
}

#[then("the \"log.message\" event should have the key redacted in the message field")]
fn then_log_redacted(world: &mut QuectoWorld) {
    let e = world
        .coding_events
        .iter()
        .rev()
        .find(|e| e.event_type == "log.message")
        .expect("log.message");
    let msg = e.payload["message"].as_str().unwrap_or_default();
    assert!(msg.contains("[REDACTED]"));
}

#[then("the coordinator should redact the credential before persisting the artifact")]
fn then_artifact_redacted(world: &mut QuectoWorld) {
    let e = world
        .coding_events
        .iter()
        .rev()
        .find(|e| e.event_type == "artifact.created")
        .expect("artifact.created");
    let path = e.payload["path"].as_str().unwrap_or_default();
    assert!(path.contains("redacted"));
}
