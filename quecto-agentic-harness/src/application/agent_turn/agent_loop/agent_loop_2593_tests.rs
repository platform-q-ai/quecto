//! #2593: a reply cut off at the output limit partway through a tool
//! call's arguments is not run and not recorded. The model is told the
//! call was cut off and how to split it, the request is sent again, and
//! the turn goes on. The correction is bounded: past it, the cut-off call
//! is answered as any invalid call is (#2123), and the turn still goes on.
use crate::application::agent_turn::agent_loop::tests::{
    MockProvider, MockRegistry, test_config, text_response,
};
use crate::application::agent_turn::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::application::tools::ports::Tool;
use crate::domain::conversation::value_objects::message::{
    LlmResponse, Message, Role, StopReason, ToolCall,
};
use crate::domain::shared::error::DomainError;
use crate::domain::tool_policy::value_objects::tool::{ToolDefinition, ToolResult};
use std::pin::Pin;
use std::sync::{Arc, Mutex};

/// A `write` tool that records every call it runs.
#[derive(Debug)]
struct RecordingWrite {
    ran: Arc<Mutex<Vec<String>>>,
}

impl Tool for RecordingWrite {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "write".into(),
            description: "records its arguments".into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
        }
    }
    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<ToolResult, DomainError>> + Send + '_>>
    {
        self.ran.lock().unwrap().push(arguments.to_string());
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

/// A `write` call whose `content` the output limit cut off mid-string.
fn cut_off_write() -> LlmResponse {
    let mut response = text_response("");
    response.content = None;
    response.stop_reason = Some(StopReason::MaxTokens);
    response.tool_calls = vec![ToolCall {
        id: "call_write".into(),
        name: "write".into(),
        arguments: r#"{"path":"big.txt","content":"line 1\nline 2\nline 3\nli"#.into(),
    }];
    response
}

struct Run {
    agent: AgentLoopImpl,
    provider: Arc<MockProvider>,
    ran: Arc<Mutex<Vec<String>>>,
}

fn run_with(responses: Vec<Result<LlmResponse, DomainError>>, streaming: bool) -> Run {
    let ran = Arc::new(Mutex::new(Vec::new()));
    let mut registry = MockRegistry::new();
    registry.register(Arc::new(RecordingWrite { ran: ran.clone() }));
    let provider = Arc::new(MockProvider::new_results(responses));
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        streaming,
        max_tokens: 8192,
        ..test_config(provider.clone(), Box::new(registry))
    });
    Run {
        agent,
        provider,
        ran,
    }
}

/// The user messages that tell the model a `write` call was cut off.
fn cut_off_feedback(messages: &[Message]) -> Vec<&Message> {
    messages
        .iter()
        .filter(|m| {
            m.role == Role::User
                && m.content.contains("hit the output limit partway through")
                && m.content.contains("`write`")
        })
        .collect()
}

#[tokio::test]
async fn a_write_cut_off_inside_its_arguments_is_asked_again_and_the_turn_goes_on() {
    for streaming in [false, true] {
        let mut run = run_with(
            vec![Ok(cut_off_write()), Ok(text_response("split it up"))],
            streaming,
        );
        let mut messages = vec![Message::user("write the big file")];
        let result = run
            .agent
            .run_loop(&mut messages)
            .await
            .expect("a cut-off call never ends the turn");
        assert_eq!(result.response, "split it up", "streaming={streaming}");
        assert_eq!(run.provider.request_count(), 2, "streaming={streaming}");
        assert!(
            run.ran.lock().unwrap().is_empty(),
            "the cut-off call is never run"
        );
        let feedback = cut_off_feedback(&messages);
        assert_eq!(feedback.len(), 1, "told once, why: {messages:?}");
        assert_eq!(feedback[0].turn, Some(1), "stamped with the open turn");
        assert!(
            messages.iter().all(|m| m.tool_calls.is_empty()),
            "the cut-off reply is not recorded: {messages:?}"
        );
        assert!(
            result
                .appended_messages
                .iter()
                .any(|m| m.id() == feedback[0].id()),
            "the run added the feedback, so its ledger has it"
        );
    }
}

#[tokio::test]
async fn the_retry_after_a_cut_off_call_may_use_the_model_cap() {
    let run = run_with(vec![Ok(cut_off_write()), Ok(text_response("done"))], false);
    let mut agent = run.agent.with_model_max_tokens(Some(32_768));
    let mut messages = vec![Message::user("write the big file")];
    agent.run_loop(&mut messages).await.unwrap();
    assert_eq!(run.provider.seen_max_tokens(), [8192, 16_384]);
}

#[tokio::test]
async fn a_call_that_keeps_being_cut_off_is_answered_as_invalid_and_the_turn_goes_on() {
    let mut run = run_with(
        vec![
            Ok(cut_off_write()),
            Ok(cut_off_write()),
            Ok(cut_off_write()),
            Ok(text_response("done")),
        ],
        false,
    );
    let mut messages = vec![Message::user("write the big file")];
    let result = run
        .agent
        .run_loop(&mut messages)
        .await
        .expect("the turn goes on past the correction budget");
    assert_eq!(result.response, "done");
    assert_eq!(run.provider.request_count(), 4);
    assert_eq!(
        cut_off_feedback(&messages).len(),
        2,
        "corrected twice, then answered as an invalid call"
    );
    let answer = messages
        .iter()
        .find(|m| m.role == Role::Tool)
        .expect("the third cut-off call gets a tool result");
    assert!(answer.content.contains("not a JSON object"), "{answer:?}");
    assert!(run.ran.lock().unwrap().is_empty(), "never run");
}

#[tokio::test]
async fn a_complete_call_at_the_output_limit_runs() {
    let mut complete = cut_off_write();
    complete.tool_calls[0].arguments = r#"{"path":"a.txt","content":"all of it"}"#.into();
    let mut run = run_with(vec![Ok(complete), Ok(text_response("done"))], false);
    let mut messages = vec![Message::user("write it")];
    run.agent.run_loop(&mut messages).await.unwrap();
    assert_eq!(run.ran.lock().unwrap().len(), 1, "a whole call is a call");
    assert!(cut_off_feedback(&messages).is_empty());
}

#[tokio::test]
async fn invalid_arguments_without_the_output_limit_are_no_cut_off() {
    let mut malformed = cut_off_write();
    malformed.stop_reason = Some(StopReason::ToolUse);
    let mut run = run_with(vec![Ok(malformed), Ok(text_response("done"))], false);
    let mut messages = vec![Message::user("write it")];
    run.agent.run_loop(&mut messages).await.unwrap();
    assert!(cut_off_feedback(&messages).is_empty());
    assert_eq!(
        run.provider.request_count(),
        2,
        "answered as invalid (#2123)"
    );
}

/// Review of #2598: the correction budget counts consecutive cut-offs; an
/// accepted reply between them gives the next ones their budget back.
#[tokio::test]
async fn an_accepted_reply_restores_the_correction_budget() {
    let mut complete = cut_off_write();
    complete.tool_calls[0].arguments = r#"{"path":"a.txt","content":"all of it"}"#.into();
    let mut run = run_with(
        vec![
            Ok(cut_off_write()),
            Ok(cut_off_write()),
            Ok(complete),
            Ok(cut_off_write()),
            Ok(cut_off_write()),
            Ok(text_response("done")),
        ],
        false,
    );
    let mut messages = vec![Message::user("write the big file")];
    run.agent.run_loop(&mut messages).await.unwrap();
    assert_eq!(cut_off_feedback(&messages).len(), 4, "two, then two more");
    assert_eq!(run.ran.lock().unwrap().len(), 1, "only the whole call ran");
}

/// A cut-off call answered as invalid does not restore the budget: the
/// next cut-off in a row is answered as invalid too.
#[tokio::test]
async fn a_cut_off_answered_as_invalid_keeps_the_budget_spent() {
    let mut run = run_with(
        vec![
            Ok(cut_off_write()),
            Ok(cut_off_write()),
            Ok(cut_off_write()),
            Ok(cut_off_write()),
            Ok(text_response("done")),
        ],
        false,
    );
    let mut messages = vec![Message::user("write the big file")];
    run.agent.run_loop(&mut messages).await.unwrap();
    assert_eq!(cut_off_feedback(&messages).len(), 2);
    let invalid = messages
        .iter()
        .filter(|m| m.role == Role::Tool && m.content.contains("not a JSON object"))
        .count();
    assert_eq!(invalid, 2, "the third and fourth are answered as invalid");
}

/// Review of #2598: whole calls in a reply cut off later are not run
/// either (the reply is dropped whole), and the feedback says so.
#[tokio::test]
async fn whole_calls_in_a_cut_off_reply_are_not_run_and_the_feedback_says_so() {
    let mut reply = cut_off_write();
    reply.tool_calls.insert(
        0,
        ToolCall {
            id: "call_first".into(),
            name: "write".into(),
            arguments: r#"{"path":"small.txt","content":"whole"}"#.into(),
        },
    );
    let mut run = run_with(vec![Ok(reply), Ok(text_response("done"))], false);
    let mut messages = vec![Message::user("write both files")];
    run.agent.run_loop(&mut messages).await.unwrap();
    assert!(
        run.ran.lock().unwrap().is_empty(),
        "no call of the reply ran"
    );
    let feedback = cut_off_feedback(&messages);
    assert_eq!(feedback.len(), 1);
    assert!(
        feedback[0]
            .content
            .contains("none of its tool calls were run"),
        "{}",
        feedback[0].content
    );
}

/// Review round 2 of #2598: past the budget, a call cut off before any
/// argument text is answered as invalid (#2123) like any cut-off call;
/// it never runs as a call with no arguments.
#[tokio::test]
async fn past_the_budget_a_call_cut_off_with_no_arguments_never_runs() {
    let mut empty = cut_off_write();
    empty.tool_calls[0].arguments = String::new();
    let mut run = run_with(
        vec![
            Ok(empty.clone()),
            Ok(empty.clone()),
            Ok(empty),
            Ok(text_response("done")),
        ],
        false,
    );
    let mut messages = vec![Message::user("write the big file")];
    run.agent.run_loop(&mut messages).await.unwrap();
    assert!(run.ran.lock().unwrap().is_empty(), "never run as `{{}}`");
    assert_eq!(cut_off_feedback(&messages).len(), 2);
    let answers: Vec<&str> = messages
        .iter()
        .filter(|m| m.role == Role::Tool)
        .map(|m| m.content.as_str())
        .collect();
    assert_eq!(answers.len(), 1, "{answers:?}");
    assert!(answers[0].contains("not a JSON object"), "{answers:?}");
}
