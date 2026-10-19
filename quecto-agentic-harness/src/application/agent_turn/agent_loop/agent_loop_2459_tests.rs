//! #2459 and #2531 through the agent turn: a terminal failure's guidance
//! says what the request did (its attempts, whether output streamed), and
//! a Codex 507 for an exhausted upstream retry buffer is retried before
//! any output like any server failure.
use crate::application::agent_turn::agent_loop::tests::{
    MockRegistry, MockStreamingProvider, test_config, text_response,
};
use crate::application::agent_turn::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::domain::conversation::value_objects::message::Message;
use crate::domain::inference::value_objects::provider::StreamEvent;
use std::sync::Arc;

const CUT_SHORT: &str =
    "OpenAI SSE stream ended without completion: connection closed before [DONE]";

fn streaming(responses: Vec<Vec<StreamEvent>>) -> (AgentLoopImpl, Arc<MockStreamingProvider>) {
    let provider = Arc::new(MockStreamingProvider::new(responses));
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        streaming: true,
        ..test_config(provider.clone(), Box::new(MockRegistry::default()))
    });
    (agent, provider)
}

#[tokio::test]
async fn a_stream_cut_off_after_output_says_it_was_not_retried() {
    let (mut agent, provider) = streaming(vec![vec![
        StreamEvent::TextDelta("half an answ".into()),
        StreamEvent::Error(CUT_SHORT.into()),
    ]]);
    let mut messages = vec![Message::user("hello")];
    let err = agent
        .run_loop(&mut messages)
        .await
        .expect_err("a reply cut off after output ends the turn");
    let text = err.to_string();
    assert_eq!(provider.request_count(), 1, "never replayed");
    assert!(text.contains("It was not retried"), "{text}");
    assert!(
        text.contains("continuing or re-prompting is safe"),
        "{text}"
    );
    assert!(!text.contains("It was retried"), "{text}");
    assert!(!text.contains("could not reach"), "{text}");
}

#[tokio::test(start_paused = true)]
async fn a_failure_before_output_after_retries_states_the_attempt_count() {
    let (mut agent, provider) = streaming(vec![
        vec![StreamEvent::Error(CUT_SHORT.into())],
        vec![StreamEvent::Error(CUT_SHORT.into())],
        vec![StreamEvent::Error(CUT_SHORT.into())],
    ]);
    let mut messages = vec![Message::user("hello")];
    let err = agent.run_loop(&mut messages).await.expect_err("exhausted");
    let text = err.to_string();
    assert_eq!(provider.request_count(), 3);
    assert!(text.contains("It was retried (3 attempts)"), "{text}");
    assert!(
        !text.contains("could not reach"),
        "no attempt record says the provider was unreachable: {text}"
    );
}

#[tokio::test(start_paused = true)]
async fn a_codex_507_retry_buffer_limit_before_output_is_retried() {
    let (mut agent, provider) = streaming(vec![
        vec![StreamEvent::Error(
            "HTTP 507 from Codex: exceeded request buffer limit while retrying upstream".into(),
        )],
        vec![StreamEvent::Done(text_response("recovered"))],
    ]);
    let mut messages = vec![Message::user("hello")];
    let result = agent
        .run_loop(&mut messages)
        .await
        .expect("a 507 retry-buffer limit is transient");
    assert_eq!(result.response, "recovered");
    assert_eq!(provider.request_count(), 2);
}
