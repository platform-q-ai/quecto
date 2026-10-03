//! #2226: a workflow child answers its task, its engine then nudges it, and
//! the supervisor's plain `get_messages` still returns the answer. The
//! nudges go out only while the model is shown the workflow tool.
use super::super::dispatch_test_env::{
    DispatchTestEnv, make_completed_feature_workflow, make_workflow,
};
use crate::application::providers::ports::{ChatRequest, LlmProvider};
use crate::domain::error::DomainError;
use crate::domain::message::{LlmResponse, Message, Role};
use crate::domain::turn_origin::TurnOrigin;
use crate::infrastructure::tools::workflow_tool::WORKFLOW_TOOL_NAME;
use crate::interface::cli::uds_session::{HISTORY_PAGE_SIZE, messages_page_json};
use crate::interface::shared::WorkflowStateHandle;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

const ANSWER: &str = "REPORT 2226";
const HANDOFF: &str = "HANDOFF 2226";
const STATUS: &str = "status";

/// Answers the task with [`ANSWER`] and the completion nudge with
/// [`HANDOFF`]. Each progress nudge gets [`STATUS`] (no progress); with
/// `cut_off_in_nudges` a nudge turn first calls the workflow tool, then is
/// cut off at the output limit with nothing visible, so the loop adds its
/// own feedback message inside the nudge turn, and only then says
/// [`STATUS`] (review probe P1).
#[derive(Debug, Default)]
struct AnswerThenStatus {
    calls: AtomicU32,
    cut_off_in_nudges: bool,
}

fn text(reply: &str) -> LlmResponse {
    LlmResponse {
        content: Some(reply.to_string()),
        tool_calls: vec![],
        usage: None,
        stop_reason: None,
        thinking_blocks: vec![],
    }
}

impl AnswerThenStatus {
    fn respond(&self, messages: &[Message]) -> LlmResponse {
        use crate::domain::message::{StopReason, ToolCall};
        use crate::domain::turn_origin::TurnOrigin;
        let last = messages.last().expect("a turn has messages");
        let opener = messages
            .iter()
            .rev()
            .find(|m| m.role == Role::User && m.turn.is_none())
            .expect("every turn opens with a user message");
        match (opener.turn_origin, opener.content.as_str()) {
            (TurnOrigin::Instruction, "task") => text(ANSWER),
            (TurnOrigin::Instruction, _) => text(HANDOFF),
            (TurnOrigin::ProgressNudge, _) if self.cut_off_in_nudges => match last.role {
                Role::User if last.turn.is_none() => LlmResponse {
                    tool_calls: vec![ToolCall {
                        id: format!("call-{}", messages.len()),
                        name: WORKFLOW_TOOL_NAME.into(),
                        arguments: r#"{"action":"status"}"#.into(),
                    }],
                    ..text("")
                },
                Role::Tool => LlmResponse {
                    stop_reason: Some(StopReason::MaxTokens),
                    ..text("")
                },
                _ => text(STATUS),
            },
            (TurnOrigin::ProgressNudge, _) => text(STATUS),
            (TurnOrigin::Unknown | TurnOrigin::Unrecognised, _) => {
                panic!("every opener is stamped: {opener:?}")
            }
        }
    }
}

impl LlmProvider for AnswerThenStatus {
    fn route_order(&self) -> Vec<String> {
        vec![self.name().to_string()]
    }
    fn name(&self) -> &str {
        "answer-then-status"
    }

    fn chat<'a>(
        &'a self,
        request: ChatRequest<'a>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>,
    > {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let response = self.respond(request.messages);
        Box::pin(async move { Ok(response) })
    }
}

fn env_over(workflow: WorkflowStateHandle) -> (DispatchTestEnv, Arc<AnswerThenStatus>) {
    let provider = Arc::new(AnswerThenStatus::default());
    (DispatchTestEnv::new(workflow, provider.clone()), provider)
}

fn selected_workflow() -> WorkflowStateHandle {
    let workflow = make_workflow();
    workflow
        .lock()
        .unwrap()
        .select_template("feature", None)
        .unwrap();
    workflow
}

/// Hand the child its task as a spawn does and let the idle drain run the
/// workflow nudges that follow it.
async fn run_task(env: &mut DispatchTestEnv) {
    let mut ctx = env.ctx();
    super::super::uds_dispatch::handle_follow_up(
        &mut ctx,
        Some("task"),
        "follow_up",
        "task".into(),
    )
    .await;
}

/// The supervisor's first plain `get_messages`, through its own
/// `AgentCmdTool`, of a mock child serving `env`'s newest page as the
/// child's state serves it: the delivered messages' contents.
async fn first_default_report(env: &DispatchTestEnv) -> Vec<String> {
    use crate::application::tools::ports::Tool;
    use crate::infrastructure::tools::agent_cmd::AgentCmdTool;
    use crate::infrastructure::tools::subagent_registry::SubagentEntry;
    let page = messages_page_json(&env.messages, HISTORY_PAGE_SIZE, None);
    let tmp = tempfile::TempDir::new().unwrap();
    let socket = tmp.path().join("child.sock");
    let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    let child = std::thread::spawn(move || {
        use std::io::Write;
        let (mut stream, _) = listener.accept().unwrap();
        let line = crate::infrastructure::test_support::read_framed_command(&stream).unwrap();
        let request: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(request["type"], "get_messages");
        let response = serde_json::json!({"type": "response", "id": request["id"],
            "command": "get_messages", "success": true, "data": page});
        stream
            .write_all(format!("{response}\n").as_bytes())
            .expect("the mock child writes its whole reply");
    });
    let registry = AgentCmdTool::new_registry();
    registry
        .lock()
        .unwrap()
        .insert("w1".into(), SubagentEntry::new(socket, 0));
    let tool = AgentCmdTool::new(registry);
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        tool.execute(r#"{"agent_id":"w1","command":"get_messages"}"#),
    )
    .await
    .expect("the mock child answers within 10 s")
    .expect("agent_cmd answers");
    child.join().unwrap();
    assert!(!result.is_error, "{}", result.content);
    let report: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert!(report["data"].get("reportIncomplete").is_none(), "{report}");
    report["data"]["messages"]
        .as_array()
        .expect("a report is delivered")
        .iter()
        .map(|m| m["content"].as_str().unwrap_or_default().to_string())
        .collect()
}

fn assistant_replies(messages: &[Message]) -> Vec<&str> {
    messages
        .iter()
        .filter(|m| m.role == Role::Assistant)
        .map(|m| m.content.as_str())
        .collect()
}

#[tokio::test]
async fn nudge_replies_after_the_answer_never_replace_it_as_the_report() {
    let (mut env, provider) = env_over(selected_workflow());
    run_task(&mut env).await;
    assert_eq!(
        assistant_replies(&env.messages),
        [ANSWER, STATUS, STATUS, STATUS],
        "the task turn and three no-progress nudge turns ran"
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 4);
    let origins: Vec<TurnOrigin> = env.messages.iter().map(|m| m.turn_origin).collect();
    use TurnOrigin::{Instruction as I, ProgressNudge as N};
    assert_eq!(
        origins,
        [I, I, N, N, N, N, N, N],
        "every message is stamped"
    );
    // The ledger's full copies, which `get_message` serves, carry the stamps.
    let state = env.sessions.active_session.read().await;
    for message in &env.messages {
        let copy = state.conversation().lookup(&message.id().to_string());
        assert_eq!(copy.map(|c| c.turn_origin), Some(message.turn_origin));
    }
    drop(state);
    assert_eq!(first_default_report(&env).await, [ANSWER]);
}

#[tokio::test]
async fn the_completion_nudge_asks_for_the_report_its_reply_is() {
    let (mut env, _provider) = env_over(make_completed_feature_workflow());
    run_task(&mut env).await;
    assert_eq!(assistant_replies(&env.messages), [ANSWER, HANDOFF]);
    assert!(
        env.messages
            .iter()
            .all(|m| m.turn_origin == TurnOrigin::Instruction)
    );
    assert_eq!(first_default_report(&env).await, [HANDOFF]);
}

#[tokio::test]
async fn no_progress_nudge_goes_out_while_the_workflow_tool_is_hidden() {
    let (mut env, provider) = env_over(selected_workflow());
    env.agent.unregister_runtime_tool(WORKFLOW_TOOL_NAME);
    assert!(!env.agent.is_tool_model_visible(WORKFLOW_TOOL_NAME));
    run_task(&mut env).await;
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1, "the task only");
    assert_eq!(assistant_replies(&env.messages), [ANSWER]);
    assert_eq!(first_default_report(&env).await, [ANSWER]);
}

/// Review probe P1: the loop's own feedback inside a nudge turn (here after
/// a reply cut off at the output limit) is not an instruction, so the reply
/// that follows it is still progress, never the report.
#[tokio::test]
async fn loop_feedback_inside_a_nudge_turn_never_makes_its_reply_the_report() {
    let provider = Arc::new(AnswerThenStatus {
        cut_off_in_nudges: true,
        ..AnswerThenStatus::default()
    });
    let mut env = DispatchTestEnv::new(selected_workflow(), provider.clone());
    run_task(&mut env).await;
    let feedback: Vec<&Message> = env
        .messages
        .iter()
        .filter(|m| m.role == Role::User && m.turn.is_some())
        .collect();
    assert!(!feedback.is_empty(), "the loop added its own feedback");
    assert!(
        feedback
            .iter()
            .all(|m| m.turn_origin == TurnOrigin::ProgressNudge)
    );
    assert!(assistant_replies(&env.messages).contains(&STATUS));
    assert_eq!(first_default_report(&env).await, [ANSWER]);
}
