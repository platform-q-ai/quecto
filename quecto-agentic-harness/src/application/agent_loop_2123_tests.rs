//! #2123: a tool call whose arguments are not a JSON object is answered
//! with an error and never run; empty arguments run as `{}`.
use super::*;

#[derive(Debug)]
struct RecordingTool {
    received: Arc<Mutex<Vec<String>>>,
}

impl Tool for RecordingTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "bash".into(),
            description: "records its arguments".into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
        }
    }
    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<ToolResult, DomainError>> + Send + '_>>
    {
        self.received.lock().unwrap().push(arguments.to_string());
        Box::pin(async {
            Ok(ToolResult {
                content: "ran".into(),
                is_error: false,
                image_blocks: vec![],
                delivery_metadata: None,
            })
        })
    }
}

fn call_then_done(arguments: &str) -> Vec<Result<LlmResponse, DomainError>> {
    let mut call = text_response("");
    call.content = None;
    call.tool_calls = vec![ToolCall {
        id: "call_1".into(),
        name: "bash".into(),
        arguments: arguments.into(),
    }];
    vec![Ok(call), Ok(text_response("done"))]
}

async fn run(arguments: &str) -> (Vec<String>, Vec<Message>) {
    run_with(arguments, false).await
}

async fn run_with(arguments: &str, bash_disabled: bool) -> (Vec<String>, Vec<Message>) {
    let received = Arc::new(Mutex::new(Vec::new()));
    let mut registry = MockRegistry::new();
    registry.register(Arc::new(RecordingTool {
        received: received.clone(),
    }));
    let provider = Arc::new(MockProvider::new_results(call_then_done(arguments)));
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(registry),
        model: "test-model".to_string(),
        max_tokens: 1024,
        temperature: 0.7,
        retention: None,
        session_key: String::new(),
        context_collapse_after_tool_calls: u32::MAX,
        max_context_tokens: 190_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
    });
    if bash_disabled {
        agent
            .tool_policy_state
            .lock()
            .unwrap()
            .disabled_tools
            .insert("bash".to_string());
    }
    let mut messages = vec![Message::user("run it")];
    let result = agent
        .run_loop(&mut messages)
        .await
        .expect("the turn completes");
    assert_eq!(result.response, "done");
    let received = received.lock().unwrap().clone();
    (received, messages)
}

#[tokio::test]
async fn a_call_with_invalid_json_arguments_is_answered_with_an_error_and_not_run() {
    let truncated = r#"{"command":"cat /very/lo"#;
    let (received, messages) = run(truncated).await;
    assert!(received.is_empty(), "the tool must not run: {received:?}");
    let answer = messages
        .iter()
        .find(|m| m.role == Role::Tool)
        .expect("the call gets a tool result");
    assert!(
        answer.content.contains("not a JSON object") && answer.content.contains(truncated),
        "{}",
        answer.content
    );
}

#[tokio::test]
async fn a_call_with_empty_arguments_runs_with_an_empty_object() {
    let (received, _) = run("").await;
    assert_eq!(received, ["{}"]);
}

#[tokio::test]
async fn a_call_with_object_arguments_runs_unchanged() {
    let (received, _) = run(r#"{"command":"ls"}"#).await;
    assert_eq!(received, [r#"{"command":"ls"}"#]);
}

fn tool_answer(messages: &[Message]) -> String {
    messages
        .iter()
        .find(|m| m.role == Role::Tool)
        .expect("the call gets a tool result")
        .content
        .clone()
}

#[tokio::test]
async fn valid_json_that_is_not_an_object_is_refused_with_accurate_wording() {
    for value in ["[1,2]", r#""ls""#, "null"] {
        let (received, messages) = run(value).await;
        assert!(received.is_empty(), "{value}");
        assert!(
            tool_answer(&messages).contains("not a JSON object"),
            "{value}"
        );
    }
}

#[tokio::test]
async fn a_long_cut_off_call_shows_both_ends_of_what_was_received() {
    let raw = format!(r#"{{"command":"echo {}BREAKS-HERE"#, "x".repeat(2000));
    let (_, messages) = run(&raw).await;
    let answer = tool_answer(&messages);
    assert!(answer.contains(r#"{"command":"echo"#), "keeps the start");
    assert!(
        answer.contains("BREAKS-HERE"),
        "keeps the end, where it broke"
    );
    assert!(answer.len() < 1500, "bounded: {}", answer.len());
}

#[tokio::test]
async fn a_disabled_tool_is_reported_as_disabled_not_as_resend_it() {
    let (received, messages) = run_with(r#"{"command":"cat /very/lo"#, true).await;
    assert!(received.is_empty());
    let answer = tool_answer(&messages);
    assert!(answer.contains("disabled by runtime policy"), "{answer}");
    assert!(!answer.contains("Resend"), "{answer}");
}
