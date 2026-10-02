//! #2421: the request the provider receives carries images only when the
//! active model takes them; otherwise each image is a marker in its message.
//! The conversation itself keeps its images either way.

use super::*;
use crate::application::catalogue::dto::ModelLimits;
use crate::application::catalogue::ports::ModelRuntime;
use crate::domain::conversation::image_input::not_sent_marker;
use crate::domain::message::{Message, UserImageBlock};

/// What one request carried: each message's text and image count.
type Sent = Vec<(String, usize)>;

#[derive(Debug, Default)]
struct RecordingProvider {
    requests: Mutex<Vec<Sent>>,
}

impl LlmProvider for RecordingProvider {
    fn name(&self) -> &str {
        "recording"
    }

    fn chat(
        &self,
        request: ChatRequest<'_>,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<LlmResponse, DomainError>> + Send + '_>>
    {
        let sent = request
            .messages
            .iter()
            .map(|m| {
                let images = m.image_blocks.len() + m.user_image_blocks.len();
                (m.content.clone(), images)
            })
            .collect();
        self.requests.lock().unwrap().push(sent);
        Box::pin(async { Ok(text_response("seen")) })
    }
}

const MODEL: &str = "acme/glm";

fn agent(takes_images: bool) -> (AgentLoopImpl, Arc<RecordingProvider>) {
    let provider = Arc::new(RecordingProvider::default());
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        model: MODEL.to_string(),
        ..test_config(provider.clone(), Box::new(MockRegistry::new()))
    });
    agent.apply_model(
        MODEL.to_string(),
        ModelLimits {
            takes_images,
            ..ModelLimits::default()
        },
    );
    (agent, provider)
}

fn prompt_with_image() -> Message {
    let mut message = Message::user("what is this?");
    message.user_image_blocks = vec![UserImageBlock {
        mime_type: "image/png".into(),
        data: "cG5n".into(),
    }];
    message
}

#[tokio::test]
async fn a_model_that_takes_images_is_sent_them() {
    let (mut agent, provider) = agent(true);
    let mut messages = vec![prompt_with_image()];
    agent.run_loop(&mut messages).await.unwrap();
    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests[0], vec![("what is this?".to_string(), 1)]);
}

#[tokio::test]
async fn a_model_that_takes_no_images_is_sent_a_marker_in_each_ones_place() {
    let (mut agent, provider) = agent(false);
    let mut messages = vec![prompt_with_image()];
    agent.run_loop(&mut messages).await.unwrap();
    let requests = provider.requests.lock().unwrap();
    assert_eq!(
        requests[0],
        vec![(format!("what is this?\n{}", not_sent_marker(MODEL)), 0)]
    );
    assert_eq!(
        messages[0].user_image_blocks.len(),
        1,
        "the conversation keeps the image for a model that takes it"
    );
    assert_eq!(messages[0].content, "what is this?");
}

#[tokio::test]
async fn a_model_with_no_declared_limits_takes_no_images() {
    let provider = Arc::new(RecordingProvider::default());
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        model: MODEL.to_string(),
        ..test_config(provider.clone(), Box::new(MockRegistry::new()))
    });
    let mut messages = vec![prompt_with_image()];
    agent.run_loop(&mut messages).await.unwrap();
    assert_eq!(provider.requests.lock().unwrap()[0][0].1, 0);
}

#[tokio::test]
async fn a_switch_to_a_model_that_takes_images_sends_the_kept_ones() {
    let (mut agent, provider) = agent(false);
    let mut messages = vec![prompt_with_image()];
    agent.run_loop(&mut messages).await.unwrap();
    agent.apply_model(
        "acme/seeing".to_string(),
        ModelLimits {
            takes_images: true,
            ..ModelLimits::default()
        },
    );
    messages.push(Message::user("and now?"));
    agent.run_loop(&mut messages).await.unwrap();
    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests[1][0], ("what is this?".to_string(), 1));
}
