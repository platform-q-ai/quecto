use super::*;
use crate::domain::message::{LlmResponse, ToolCall, UsageInfo};
use crate::domain::provider::ChatRequest;
use std::sync::Mutex as StdMutex;

// ── MockEventSink ──────────────────────────────────────────────────────

/// A test-only event sink that collects emitted events in memory.
#[derive(Debug, Default)]
struct MockEventSink {
    events: Vec<serde_json::Value>,
    seq: u64,
}

impl MockEventSink {
    fn new() -> Self {
        Self::default()
    }

    fn events(&self) -> &[serde_json::Value] {
        &self.events
    }
}

impl WorkerEventSink for MockEventSink {
    fn emit(&mut self, event_type: &str, payload: serde_json::Value) -> Result<u64, String> {
        self.seq += 1;
        self.events.push(serde_json::json!({
            "type": event_type,
            "seq": self.seq,
            "payload": payload,
        }));
        Ok(self.seq)
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

fn make_sink() -> Arc<Mutex<MockEventSink>> {
    Arc::new(Mutex::new(MockEventSink::new()))
}

fn get_events(sink: &Mutex<MockEventSink>) -> Vec<serde_json::Value> {
    sink.lock().unwrap().events().to_vec()
}

fn events_of_type(events: &[serde_json::Value], event_type: &str) -> Vec<serde_json::Value> {
    events
        .iter()
        .filter(|e| e["type"].as_str() == Some(event_type))
        .cloned()
        .collect()
}

// ── Tests ──────────────────────────────────────────────────────────────

#[test]
fn test_build_worker_system_prompt_contains_goal() {
    let prompt = build_worker_system_prompt("Fix the flaky test");
    assert!(prompt.contains("Fix the flaky test"));
}

#[test]
fn test_build_worker_system_prompt_contains_tool_names() {
    let prompt = build_worker_system_prompt("any goal");
    assert!(prompt.contains("worker_read"));
    assert!(prompt.contains("worker_edit"));
    assert!(prompt.contains("worker_grep"));
    assert!(prompt.contains("worker_find"));
}

#[test]
fn test_truncate_args_preview_short() {
    let args = r#"{"path":"test.rs"}"#;
    assert_eq!(truncate_args_preview(args), args);
}

#[test]
fn test_truncate_args_preview_long() {
    let args = "x".repeat(300);
    let result = truncate_args_preview(&args);
    assert_eq!(result.len(), 203);
    assert!(result.ends_with("..."));
}

#[test]
fn test_generate_call_id_unique() {
    let id1 = generate_call_id("worker_read");
    let id2 = generate_call_id("worker_read");
    assert_ne!(id1, id2);
    assert!(id1.starts_with("wc_worker_read_"));
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
            .contains("provider")
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
