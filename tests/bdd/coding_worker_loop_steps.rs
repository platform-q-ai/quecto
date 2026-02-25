//! Step definitions for the worker agent loop feature.

use cucumber::then;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use quecto::application::worker_loop::{
    WorkerLoopConfig, WorkerLoopParams, build_worker_system_prompt, run_worker_loop,
};
use quecto::domain::coding_ports::WorkerEventSink;
use quecto::domain::error::DomainError;
use quecto::domain::message::{LlmResponse, Message, ToolCall, UsageInfo};
use quecto::domain::provider::{ChatRequest, LlmProvider};
use quecto::infrastructure::coding::worker_tool_wrappers::build_worker_tool_registry;

use crate::QuectoWorld;

// ── BddEventSink ──────────────────────────────────────────────────────

/// A test-only event sink that collects emitted events in memory.
/// Uses interior mutability (`Mutex`) so `emit(&self, ...)` works.
#[derive(Debug, Default)]
struct BddEventSink {
    events: Mutex<Vec<serde_json::Value>>,
    seq: AtomicU64,
}

impl BddEventSink {
    fn new() -> Self {
        Self::default()
    }

    fn events(&self) -> Vec<serde_json::Value> {
        self.events.lock().unwrap().clone()
    }
}

impl WorkerEventSink for BddEventSink {
    fn emit(&self, event_type: &str, payload: serde_json::Value) -> Result<u64, String> {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
        self.events.lock().unwrap().push(serde_json::json!({
            "type": event_type,
            "seq": seq,
            "payload": payload,
        }));
        Ok(seq)
    }
}

// ── Mock providers ─────────────────────────────────────────────────────

#[derive(Debug)]
struct TextProvider {
    text: String,
    captured: Arc<Mutex<Vec<Vec<Message>>>>,
}

impl TextProvider {
    fn new(text: &str, captured: Arc<Mutex<Vec<Vec<Message>>>>) -> Self {
        Self {
            text: text.to_string(),
            captured,
        }
    }
}

impl LlmProvider for TextProvider {
    fn name(&self) -> &str {
        "text-mock"
    }

    fn chat(
        &self,
        request: ChatRequest<'_>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<LlmResponse, DomainError>> + Send + '_>,
    > {
        self.captured
            .lock()
            .unwrap()
            .push(request.messages.to_vec());

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
struct ErrorProvider {
    error_msg: String,
}

impl ErrorProvider {
    fn new(msg: &str) -> Self {
        Self {
            error_msg: msg.to_string(),
        }
    }
}

impl LlmProvider for ErrorProvider {
    fn name(&self) -> &str {
        "error-mock"
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

#[derive(Debug)]
struct ToolThenTextProvider {
    tool_responses: Mutex<Vec<LlmResponse>>,
    captured: Mutex<Vec<Vec<Message>>>,
}

impl ToolThenTextProvider {
    fn new(responses: Vec<LlmResponse>) -> Self {
        Self {
            tool_responses: Mutex::new(responses),
            captured: Mutex::new(vec![]),
        }
    }
}

impl LlmProvider for ToolThenTextProvider {
    fn name(&self) -> &str {
        "tool-then-text-mock"
    }

    fn chat(
        &self,
        request: ChatRequest<'_>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<LlmResponse, DomainError>> + Send + '_>,
    > {
        self.captured
            .lock()
            .unwrap()
            .push(request.messages.to_vec());

        let response = {
            let mut responses = self.tool_responses.lock().unwrap();
            if responses.is_empty() {
                LlmResponse {
                    content: Some("(exhausted)".to_string()),
                    tool_calls: vec![],
                    usage: None,
                }
            } else {
                responses.remove(0)
            }
        };

        Box::pin(async move { Ok(response) })
    }
}

// ── Helpers ────────────────────────────────────────────────────────────

fn make_tool_call_response(tool_name: &str, args: &str) -> LlmResponse {
    LlmResponse {
        content: None,
        tool_calls: vec![ToolCall {
            id: format!("call_{tool_name}"),
            name: tool_name.to_string(),
            arguments: args.to_string(),
        }],
        usage: None,
    }
}

fn make_text_response(text: &str) -> LlmResponse {
    LlmResponse {
        content: Some(text.to_string()),
        tool_calls: vec![],
        usage: Some(UsageInfo {
            prompt_tokens: 10,
            completion_tokens: 20,
        }),
    }
}

fn events_by_type(events: &[serde_json::Value], t: &str) -> Vec<serde_json::Value> {
    events
        .iter()
        .filter(|e| e["type"].as_str() == Some(t))
        .cloned()
        .collect()
}

// ── Given steps ────────────────────────────────────────────────────────

#[cucumber::given("a worker loop context with a valid job directory")]
fn given_valid_job_dir(world: &mut QuectoWorld) {
    let tmp = tempfile::TempDir::new().unwrap();
    world.wl_job_dir = Some(tmp.path().to_path_buf());
    world.wl_temp_dir = Some(tmp);
    world.wl_config = Some(WorkerLoopConfig {
        run_id: "run-1".to_string(),
        job_id: "job-1".to_string(),
        job_dir: world
            .wl_job_dir
            .as_ref()
            .unwrap()
            .to_string_lossy()
            .to_string(),
        goal: "default goal".to_string(),
        ..WorkerLoopConfig::default()
    });
}

#[cucumber::given(expr = "a worker loop context with run_id {string} and job_id {string}")]
fn given_run_and_job_id(world: &mut QuectoWorld, run_id: String, job_id: String) {
    let tmp = tempfile::TempDir::new().unwrap();
    world.wl_job_dir = Some(tmp.path().to_path_buf());
    world.wl_temp_dir = Some(tmp);
    world.wl_config = Some(WorkerLoopConfig {
        run_id,
        job_id,
        job_dir: world
            .wl_job_dir
            .as_ref()
            .unwrap()
            .to_string_lossy()
            .to_string(),
        goal: "default goal".to_string(),
        ..WorkerLoopConfig::default()
    });
}

#[cucumber::given(expr = "a worker loop context with goal {string}")]
fn given_goal(world: &mut QuectoWorld, goal: String) {
    if world.wl_config.is_none() {
        let tmp = tempfile::TempDir::new().unwrap();
        world.wl_job_dir = Some(tmp.path().to_path_buf());
        world.wl_temp_dir = Some(tmp);
        world.wl_config = Some(WorkerLoopConfig {
            run_id: "run-1".to_string(),
            job_id: "job-1".to_string(),
            job_dir: world
                .wl_job_dir
                .as_ref()
                .unwrap()
                .to_string_lossy()
                .to_string(),
            goal: goal.clone(),
            ..WorkerLoopConfig::default()
        });
    }
    if let Some(ref mut cfg) = world.wl_config {
        cfg.goal = goal;
    }
}

#[cucumber::given(expr = "a worker loop context with max_iterations {int}")]
fn given_max_iterations(world: &mut QuectoWorld, max: u32) {
    if world.wl_config.is_none() {
        let tmp = tempfile::TempDir::new().unwrap();
        world.wl_job_dir = Some(tmp.path().to_path_buf());
        world.wl_temp_dir = Some(tmp);
        world.wl_config = Some(WorkerLoopConfig {
            run_id: "run-1".to_string(),
            job_id: "job-1".to_string(),
            job_dir: world
                .wl_job_dir
                .as_ref()
                .unwrap()
                .to_string_lossy()
                .to_string(),
            goal: "test goal".to_string(),
            ..WorkerLoopConfig::default()
        });
    }
    if let Some(ref mut cfg) = world.wl_config {
        cfg.max_iterations = max;
    }
}

#[cucumber::given(expr = "a worker loop context with a file {string} containing {string}")]
fn given_file_in_job_dir(world: &mut QuectoWorld, filename: String, content: String) {
    if world.wl_config.is_none() {
        let tmp = tempfile::TempDir::new().unwrap();
        world.wl_job_dir = Some(tmp.path().to_path_buf());
        world.wl_temp_dir = Some(tmp);
        world.wl_config = Some(WorkerLoopConfig {
            run_id: "run-1".to_string(),
            job_id: "job-1".to_string(),
            job_dir: world
                .wl_job_dir
                .as_ref()
                .unwrap()
                .to_string_lossy()
                .to_string(),
            goal: "test goal".to_string(),
            ..WorkerLoopConfig::default()
        });
    }
    let job_dir = world.wl_job_dir.as_ref().unwrap();
    let file_path = job_dir.join(&filename);
    if let Some(parent) = file_path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&file_path, &content).unwrap();
}

#[cucumber::given(expr = "a mock LLM provider that returns text {string}")]
fn given_text_provider(world: &mut QuectoWorld, text: String) {
    let captured = world.wl_captured_messages.clone();
    world.wl_provider = Some(Arc::new(TextProvider::new(&text, captured)));
}

#[cucumber::given(expr = "a mock LLM provider that captures messages and returns text {string}")]
fn given_capturing_text_provider(world: &mut QuectoWorld, text: String) {
    let captured = world.wl_captured_messages.clone();
    world.wl_provider = Some(Arc::new(TextProvider::new(&text, captured)));
}

#[cucumber::given(expr = "a mock LLM provider that returns an error {string}")]
fn given_error_provider(world: &mut QuectoWorld, msg: String) {
    world.wl_provider = Some(Arc::new(ErrorProvider::new(&msg)));
}

#[cucumber::given(
    expr = "a mock LLM provider that calls {string} for {string} then returns text {string}"
)]
fn given_tool_then_text_provider(
    world: &mut QuectoWorld,
    tool: String,
    file: String,
    text: String,
) {
    let responses = vec![
        make_tool_call_response(&tool, &format!(r#"{{"file_path":"{file}"}}"#)),
        make_text_response(&text),
    ];
    world.wl_provider = Some(Arc::new(ToolThenTextProvider::new(responses)));
}

#[cucumber::given(expr = "a mock LLM provider that always calls {string} for {string}")]
fn given_always_tool_provider(world: &mut QuectoWorld, tool: String, file: String) {
    // Provide more responses than max_iterations can consume
    let responses: Vec<LlmResponse> = (0..20)
        .map(|_| make_tool_call_response(&tool, &format!(r#"{{"file_path":"{file}"}}"#)))
        .collect();
    world.wl_provider = Some(Arc::new(ToolThenTextProvider::new(responses)));
}

#[cucumber::given(
    expr = "a mock LLM provider that calls tools {string} for {string} then returns text {string}"
)]
fn given_multi_tools_then_text(
    world: &mut QuectoWorld,
    tools_csv: String,
    file: String,
    text: String,
) {
    let tool_names: Vec<&str> = tools_csv.split(", ").collect();
    let mut responses: Vec<LlmResponse> = tool_names
        .iter()
        .map(|t| {
            let args = if t.contains("find") {
                r#"{"glob":"**/*.rs"}"#.to_string()
            } else {
                format!(r#"{{"file_path":"{file}"}}"#)
            };
            make_tool_call_response(t, &args)
        })
        .collect();
    responses.push(make_text_response(&text));
    world.wl_provider = Some(Arc::new(ToolThenTextProvider::new(responses)));
}

// ── When steps ─────────────────────────────────────────────────────────

#[cucumber::when("the worker loop builds the tool registry")]
fn when_build_registry(world: &mut QuectoWorld) {
    let job_dir = world.wl_job_dir.as_ref().unwrap().clone();
    let registry = build_worker_tool_registry(job_dir);
    world.wl_registry_names = Some(registry.names());
}

#[cucumber::when("the worker loop builds the event emitter")]
fn when_build_emitter(world: &mut QuectoWorld) {
    let cfg = world.wl_config.as_ref().unwrap();
    // Verify that we can construct a BddEventSink
    let sink = BddEventSink::new();
    world.wl_emitter_run_id = Some(cfg.run_id.clone());
    world.wl_emitter_job_id = Some(cfg.job_id.clone());
    // Verify construction succeeds — seq starts at 0
    assert_eq!(sink.seq.load(Ordering::Relaxed), 0);
}

#[cucumber::when("the worker loop builds the system prompt")]
fn when_build_system_prompt(world: &mut QuectoWorld) {
    let cfg = world.wl_config.as_ref().unwrap();
    let job_dir = world.wl_job_dir.as_ref().unwrap().clone();
    let registry = build_worker_tool_registry(job_dir);
    let defs = registry.definitions();
    world.wl_system_prompt = Some(build_worker_system_prompt(&cfg.goal, &defs));
}

#[cucumber::when("the worker loop runs to completion")]
fn when_run_loop(world: &mut QuectoWorld) {
    let config = world.wl_config.as_ref().unwrap().clone();
    let provider = world
        .wl_provider
        .take()
        .expect("mock provider must be set before running the loop");

    let sink = Arc::new(BddEventSink::new());
    let job_dir = world.wl_job_dir.as_ref().unwrap().clone();
    let tool_registry = build_worker_tool_registry(job_dir);

    let sink_clone: Arc<BddEventSink> = sink.clone();
    let result = tokio::runtime::Runtime::new().unwrap().block_on(async {
        run_worker_loop(
            WorkerLoopParams {
                config,
                provider,
                sink: sink_clone,
            },
            Box::new(tool_registry),
        )
        .await
    });

    world.wl_result = Some(result);
    // Extract events from the sink into the world for Then steps
    world.wl_emitted_events = sink.events();
}

// ── Then steps ─────────────────────────────────────────────────────────

#[then(expr = "the registry should contain exactly {string}")]
fn then_registry_contains(world: &mut QuectoWorld, expected: String) {
    let names = world.wl_registry_names.as_ref().unwrap();
    let expected_names: Vec<&str> = expected.split(", ").collect();
    assert_eq!(
        names.len(),
        expected_names.len(),
        "expected {} tools, got {}: {:?}",
        expected_names.len(),
        names.len(),
        names
    );
    for name in &expected_names {
        assert!(
            names.contains(&name.to_string()),
            "registry missing tool: {name}"
        );
    }
}

#[then(expr = "the emitter should be configured with run_id {string} and job_id {string}")]
fn then_emitter_config(world: &mut QuectoWorld, run_id: String, job_id: String) {
    assert_eq!(world.wl_emitter_run_id.as_deref(), Some(run_id.as_str()));
    assert_eq!(world.wl_emitter_job_id.as_deref(), Some(job_id.as_str()));
}

#[then(expr = "the system prompt should contain {string}")]
fn then_prompt_contains(world: &mut QuectoWorld, expected: String) {
    let prompt = world.wl_system_prompt.as_ref().unwrap();
    assert!(
        prompt.contains(&expected),
        "system prompt does not contain '{expected}': {prompt}"
    );
}

#[then(expr = "the first emitted event should be {string} with level {string}")]
fn then_first_event(world: &mut QuectoWorld, event_type: String, level: String) {
    let events = &world.wl_emitted_events;
    assert!(!events.is_empty(), "no events emitted");
    let first = &events[0];
    assert_eq!(
        first["type"].as_str(),
        Some(event_type.as_str()),
        "first event type mismatch"
    );
    assert_eq!(
        first["payload"]["level"].as_str(),
        Some(level.as_str()),
        "first event level mismatch"
    );
}

#[then(expr = "the first emitted event message should contain {string}")]
fn then_first_event_msg_contains(world: &mut QuectoWorld, expected: String) {
    let events = &world.wl_emitted_events;
    let msg = events[0]["payload"]["message"].as_str().unwrap();
    assert!(
        msg.contains(&expected),
        "first event message does not contain '{expected}': {msg}"
    );
}

#[then(expr = "the last emitted event should be {string} with level {string}")]
fn then_last_event(world: &mut QuectoWorld, event_type: String, level: String) {
    let events = &world.wl_emitted_events;
    let last = events.last().expect("no events emitted");
    assert_eq!(
        last["type"].as_str(),
        Some(event_type.as_str()),
        "last event type mismatch"
    );
    assert_eq!(
        last["payload"]["level"].as_str(),
        Some(level.as_str()),
        "last event level mismatch"
    );
}

#[then(expr = "the last emitted event message should contain {string}")]
fn then_last_event_msg_contains(world: &mut QuectoWorld, expected: String) {
    let events = &world.wl_emitted_events;
    let last = events.last().expect("no events emitted");
    let msg = last["payload"]["message"].as_str().unwrap();
    assert!(
        msg.contains(&expected),
        "last event message does not contain '{expected}': {msg}"
    );
}

#[then(expr = "the LLM should have received a user message containing {string}")]
fn then_llm_received_user_msg(world: &mut QuectoWorld, expected: String) {
    let captured = world.wl_captured_messages.lock().unwrap();
    assert!(!captured.is_empty(), "LLM received no messages");

    let first_call = &captured[0];
    let user_msg = first_call
        .iter()
        .find(|m| m.role == quecto::domain::message::Role::User);
    assert!(user_msg.is_some(), "no user message found in LLM input");
    assert!(
        user_msg.unwrap().content.contains(&expected),
        "user message does not contain '{expected}'"
    );
}

#[then(expr = "the worker loop result should have exit code {int}")]
fn then_exit_code(world: &mut QuectoWorld, code: i32) {
    let result = world.wl_result.as_ref().unwrap();
    assert_eq!(result.exit_code, code, "exit code mismatch");
}

#[then(expr = "the worker loop result should contain response {string}")]
fn then_response_contains(world: &mut QuectoWorld, expected: String) {
    let result = world.wl_result.as_ref().unwrap();
    let response = result.response.as_ref().expect("result has no response");
    assert!(
        response.contains(&expected),
        "response does not contain '{expected}': {response}"
    );
}

#[then("the worker loop result should have no response")]
fn then_no_response(world: &mut QuectoWorld) {
    let result = world.wl_result.as_ref().unwrap();
    assert!(result.response.is_none(), "expected no response");
}

#[then(expr = "the emitted events should include a {string} with tool {string}")]
fn then_event_with_tool(world: &mut QuectoWorld, event_type: String, tool: String) {
    let events = &world.wl_emitted_events;
    let matching = events_by_type(events, &event_type);
    let found = matching
        .iter()
        .any(|e| e["payload"]["tool"].as_str() == Some(&tool));
    assert!(
        found,
        "no {event_type} event found with tool '{tool}': {:?}",
        matching
    );
}

#[then(expr = "the {string} event should have ok true")]
fn then_event_ok_true(world: &mut QuectoWorld, event_type: String) {
    let events = &world.wl_emitted_events;
    let matching = events_by_type(events, &event_type);
    assert!(!matching.is_empty(), "no {event_type} events found");
    let ok = matching[0]["payload"]["ok"].as_bool();
    assert_eq!(ok, Some(true), "expected ok=true in {event_type} event");
}

#[then(expr = "the {string} event should have a numeric duration_ms")]
fn then_event_has_duration(world: &mut QuectoWorld, event_type: String) {
    let events = &world.wl_emitted_events;
    let matching = events_by_type(events, &event_type);
    assert!(!matching.is_empty(), "no {event_type} events found");
    let dur = matching[0]["payload"]["duration_ms"].as_u64();
    assert!(dur.is_some(), "duration_ms missing in {event_type} event");
}

#[then("the worker loop result should indicate iteration limit reached")]
fn then_iteration_limit_reached(world: &mut QuectoWorld) {
    let result = world.wl_result.as_ref().unwrap();
    assert!(
        result.iteration_limit_reached,
        "expected iteration_limit_reached to be true"
    );
}

#[then(expr = "the emitted events should include exactly {int} {string} events")]
fn then_exact_event_count(world: &mut QuectoWorld, count: usize, event_type: String) {
    let events = &world.wl_emitted_events;
    let matching = events_by_type(events, &event_type);
    assert_eq!(
        matching.len(),
        count,
        "expected {count} {event_type} events, got {}",
        matching.len()
    );
}

#[then(expr = "the event type sequence should be {string}")]
fn then_event_sequence(world: &mut QuectoWorld, expected: String) {
    let events = &world.wl_emitted_events;
    let types: Vec<&str> = events.iter().filter_map(|e| e["type"].as_str()).collect();
    let expected_types: Vec<&str> = expected.split(", ").collect();
    assert_eq!(types, expected_types, "event sequence mismatch");
}
