use std::sync::{Arc, Mutex};

use super::tests::{MockRegistry, MockStreamingProvider, test_config, text_response};
use super::{AgentLoopConfig, AgentLoopImpl};
use crate::domain::message::Message;

#[tokio::test]
async fn thinking_delta_emits_distinct_progress_before_answer_token() {
    let provider = Arc::new(MockStreamingProvider::new(vec![vec![
        crate::domain::provider::StreamEvent::ThinkingDelta("thinking aloud".into()),
        crate::domain::provider::StreamEvent::TextDelta("answer".into()),
        crate::domain::provider::StreamEvent::Done(text_response("answer")),
    ]]));
    let events = Arc::new(Mutex::new(Vec::new()));
    let seen = events.clone();
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        streaming: true,
        ..test_config(provider, Box::new(MockRegistry::default()))
    })
    .with_progress_callback(Some(Arc::new(move |event| {
        seen.lock().unwrap().push(event)
    })));
    let mut messages = vec![Message::user("hi")];

    let result = agent.run_loop(&mut messages).await.unwrap();
    assert_eq!(result.response, "answer");
    let events = events.lock().unwrap();
    let thinking_pos = events
        .iter()
        .position(|event| {
            matches!(event, crate::domain::agent::AgentProgressEvent::ThinkingDelta(text) if text == "thinking aloud")
        })
        .expect("thinking delta progress event");
    let token_pos = events
        .iter()
        .position(|event| {
            matches!(event, crate::domain::agent::AgentProgressEvent::Token(text) if text == "answer")
        })
        .expect("answer token progress event");
    assert!(thinking_pos < token_pos);
}
