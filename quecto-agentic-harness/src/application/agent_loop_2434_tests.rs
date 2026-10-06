//! #2434: a reply to tool results with nothing in it ends the turn as a
//! final answer with no text: never asked again, never an `empty_stream`
//! error, and recorded on its request's observation. A reply that must have
//! output (to a prompt, a steer, a follow-up) keeps the empty-stream error
//! and its retries, and a reply stopped at the output limit is never a clean
//! empty end (#2124).
use super::*;
use crate::domain::inference::services::provider_error::{
    ProviderErrorClass, classify_provider_error,
};
use crate::domain::inference::value_objects::provider::StreamEvent;
use crate::domain::message::{StopReason, ThinkingBlock};

fn empty_reply(stop_reason: Option<StopReason>) -> LlmResponse {
    LlmResponse {
        content: None,
        tool_calls: vec![],
        usage: None,
        stop_reason,
        thinking_blocks: vec![],
    }
}

fn background_call() -> LlmResponse {
    LlmResponse {
        content: None,
        tool_calls: vec![ToolCall {
            id: "call_background".into(),
            name: "background".into(),
            arguments: "{}".into(),
        }],
        usage: None,
        stop_reason: Some(StopReason::ToolUse),
        thinking_blocks: vec![],
    }
}

/// Only reasoning, shown, at the output limit (#2124).
fn reasoning_only_at_the_limit() -> LlmResponse {
    LlmResponse {
        thinking_blocks: vec![ThinkingBlock::Normal {
            thinking: "let me think about this at great length".into(),
            signature: String::new(),
        }],
        ..empty_reply(Some(StopReason::MaxTokens))
    }
}

fn registry() -> MockRegistry {
    let mut registry = MockRegistry::new();
    registry.register(Arc::new(MockTool::new(
        "background",
        "started — end your turn now",
    )));
    registry
}

fn done(response: LlmResponse) -> Vec<StreamEvent> {
    vec![StreamEvent::Done(response)]
}

fn streaming(replies: Vec<Vec<StreamEvent>>) -> (AgentLoopImpl, Arc<MockStreamingProvider>) {
    let provider = Arc::new(MockStreamingProvider::new(replies));
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        streaming: true,
        ..test_config(provider.clone(), Box::new(registry()))
    });
    (agent, provider)
}

fn whole(replies: Vec<LlmResponse>) -> (AgentLoopImpl, Arc<MockProvider>) {
    let provider = Arc::new(MockProvider::new(replies));
    let agent = AgentLoopImpl::new(test_config(provider.clone(), Box::new(registry())));
    (agent, provider)
}

/// The turn ended on the tool result: nothing empty was recorded as the
/// model's reply, and the run's messages are the call and its result.
fn assert_ended_on_the_tool_result(messages: &[Message], result: &AgentResult) {
    assert_eq!(result.response, "");
    assert!(!result.iteration_limit_reached);
    assert_eq!(result.tool_iterations, 1);
    assert_eq!(
        messages.last().map(|m| m.role.clone()),
        Some(Role::Tool),
        "{messages:?}"
    );
    let roles: Vec<Role> = result
        .appended_messages
        .iter()
        .map(|m| m.role.clone())
        .collect();
    assert_eq!(roles, vec![Role::Assistant, Role::Tool]);
}

#[tokio::test]
async fn an_empty_reply_to_tool_results_ends_the_turn_without_a_retry() {
    let (mut agent, provider) = streaming(vec![
        done(background_call()),
        done(empty_reply(Some(StopReason::EndTurn))),
    ]);
    let mut messages = vec![Message::user("start the job")];
    let result = agent
        .process(&mut messages)
        .await
        .expect("an empty reply to tool results ends the turn");
    assert_ended_on_the_tool_result(&messages, &result);
    assert_eq!(
        provider.request_count(),
        2,
        "the reply is never asked again"
    );
    let observations = agent.take_request_observations();
    assert_eq!(observations.len(), 2);
    assert!(!observations[0].ended_empty_after_tools);
    let ended = &observations[1];
    assert!(ended.ended_empty_after_tools, "{ended:?}");
    assert_eq!(ended.outcome, "succeeded");
    assert_eq!(ended.error_class, None);
    assert_eq!(ended.instrumented_attempts, 1);
}

#[tokio::test]
async fn an_empty_reply_to_tool_results_ends_the_turn_on_the_whole_reply_path() {
    let (mut agent, provider) = whole(vec![
        background_call(),
        empty_reply(Some(StopReason::EndTurn)),
    ]);
    let mut messages = vec![Message::user("start the job")];
    let result = agent
        .process(&mut messages)
        .await
        .expect("an empty reply to tool results ends the turn");
    assert_ended_on_the_tool_result(&messages, &result);
    assert_eq!(provider.request_count(), 2);
    let observations = agent.take_request_observations();
    assert!(observations[1].ended_empty_after_tools, "{observations:?}");
}

/// A Responses API reply holds encrypted reasoning the user never sees:
/// with nothing else, it is an empty reply.
#[tokio::test]
async fn encrypted_reasoning_alone_after_tool_results_ends_the_turn() {
    let reply = LlmResponse {
        thinking_blocks: vec![ThinkingBlock::EncryptedReasoning {
            origin: String::new(),
            leads_to: None,
            item: r#"{"type":"reasoning","encrypted_content":"opaque"}"#.into(),
        }],
        ..empty_reply(Some(StopReason::EndTurn))
    };
    let (mut agent, provider) = streaming(vec![done(background_call()), done(reply)]);
    let mut messages = vec![Message::user("start the job")];
    let result = agent.process(&mut messages).await.expect("the turn ends");
    assert_ended_on_the_tool_result(&messages, &result);
    assert_eq!(provider.request_count(), 2);
}

#[tokio::test]
async fn an_empty_first_reply_to_a_prompt_is_still_retried() {
    let (mut agent, provider) = streaming(vec![
        done(empty_reply(Some(StopReason::EndTurn))),
        done(text_response("the answer")),
    ]);
    let mut messages = vec![Message::user("question")];
    let result = agent
        .process(&mut messages)
        .await
        .expect("the retry answers");
    assert_eq!(result.response, "the answer");
    assert_eq!(provider.request_count(), 2);
    let observations = agent.take_request_observations();
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0].instrumented_attempts, 2);
    assert!(!observations[0].ended_empty_after_tools);
}

#[tokio::test]
async fn an_empty_first_reply_every_time_is_still_an_empty_stream_error() {
    let empty = || done(empty_reply(Some(StopReason::EndTurn)));
    let (mut agent, provider) = streaming(vec![empty(), empty(), empty()]);
    let mut messages = vec![Message::user("question")];
    let error = agent
        .process(&mut messages)
        .await
        .expect_err("a prompt must be answered");
    assert_eq!(
        classify_provider_error(&error),
        ProviderErrorClass::EmptyStream,
        "{error}"
    );
    assert_eq!(provider.request_count(), 3);
    let observations = agent.take_request_observations();
    assert_eq!(
        observations[0].error_class,
        Some(ProviderErrorClass::EmptyStream)
    );
    assert!(!observations[0].ended_empty_after_tools);
}

/// A steer or a follow-up after tool results is something new to answer.
#[tokio::test]
async fn an_empty_reply_to_a_steer_after_tool_results_is_still_retried() {
    let (mut agent, provider) = streaming(vec![
        done(empty_reply(Some(StopReason::EndTurn))),
        done(text_response("about ten minutes")),
    ]);
    let mut call = Message::assistant("", background_call().tool_calls);
    call.stop_reason = Some(StopReason::ToolUse);
    let mut messages = vec![
        Message::user("start the job"),
        call,
        Message::tool("call_background", "started — end your turn now"),
        Message::user("how long will it take?"),
    ];
    let result = agent
        .process(&mut messages)
        .await
        .expect("the retry answers");
    assert_eq!(result.response, "about ten minutes");
    assert_eq!(provider.request_count(), 2);
}

/// An empty reply at the output limit was cut off, not finished (#2124).
#[tokio::test]
async fn an_empty_reply_to_tool_results_at_the_output_limit_is_still_an_error() {
    let (mut agent, provider) = streaming(vec![
        done(background_call()),
        done(empty_reply(Some(StopReason::MaxTokens))),
    ]);
    let mut messages = vec![Message::user("start the job")];
    let error = agent
        .process(&mut messages)
        .await
        .expect_err("a reply cut off at the limit is no answer");
    assert!(
        error.to_string().contains("stop_reason=max_tokens"),
        "{error}"
    );
    assert_eq!(provider.request_count(), 2);
    let observations = agent.take_request_observations();
    assert!(!observations[1].ended_empty_after_tools);
}

/// A reply that does not say why it stopped is not known to have finished:
/// it stays an empty stream, retried.
#[tokio::test]
async fn an_empty_reply_to_tool_results_without_a_stop_reason_is_still_retried() {
    let (mut agent, provider) = streaming(vec![
        done(background_call()),
        done(empty_reply(None)),
        done(text_response("started it")),
    ]);
    let mut messages = vec![Message::user("start the job")];
    let result = agent
        .process(&mut messages)
        .await
        .expect("the retry answers");
    assert_eq!(result.response, "started it");
    assert_eq!(provider.request_count(), 3);
    let observations = agent.take_request_observations();
    assert!(!observations[1].ended_empty_after_tools);
}

#[tokio::test]
async fn a_reasoning_only_reply_to_tool_results_at_the_limit_is_asked_again() {
    let (mut agent, provider) = streaming(vec![
        done(background_call()),
        done(reasoning_only_at_the_limit()),
        done(text_response("the answer")),
    ]);
    let mut messages = vec![Message::user("start the job")];
    let result = agent
        .process(&mut messages)
        .await
        .expect("the turn completes");
    assert_eq!(result.response, "the answer");
    assert_eq!(provider.request_count(), 3);
    assert!(
        messages
            .iter()
            .any(|m| m.role == Role::User && m.content.contains("output limit")),
        "the model is told why: {messages:?}"
    );
}

#[tokio::test]
async fn a_reasoning_only_reply_to_tool_results_at_the_limit_twice_fails_the_turn() {
    let (mut agent, provider) = streaming(vec![
        done(background_call()),
        done(reasoning_only_at_the_limit()),
        done(reasoning_only_at_the_limit()),
    ]);
    let mut messages = vec![Message::user("start the job")];
    let error = agent
        .process(&mut messages)
        .await
        .expect_err("never an empty answer");
    assert!(
        error.to_string().contains("stop_reason=max_tokens"),
        "{error}"
    );
    assert_eq!(provider.request_count(), 3);
}

/// Streams each scripted reply as its `Done` event and keeps the roles of
/// every conversation it was sent.
#[derive(Debug)]
struct Recorder {
    replies: std::sync::Mutex<Vec<LlmResponse>>,
    sent: std::sync::Mutex<Vec<Vec<Role>>>,
}

impl Recorder {
    fn new(replies: Vec<LlmResponse>) -> Self {
        Self {
            replies: std::sync::Mutex::new(replies),
            sent: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn sent(&self) -> Vec<Vec<Role>> {
        self.sent.lock().unwrap().clone()
    }
}

impl LlmProvider for Recorder {
    fn route_order(&self) -> Vec<String> {
        vec![self.name().to_string()]
    }
    fn name(&self) -> &str {
        "recorder-2434"
    }
    fn chat(
        &self,
        _request: ChatRequest<'_>,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<LlmResponse, DomainError>> + Send + '_>>
    {
        Box::pin(async { Err(DomainError::Provider("streamed only".into())) })
    }
    fn chat_stream_incremental(
        &self,
        request: ChatRequest<'_>,
    ) -> Pin<
        Box<dyn std::future::Future<Output = tokio::sync::mpsc::Receiver<StreamEvent>> + Send + '_>,
    > {
        let roles = request.messages.iter().map(|m| m.role.clone()).collect();
        self.sent.lock().unwrap().push(roles);
        let reply = self.replies.lock().unwrap().remove(0);
        Box::pin(async move {
            let (tx, rx) = tokio::sync::mpsc::channel(4);
            let _ = tx.send(StreamEvent::Done(reply)).await;
            rx
        })
    }
}

/// Review round 1 (L2): after a turn that ended empty, the next prompt is
/// sent after the tool results, with no empty reply between them, and its
/// own empty first reply is still an empty stream.
#[tokio::test]
async fn the_prompt_after_an_empty_end_follows_the_tool_results_and_must_be_answered() {
    let empty = || empty_reply(Some(StopReason::EndTurn));
    let provider = Arc::new(Recorder::new(vec![
        background_call(),
        empty(),
        empty(),
        empty(),
        empty(),
    ]));
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        streaming: true,
        ..test_config(provider.clone(), Box::new(registry()))
    });
    let mut messages = vec![Message::user("start the job")];
    agent
        .process(&mut messages)
        .await
        .expect("the first prompt's turn ends empty");
    messages.push(Message::user("is it done?"));
    let error = agent
        .process(&mut messages)
        .await
        .expect_err("a prompt must be answered");
    assert_eq!(
        classify_provider_error(&error),
        ProviderErrorClass::EmptyStream,
        "{error}"
    );
    let sent = provider.sent();
    assert_eq!(sent.len(), 5, "two for the first prompt, three attempts");
    let expected = vec![Role::User, Role::Assistant, Role::Tool, Role::User];
    for attempt in &sent[2..] {
        assert_eq!(attempt, &expected);
    }
}

/// Review round 1: a reply of whitespace alone has nothing in it.
#[tokio::test]
async fn a_whitespace_reply_to_tool_results_ends_the_turn() {
    let blank = LlmResponse {
        content: Some(" \n\t ".into()),
        ..empty_reply(Some(StopReason::EndTurn))
    };
    let (mut agent, provider) = streaming(vec![done(background_call()), done(blank)]);
    let mut messages = vec![Message::user("start the job")];
    let result = agent.process(&mut messages).await.expect("the turn ends");
    assert_ended_on_the_tool_result(&messages, &result);
    assert_eq!(provider.request_count(), 2);
}

/// Review round 2 (M1): a real stream sends the whitespace as a text delta
/// before it ends. Whitespace shown is no output, so the reply is still
/// asked again rather than failing as output already shown.
#[tokio::test]
async fn a_whitespace_first_reply_to_a_prompt_is_retried() {
    let blank = LlmResponse {
        content: Some("\n\n".into()),
        ..empty_reply(Some(StopReason::EndTurn))
    };
    let (mut agent, provider) = streaming(vec![
        vec![
            StreamEvent::TextDelta("\n\n".into()),
            StreamEvent::Done(blank),
        ],
        done(text_response("the answer")),
    ]);
    let mut messages = vec![Message::user("question")];
    let result = agent
        .process(&mut messages)
        .await
        .expect("the retry answers");
    assert_eq!(result.response, "the answer");
    assert_eq!(provider.request_count(), 2);
}

/// Text that is not whitespace is output shown: never sent again (#2155).
#[tokio::test]
async fn a_failure_after_visible_text_is_still_not_retried() {
    let (mut agent, provider) = streaming(vec![
        vec![
            StreamEvent::TextDelta("\n".into()),
            StreamEvent::TextDelta("Hal".into()),
            StreamEvent::Error("connection reset by peer".into()),
        ],
        done(text_response("unused")),
    ]);
    let mut messages = vec![Message::user("question")];
    agent
        .process(&mut messages)
        .await
        .expect_err("output was shown");
    assert_eq!(provider.request_count(), 1);
}
