//! Step definitions for the Worker IPC Integration feature.

use std::sync::Arc;

use cucumber::{given, then, when};

use quecto::domain::coding_ports::WorkerEventSink;
use quecto::domain::error::DomainError;
use quecto::domain::message::{LlmResponse, UsageInfo};
use quecto::domain::provider::{ChatRequest, LlmProvider};
use quecto::infrastructure::coding::worker_event_emitter::{
    EmitterConfig, WorkerEventEmitter, WorkerEventSinkAdapter,
};
use quecto::interface::cli::worker::{WorkerDeps, cmd_worker_with_deps};

use crate::QuectoWorld;

// ── Mock providers (local to IPC steps) ────────────────────────────────

#[derive(Debug)]
struct IpcTextProvider {
    text: String,
}

impl LlmProvider for IpcTextProvider {
    fn name(&self) -> &str {
        "ipc-text-mock"
    }

    fn chat(
        &self,
        _request: ChatRequest<'_>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<LlmResponse, DomainError>> + Send + '_>,
    > {
        let text = self.text.clone();
        Box::pin(async move {
            Ok(LlmResponse {
                content: Some(text),
                tool_calls: vec![],
                usage: Some(UsageInfo {
                    prompt_tokens: 10,
                    completion_tokens: 20,
                }),
            })
        })
    }
}

#[derive(Debug)]
struct IpcErrorProvider {
    error_msg: String,
}

impl LlmProvider for IpcErrorProvider {
    fn name(&self) -> &str {
        "ipc-error-mock"
    }

    fn chat(
        &self,
        _request: ChatRequest<'_>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<LlmResponse, DomainError>> + Send + '_>,
    > {
        let msg = self.error_msg.clone();
        Box::pin(async move { Err(DomainError::Provider(msg)) })
    }
}

// ── Given steps: adapter ───────────────────────────────────────────────

#[given(expr = "a sink adapter wrapping a buffer emitter for run {string} and job {string}")]
fn given_sink_adapter(world: &mut QuectoWorld, run_id: String, job_id: String) {
    let emitter = WorkerEventEmitter::new(
        EmitterConfig {
            run_id,
            job_id,
            version: "1.0".to_string(),
        },
        Vec::new(),
    );
    world.ipc_adapter = Some(Arc::new(WorkerEventSinkAdapter::new(emitter)));
    world.ipc_emit_results = Vec::new();
}

// ── When steps: adapter ────────────────────────────────────────────────

#[when(expr = "I emit a {string} event through the adapter")]
fn when_emit_via_adapter(world: &mut QuectoWorld, event_type: String) {
    let adapter = world.ipc_adapter.as_ref().expect("adapter not set");
    let result = adapter.emit(
        &event_type,
        serde_json::json!({"level": "info", "message": "test"}),
    );
    world.ipc_last_emit_result = Some(result);
}

#[when(expr = "I emit {int} {string} events through the adapter")]
fn when_emit_multiple(world: &mut QuectoWorld, count: usize, event_type: String) {
    let adapter = world.ipc_adapter.as_ref().expect("adapter not set");
    for i in 0..count {
        let result = adapter.emit(
            &event_type,
            serde_json::json!({"level": "info", "message": format!("msg-{i}")}),
        );
        world.ipc_emit_results.push(result);
    }
}

// ── Then steps: adapter ────────────────────────────────────────────────

#[then("the adapter emit should succeed with a sequence number")]
fn then_adapter_emit_ok(world: &mut QuectoWorld) {
    let result = world.ipc_last_emit_result.as_ref().expect("no emit result");
    let seq = result.as_ref().expect("emit should succeed");
    assert!(*seq > 0, "sequence number should be positive, got {seq}");
}

#[then(expr = "the adapter output should contain valid JSON with run_id {string}")]
fn then_adapter_json_run_id(world: &mut QuectoWorld, expected: String) {
    let json = last_adapter_json(world);
    assert_eq!(
        json["run_id"].as_str(),
        Some(expected.as_str()),
        "run_id mismatch"
    );
}

#[then(expr = "the adapter output should contain valid JSON with job_id {string}")]
fn then_adapter_json_job_id(world: &mut QuectoWorld, expected: String) {
    let json = last_adapter_json(world);
    assert_eq!(
        json["job_id"].as_str(),
        Some(expected.as_str()),
        "job_id mismatch"
    );
}

#[then(expr = "the adapter output should contain a {string} field")]
fn then_adapter_json_has_field(world: &mut QuectoWorld, field: String) {
    let json = last_adapter_json(world);
    assert!(
        json.get(&field).is_some(),
        "expected field '{field}' in JSON: {json}"
    );
}

#[then(expr = "the adapter emit should fail with {string}")]
fn then_adapter_emit_fail(world: &mut QuectoWorld, expected: String) {
    let result = world.ipc_last_emit_result.as_ref().expect("no emit result");
    let err = result.as_ref().expect_err("expected emit to fail");
    assert!(
        err.contains(&expected),
        "expected error to contain '{expected}' but got: {err}"
    );
}

#[then(expr = "the adapter should have assigned sequences 1, 2, 3")]
fn then_adapter_sequences(world: &mut QuectoWorld) {
    let seqs: Vec<u64> = world
        .ipc_emit_results
        .iter()
        .map(|r| *r.as_ref().expect("emit failed"))
        .collect();
    assert_eq!(seqs, vec![1, 2, 3], "sequence mismatch: {seqs:?}");
}

// ── Given steps: cmd_worker IPC ────────────────────────────────────────

#[given("a temporary worker job directory")]
fn given_temp_worker_dir(world: &mut QuectoWorld) {
    let tmp = tempfile::TempDir::new().unwrap();
    world.ipc_job_dir = Some(tmp.path().to_path_buf());
    world.ipc_temp_dir = Some(tmp);
}

#[given(expr = "an IPC mock provider that returns {string}")]
fn given_ipc_text_provider(world: &mut QuectoWorld, text: String) {
    world.ipc_provider = Some(Arc::new(IpcTextProvider { text }));
}

#[given(expr = "an IPC mock provider that returns an error {string}")]
fn given_ipc_error_provider(world: &mut QuectoWorld, msg: String) {
    world.ipc_provider = Some(Arc::new(IpcErrorProvider { error_msg: msg }));
}

// ── When steps: cmd_worker IPC ─────────────────────────────────────────

#[when("I run cmd_worker with the mock provider")]
fn when_run_cmd_worker_ipc(world: &mut QuectoWorld) {
    let job_dir = world
        .ipc_job_dir
        .as_ref()
        .expect("job dir not set")
        .to_str()
        .unwrap()
        .to_string();
    let provider = world.ipc_provider.take().expect("provider not set");
    let args: Vec<String> = vec![
        "--run-id".to_string(),
        "run-ipc".to_string(),
        "--job-id".to_string(),
        "job-ipc".to_string(),
        "--job-dir".to_string(),
        job_dir,
        "--goal".to_string(),
        "fix the bug".to_string(),
    ];
    let deps = WorkerDeps { provider };
    let mut stdout = String::new();
    let mut stderr = String::new();
    let code = cmd_worker_with_deps(&args, deps, &mut stdout, &mut stderr);
    world.ipc_stdout = Some(stdout);
    world.ipc_stderr = Some(stderr);
    world.ipc_exit_code = Some(code);
}

#[when("I run cmd_worker IPC with no arguments")]
fn when_run_cmd_worker_ipc_no_args(world: &mut QuectoWorld) {
    // Use cmd_worker_with_deps with empty args — it should fail on parsing
    let provider: Arc<dyn LlmProvider> = Arc::new(IpcTextProvider {
        text: "unused".to_string(),
    });
    let deps = WorkerDeps { provider };
    let args: Vec<String> = vec![];
    let mut stdout = String::new();
    let mut stderr = String::new();
    let code = cmd_worker_with_deps(&args, deps, &mut stdout, &mut stderr);
    world.ipc_stdout = Some(stdout);
    world.ipc_stderr = Some(stderr);
    world.ipc_exit_code = Some(code);
}

// ── Then steps: cmd_worker IPC ─────────────────────────────────────────

#[then(expr = "the IPC worker exit code should be {int}")]
fn then_ipc_exit_code(world: &mut QuectoWorld, expected: i32) {
    let code = world.ipc_exit_code.expect("no exit code");
    assert_eq!(code, expected, "exit code mismatch");
}

#[then(expr = "the IPC worker output should contain at least {int} JSON lines")]
fn then_ipc_min_json_lines(world: &mut QuectoWorld, min: usize) {
    let lines = ipc_json_lines(world);
    assert!(
        lines.len() >= min,
        "expected at least {min} JSON lines, got {}: {:?}",
        lines.len(),
        world.ipc_stdout
    );
}

#[then(expr = "the IPC worker output should include a {string} event")]
fn then_ipc_has_event_type(world: &mut QuectoWorld, event_type: String) {
    let lines = ipc_json_lines(world);
    let found = lines
        .iter()
        .any(|j| j["type"].as_str() == Some(&event_type));
    assert!(
        found,
        "no '{event_type}' event found in output. Events: {:?}",
        lines.iter().map(|j| j["type"].clone()).collect::<Vec<_>>()
    );
}

#[then(expr = "the IPC worker first JSON line should be a {string} event")]
fn then_ipc_first_event_type(world: &mut QuectoWorld, event_type: String) {
    let lines = ipc_json_lines(world);
    assert!(!lines.is_empty(), "no JSON lines in output");
    assert_eq!(
        lines[0]["type"].as_str(),
        Some(event_type.as_str()),
        "first event type mismatch"
    );
}

#[then(expr = "the IPC worker first event message should contain {string}")]
fn then_ipc_first_msg_contains(world: &mut QuectoWorld, expected: String) {
    let lines = ipc_json_lines(world);
    let msg = lines[0]["payload"]["message"].as_str().unwrap_or_default();
    assert!(
        msg.contains(&expected),
        "first event message does not contain '{expected}': {msg}"
    );
}

#[then(expr = "the IPC worker last event message should contain {string}")]
fn then_ipc_last_msg_contains(world: &mut QuectoWorld, expected: String) {
    let lines = ipc_json_lines(world);
    let last = lines.last().expect("no JSON lines");
    let msg = last["payload"]["message"].as_str().unwrap_or_default();
    assert!(
        msg.contains(&expected),
        "last event message does not contain '{expected}': {msg}"
    );
}

#[then(expr = "the IPC worker stderr should contain {string}")]
fn then_ipc_stderr_contains(world: &mut QuectoWorld, expected: String) {
    let stderr = world.ipc_stderr.as_ref().expect("no stderr");
    assert!(
        stderr.contains(&expected),
        "stderr does not contain '{expected}': {stderr}"
    );
}

// ── Helpers ────────────────────────────────────────────────────────────

/// Get the last JSON line from the adapter's buffer output.
fn last_adapter_json(world: &QuectoWorld) -> serde_json::Value {
    let adapter = world.ipc_adapter.as_ref().expect("adapter not set");
    let buf: Vec<u8> = adapter
        .writer_snapshot()
        .expect("failed to get writer snapshot");
    let output = String::from_utf8(buf).unwrap();
    let last_line = output.lines().last().expect("no output lines");
    serde_json::from_str(last_line).expect("invalid JSON")
}

/// Parse all JSON lines from the IPC worker stdout.
fn ipc_json_lines(world: &QuectoWorld) -> Vec<serde_json::Value> {
    let stdout = world.ipc_stdout.as_ref().expect("no stdout");
    stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            serde_json::from_str(l).unwrap_or_else(|e| panic!("invalid JSON line: {e}\nline: {l}"))
        })
        .collect()
}
