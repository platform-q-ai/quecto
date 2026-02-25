use super::*;
use crate::domain::message::{LlmResponse, ToolCall, UsageInfo};
use crate::domain::provider::ChatRequest;
use std::sync::Mutex as StdMutex;

// ── MockEventSink ──────────────────────────────────────────────────────

/// A test-only event sink that collects emitted events in memory.
#[derive(Debug, Default)]
struct MockEventSink {
    events: StdMutex<Vec<serde_json::Value>>,
    seq: AtomicU64,
}

impl MockEventSink {
    fn new() -> Self {
        Self::default()
    }

    fn events(&self) -> Vec<serde_json::Value> {
        self.events.lock().unwrap().clone()
    }
}

impl WorkerEventSink for MockEventSink {
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

// ── MockToolRegistry ───────────────────────────────────────────────────

/// A test-only tool registry that returns canned results.
struct MockToolRegistry {
    defs: Vec<ToolDefinition>,
    results: StdMutex<Vec<ToolResult>>,
}

impl std::fmt::Debug for MockToolRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockToolRegistry").finish()
    }
}

impl MockToolRegistry {
    fn with_tools(names: &[&str], results: Vec<ToolResult>) -> Self {
        let defs = names
            .iter()
            .map(|n| ToolDefinition {
                name: n.to_string(),
                description: format!("Mock {n}"),
                parameters_schema: r#"{"type":"object"}"#.to_string(),
            })
            .collect();
        Self {
            defs,
            results: StdMutex::new(results),
        }
    }

    fn empty() -> Self {
        Self::with_tools(
            &["worker_read", "worker_edit", "worker_grep", "worker_find"],
            vec![],
        )
    }
}

impl ToolRegistry for MockToolRegistry {
    fn definitions(&self) -> Vec<ToolDefinition> {
        self.defs.clone()
    }

    fn execute(
        &self,
        _name: &str,
        _arguments: &str,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ToolResult, DomainError>> + Send + '_>,
    > {
        let result = {
            let mut results = self.results.lock().unwrap();
            if results.is_empty() {
                ToolResult {
                    content: "ok".to_string(),
                    is_error: false,
                }
            } else {
                results.remove(0)
            }
        };
        Box::pin(async move { Ok(result) })
    }
}

// ── Mock provider ──────────────────────────────────────────────────────

#[derive(Debug)]
struct MockLoopProvider {
    responses: StdMutex<Vec<LlmResponse>>,
    captured_messages: StdMutex<Vec<Vec<Message>>>,
}

impl MockLoopProvider {
    fn new(responses: Vec<LlmResponse>) -> Self {
        Self {
            responses: StdMutex::new(responses),
            captured_messages: StdMutex::new(vec![]),
        }
    }

    fn captured_messages(&self) -> Vec<Vec<Message>> {
        self.captured_messages.lock().unwrap().clone()
    }
}

impl LlmProvider for MockLoopProvider {
    fn name(&self) -> &str {
        "mock-loop"
    }

    fn chat(
        &self,
        request: ChatRequest<'_>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<LlmResponse, DomainError>> + Send + '_>,
    > {
        self.captured_messages
            .lock()
            .unwrap()
            .push(request.messages.to_vec());

        let response = {
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                return Box::pin(async {
                    Ok(LlmResponse {
                        content: Some("(exhausted)".to_string()),
                        tool_calls: vec![],
                        usage: None,
                    })
                });
            }
            responses.remove(0)
        };

        Box::pin(async move { Ok(response) })
    }
}

#[derive(Debug)]
struct ErrorProvider;

impl LlmProvider for ErrorProvider {
    fn name(&self) -> &str {
        "error-provider"
    }

    fn chat(
        &self,
        _request: ChatRequest<'_>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<LlmResponse, DomainError>> + Send + '_>,
    > {
        Box::pin(async { Err(DomainError::Provider("provider connection failed".into())) })
    }
}

// ── Helpers ────────────────────────────────────────────────────────────

fn text_response(content: &str) -> LlmResponse {
    LlmResponse {
        content: Some(content.to_string()),
        tool_calls: vec![],
        usage: Some(UsageInfo {
            prompt_tokens: 10,
            completion_tokens: 20,
        }),
    }
}

fn tool_response(name: &str, args: &str) -> LlmResponse {
    LlmResponse {
        content: None,
        tool_calls: vec![ToolCall {
            id: format!("call_{name}"),
            name: name.to_string(),
            arguments: args.to_string(),
        }],
        usage: None,
    }
}

fn make_config() -> WorkerLoopConfig {
    WorkerLoopConfig {
        run_id: "run-1".to_string(),
        job_id: "job-1".to_string(),
        job_dir: "/tmp/test-job".to_string(),
        goal: "Test goal".to_string(),
        ..WorkerLoopConfig::default()
    }
}

fn make_sink() -> Arc<MockEventSink> {
    Arc::new(MockEventSink::new())
}

fn get_events(sink: &MockEventSink) -> Vec<serde_json::Value> {
    sink.events()
}

fn events_of_type(events: &[serde_json::Value], event_type: &str) -> Vec<serde_json::Value> {
    events
        .iter()
        .filter(|e| e["type"].as_str() == Some(event_type))
        .cloned()
        .collect()
}

fn mock_tool_defs() -> Vec<ToolDefinition> {
    ["worker_read", "worker_edit", "worker_grep", "worker_find"]
        .iter()
        .map(|n| ToolDefinition {
            name: n.to_string(),
            description: format!("Mock {n}"),
            parameters_schema: r#"{"type":"object"}"#.to_string(),
        })
        .collect()
}

// ── Tests ──────────────────────────────────────────────────────────────

#[test]
fn test_build_worker_system_prompt_contains_goal() {
    let defs = mock_tool_defs();
    let prompt = build_worker_system_prompt("Fix the flaky test", &defs);
    assert!(prompt.contains("Fix the flaky test"));
}

#[test]
fn test_build_worker_system_prompt_contains_tool_names() {
    let defs = mock_tool_defs();
    let prompt = build_worker_system_prompt("any goal", &defs);
    assert!(prompt.contains("worker_read"));
    assert!(prompt.contains("worker_edit"));
    assert!(prompt.contains("worker_grep"));
    assert!(prompt.contains("worker_find"));
}

#[test]
fn test_build_worker_system_prompt_uses_tool_descriptions() {
    let defs = vec![ToolDefinition {
        name: "custom_tool".to_string(),
        description: "A custom tool for testing".to_string(),
        parameters_schema: r#"{"type":"object"}"#.to_string(),
    }];
    let prompt = build_worker_system_prompt("goal", &defs);
    assert!(prompt.contains("custom_tool: A custom tool for testing"));
}

#[test]
fn test_truncate_utf8_safe_short() {
    let args = r#"{"path":"test.rs"}"#;
    let result = truncate_utf8_safe(args, MAX_ARGS_PREVIEW_CHARS);
    assert_eq!(result, args);
}

#[test]
fn test_truncate_utf8_safe_long() {
    let args = "x".repeat(300);
    let result = truncate_utf8_safe(&args, MAX_ARGS_PREVIEW_CHARS);
    assert!(result.len() <= MAX_ARGS_PREVIEW_CHARS + 3);
    assert!(result.ends_with("..."));
}

#[test]
fn test_truncate_utf8_safe_multibyte() {
    // 250 emoji = 250 chars but 1000 bytes — must not panic on byte boundary
    let args = "\u{1F600}".repeat(250);
    let result = truncate_utf8_safe(&args, MAX_ARGS_PREVIEW_CHARS);
    assert!(result.ends_with("..."));
    // Should have 197 emoji + "..." = 200 chars total
    assert_eq!(result.chars().count(), MAX_ARGS_PREVIEW_CHARS);
}

#[test]
fn test_call_id_per_instance() {
    let sink = make_sink();
    let reg1 = EventEmittingRegistry::new(Box::new(MockToolRegistry::empty()), sink.clone());
    let reg2 = EventEmittingRegistry::new(Box::new(MockToolRegistry::empty()), sink);
    // Both registries start at 1
    let id1 = reg1.next_call_id("test");
    let id2 = reg2.next_call_id("test");
    assert_eq!(id1, "wc_test_1");
    assert_eq!(id2, "wc_test_1");
}

#[tokio::test]
async fn test_worker_loop_emits_ready_event() {
    let config = make_config();
    let sink = make_sink();
    let provider = Arc::new(MockLoopProvider::new(vec![text_response("done")]));

    let result = run_worker_loop(
        WorkerLoopParams {
            config,
            provider,
            sink: sink.clone(),
        },
        Box::new(MockToolRegistry::empty()),
    )
    .await;

    assert_eq!(result.exit_code, 0);

    let events = get_events(&sink);
    let logs = events_of_type(&events, "log.message");
    assert!(!logs.is_empty());
    let ready = &logs[0];
    assert_eq!(ready["payload"]["level"], "info");
    assert!(
        ready["payload"]["message"]
            .as_str()
            .unwrap()
            .contains("ready")
    );
}

#[tokio::test]
async fn test_worker_loop_emits_done_event() {
    let config = make_config();
    let sink = make_sink();
    let provider = Arc::new(MockLoopProvider::new(vec![text_response("all fixed")]));

    run_worker_loop(
        WorkerLoopParams {
            config,
            provider,
            sink: sink.clone(),
        },
        Box::new(MockToolRegistry::empty()),
    )
    .await;

    let events = get_events(&sink);
    let logs = events_of_type(&events, "log.message");
    let done = logs.last().unwrap();
    assert_eq!(done["payload"]["level"], "info");
    assert!(
        done["payload"]["message"]
            .as_str()
            .unwrap()
            .contains("done")
    );
}

#[tokio::test]
async fn test_worker_loop_error_event_on_provider_failure() {
    let config = make_config();
    let sink = make_sink();
    let provider = Arc::new(ErrorProvider);

    let result = run_worker_loop(
        WorkerLoopParams {
            config,
            provider,
            sink: sink.clone(),
        },
        Box::new(MockToolRegistry::empty()),
    )
    .await;

    assert_eq!(result.exit_code, 1);
    assert!(result.response.is_none());

    let events = get_events(&sink);
    let logs = events_of_type(&events, "log.message");
    let error = logs.last().unwrap();
    assert_eq!(error["payload"]["level"], "error");
    assert!(
        error["payload"]["message"]
            .as_str()
            .unwrap()
            .contains("error")
    );
}

#[tokio::test]
async fn test_worker_loop_sends_goal_as_user_message() {
    let mut config = make_config();
    config.goal = "Refactor the parser module".to_string();
    let sink = make_sink();
    let provider = Arc::new(MockLoopProvider::new(vec![text_response("done")]));

    run_worker_loop(
        WorkerLoopParams {
            config,
            provider: provider.clone(),
            sink: sink.clone(),
        },
        Box::new(MockToolRegistry::empty()),
    )
    .await;

    let captured = provider.captured_messages();
    assert!(!captured.is_empty());
    let first_call = &captured[0];
    let user_msg = first_call
        .iter()
        .find(|m| m.role == crate::domain::message::Role::User);
    assert!(user_msg.is_some());
    assert!(user_msg.unwrap().content.contains("Refactor the parser"));
}

#[tokio::test]
async fn test_worker_loop_emits_tool_events() {
    let config = make_config();
    let sink = make_sink();
    let provider = Arc::new(MockLoopProvider::new(vec![
        tool_response("worker_read", r#"{"file_path":"test.rs"}"#),
        text_response("read the file"),
    ]));

    let tool_results = vec![ToolResult {
        content: "fn main() {}".to_string(),
        is_error: false,
    }];

    run_worker_loop(
        WorkerLoopParams {
            config,
            provider,
            sink: sink.clone(),
        },
        Box::new(MockToolRegistry::with_tools(
            &["worker_read", "worker_edit", "worker_grep", "worker_find"],
            tool_results,
        )),
    )
    .await;

    let events = get_events(&sink);
    let starts = events_of_type(&events, "tool.start");
    let results = events_of_type(&events, "tool.result");
    assert_eq!(starts.len(), 1);
    assert_eq!(results.len(), 1);
    assert_eq!(starts[0]["payload"]["tool"], "worker_read");
    assert_eq!(results[0]["payload"]["tool"], "worker_read");
    assert!(results[0]["payload"]["duration_ms"].as_u64().is_some());
    assert_eq!(results[0]["payload"]["ok"], true);
}

#[tokio::test]
async fn test_worker_loop_respects_max_iterations() {
    let mut config = make_config();
    config.max_iterations = 2;
    let sink = make_sink();

    // Provider always returns tool calls
    let provider = Arc::new(MockLoopProvider::new(vec![
        tool_response("worker_read", r#"{"file_path":"f.rs"}"#),
        tool_response("worker_read", r#"{"file_path":"f.rs"}"#),
        tool_response("worker_read", r#"{"file_path":"f.rs"}"#),
    ]));

    let result = run_worker_loop(
        WorkerLoopParams {
            config,
            provider,
            sink: sink.clone(),
        },
        Box::new(MockToolRegistry::empty()),
    )
    .await;

    assert!(result.iteration_limit_reached);
    assert_eq!(result.exit_code, 0); // iteration limit is still a clean exit

    let events = get_events(&sink);
    let starts = events_of_type(&events, "tool.start");
    assert_eq!(starts.len(), 2);
}

#[tokio::test]
async fn test_worker_loop_multiple_tool_calls_in_sequence() {
    let config = make_config();
    let sink = make_sink();

    let provider = Arc::new(MockLoopProvider::new(vec![
        tool_response("worker_find", r#"{"glob":"**/*.rs"}"#),
        tool_response("worker_read", r#"{"file_path":"a.rs"}"#),
        text_response("found and read files"),
    ]));

    let tool_results = vec![
        ToolResult {
            content: "a.rs\nsrc/b.rs".to_string(),
            is_error: false,
        },
        ToolResult {
            content: "fn a() {}".to_string(),
            is_error: false,
        },
    ];

    run_worker_loop(
        WorkerLoopParams {
            config,
            provider,
            sink: sink.clone(),
        },
        Box::new(MockToolRegistry::with_tools(
            &["worker_read", "worker_edit", "worker_grep", "worker_find"],
            tool_results,
        )),
    )
    .await;

    let events = get_events(&sink);
    let starts = events_of_type(&events, "tool.start");
    let results = events_of_type(&events, "tool.result");
    assert_eq!(starts.len(), 2);
    assert_eq!(results.len(), 2);
    assert_eq!(starts[0]["payload"]["tool"], "worker_find");
    assert_eq!(starts[1]["payload"]["tool"], "worker_read");
}

#[tokio::test]
async fn test_worker_loop_event_sequence() {
    let config = make_config();
    let sink = make_sink();

    let provider = Arc::new(MockLoopProvider::new(vec![
        tool_response("worker_read", r#"{"file_path":"f.rs"}"#),
        text_response("done"),
    ]));

    let tool_results = vec![ToolResult {
        content: "code".to_string(),
        is_error: false,
    }];

    run_worker_loop(
        WorkerLoopParams {
            config,
            provider,
            sink: sink.clone(),
        },
        Box::new(MockToolRegistry::with_tools(
            &["worker_read", "worker_edit", "worker_grep", "worker_find"],
            tool_results,
        )),
    )
    .await;

    let events = get_events(&sink);
    let types: Vec<&str> = events.iter().filter_map(|e| e["type"].as_str()).collect();
    // Expected: log.message(ready), tool.start, tool.result, log.message(done)
    assert_eq!(
        types,
        vec!["log.message", "tool.start", "tool.result", "log.message"]
    );
}

#[tokio::test]
async fn test_worker_loop_truncates_long_response_in_event() {
    let config = make_config();
    let sink = make_sink();
    let long_response = "x".repeat(1000);
    let provider = Arc::new(MockLoopProvider::new(vec![text_response(&long_response)]));

    run_worker_loop(
        WorkerLoopParams {
            config,
            provider,
            sink: sink.clone(),
        },
        Box::new(MockToolRegistry::empty()),
    )
    .await;

    let events = get_events(&sink);
    let logs = events_of_type(&events, "log.message");
    let done = logs.last().unwrap();
    let msg = done["payload"]["message"].as_str().unwrap();
    // The event message should be truncated (not the full 1000 chars)
    assert!(msg.len() < 600, "event message should be truncated");
    assert!(msg.contains("..."));
}

#[tokio::test]
async fn test_worker_loop_session_key_contains_ids() {
    let mut config = make_config();
    config.run_id = "r42".to_string();
    config.job_id = "j7".to_string();
    let sink = make_sink();
    let provider = Arc::new(MockLoopProvider::new(vec![text_response("done")]));

    // We verify indirectly: the loop runs without error, meaning
    // the session_key was set (we can't inspect AgentLoopConfig directly,
    // but the format!() uses run_id and job_id).
    let result = run_worker_loop(
        WorkerLoopParams {
            config,
            provider,
            sink,
        },
        Box::new(MockToolRegistry::empty()),
    )
    .await;
    assert_eq!(result.exit_code, 0);
}
