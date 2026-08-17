use std::sync::Arc;

use super::tests::{MockRegistry, MockStreamingProvider, test_config};
use super::{AgentLoopConfig, AgentLoopImpl};
use crate::domain::message::{LlmResponse, Message, ThinkingBlock};
use crate::domain::provider::StreamEvent;

#[tokio::test]
async fn text_only_final_response_persists_thinking_blocks() {
    let response = LlmResponse {
        content: Some("answer".into()),
        tool_calls: Vec::new(),
        usage: None,
        stop_reason: None,
        thinking_blocks: vec![ThinkingBlock::Normal {
            thinking: "reasoning".into(),
            signature: "PRIVATE_SIGNATURE".into(),
        }],
    };
    let provider = Arc::new(MockStreamingProvider::new(vec![vec![StreamEvent::Done(
        response,
    )]]));
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        streaming: true,
        ..test_config(provider, Box::new(MockRegistry::default()))
    });
    let mut messages = vec![Message::user("hi")];

    agent.run_loop(&mut messages).await.unwrap();
    let assistant = messages.last().expect("assistant response");
    assert_eq!(assistant.content, "answer");
    assert_eq!(assistant.thinking_blocks.len(), 1);
}
