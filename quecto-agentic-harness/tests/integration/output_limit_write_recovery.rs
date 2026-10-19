//! #2593 reproduction: a large `write` cut off at the output limit must
//! never end the turn. A real Codex adapter streams a `write` call whose
//! arguments stop at `response.incomplete` (`max_output_tokens`); the agent
//! loop tells the model the call was cut off and asks again, and the turn
//! ends with the model's next reply.
use quecto::application::agent_turn::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use quecto::application::agent_turn::ports::AgentLoop;
use quecto::application::tools::ports::Tool;
use quecto::domain::conversation::value_objects::message::{Message, Role};
use quecto::domain::shared::error::DomainError;
use quecto::domain::tool_policy::value_objects::tool::{
    ToolDefinition, ToolProfileContext, ToolResult,
};
use quecto::infrastructure::{providers::codex::CodexProvider, tools::registry::ToolRegistryImpl};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A `write` tool that counts the calls it runs.
#[derive(Debug)]
struct CountingWrite(Arc<AtomicUsize>);

impl Tool for CountingWrite {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "write".into(),
            description: "counts its calls".into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
        }
    }
    fn execute(
        &self,
        _arguments: &str,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<ToolResult, DomainError>> + Send + '_>>
    {
        self.0.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            Ok(ToolResult {
                content: "written".into(),
                is_error: false,
                image_blocks: vec![],
                delivery_metadata: None,
            })
        })
    }
}
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

fn sse(events: &[serde_json::Value]) -> String {
    events.iter().map(|e| format!("data: {e}\n\n")).collect()
}

fn cut_off_write(reason: Option<&str>) -> String {
    let details = match reason {
        Some(reason) => {
            serde_json::json!({"status":"incomplete","incomplete_details":{"reason":reason}})
        }
        None => serde_json::json!({"status":"incomplete"}),
    };
    sse(&[
        serde_json::json!({"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","call_id":"call_1","name":"write","arguments":""}}),
        serde_json::json!({"type":"response.function_call_arguments.delta","output_index":0,"delta":"{\"path\":\"big.txt\",\"content\":\"line 1\\nli"}),
        serde_json::json!({"type":"response.incomplete","response":details}),
    ])
}

fn answer() -> String {
    sse(&[
        serde_json::json!({"type":"response.output_text.delta","delta":"I will write it in parts"}),
        serde_json::json!({"type":"response.completed","response":{"status":"completed"}}),
    ])
}

/// One turn against a Codex server whose first reply is a `write` cut
/// off with `reason` and whose second is an answer.
async fn turn(
    reason: Option<&'static str>,
) -> (Result<String, DomainError>, usize, usize, Vec<Message>) {
    let server = MockServer::start().await;
    let sent = Arc::new(AtomicUsize::new(0));
    let count = sent.clone();
    Mock::given(method("POST"))
        .respond_with(move |_: &wiremock::Request| {
            let body = match count.fetch_add(1, Ordering::SeqCst) {
                0 => cut_off_write(reason),
                _ => answer(),
            };
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body)
        })
        .mount(&server)
        .await;
    let ran = Arc::new(AtomicUsize::new(0));
    let mut registry = ToolRegistryImpl::new();
    assert!(registry.register(Arc::new(CountingWrite(ran.clone()))));
    let provider = CodexProvider::new("fixture".into(), "acct".into(), Some(server.uri()));
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: Arc::new(provider),
        tool_registry: Box::new(registry),
        model: "fixture".into(),
        max_tokens: 8192,
        temperature: 0.0,
        retention: None,
        session_key: "fixture".into(),
        max_context_tokens: 100_000,
        progress_callback: None,
        streaming: true,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_marks: Default::default(),
        model_context_window: None,
        tool_profile_context: ToolProfileContext::Parent,
    });
    let mut messages = vec![
        Message::system("You write files."),
        Message::user("write the big file"),
    ];
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        agent.process(&mut messages),
    )
    .await
    .expect("the turn finishes")
    .map(|result| result.response);
    (
        result,
        sent.load(Ordering::SeqCst),
        ran.load(Ordering::SeqCst),
        messages,
    )
}

#[tokio::test]
async fn a_write_cut_off_at_the_output_limit_does_not_end_the_turn() {
    let (result, sent, ran, messages) = turn(Some("max_output_tokens")).await;
    let response = result.expect("a write cut off at the output limit never ends the turn");
    assert_eq!(response, "I will write it in parts");
    assert_eq!(sent, 2, "asked once more");
    assert_eq!(ran, 0, "the cut-off write is never run");
    assert!(
        messages
            .iter()
            .any(|m| m.role == Role::User && m.content.contains("`write`")),
        "the model is told the write was cut off: {messages:?}"
    );
}

/// An incomplete reply that names no reason is no output-limit stop: it
/// stays a provider failure (the allowlist holds), and nothing runs.
#[tokio::test]
async fn an_incomplete_reply_without_a_reason_stays_a_failure() {
    let (result, sent, ran, _) = turn(None).await;
    let error = result.expect_err("an unexplained incomplete reply ends the turn");
    assert!(error.to_string().contains("response.incomplete"), "{error}");
    assert_eq!(sent, 1);
    assert_eq!(ran, 0);
}
