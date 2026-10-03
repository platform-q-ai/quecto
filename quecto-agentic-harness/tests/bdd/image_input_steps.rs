//! Steps for #2421: a real agent loop, its model switched by the
//! change-active-model use case over catalogue inputs, sends a model that
//! takes no images a marker in each one's place, and keeps the images for a
//! later model that takes them.

use super::active_model_steps::change_active_model_use_case;
use super::*;
use quecto::domain::conversation::image_input::not_sent_marker;
use quecto::domain::message::UserImageBlock;

/// What one request carried: each message's text and its image MIME types.
type Sent = Vec<(String, Vec<String>)>;

#[derive(Debug, Default)]
struct RequestRecorder {
    requests: Mutex<Vec<Sent>>,
}

impl LlmProvider for RequestRecorder {
    fn route_order(&self) -> Vec<String> {
        vec![self.name().to_string()]
    }
    fn name(&self) -> &str {
        "recorder"
    }

    fn chat(
        &self,
        request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        let sent = request
            .messages
            .iter()
            .map(|m| {
                let images = m
                    .user_image_blocks
                    .iter()
                    .map(|image| image.mime_type.clone())
                    .chain(
                        m.image_blocks
                            .iter()
                            .map(|image| image.mime_type.to_string()),
                    )
                    .collect();
                (m.content.clone(), images)
            })
            .collect();
        self.requests.lock().unwrap().push(sent);
        Box::pin(async {
            Ok(LlmResponse {
                content: Some("seen".into()),
                tool_calls: vec![],
                usage: None,
                stop_reason: None,
                thinking_blocks: vec![],
            })
        })
    }
}

#[derive(Default)]
pub struct ImageInputState {
    agent: Option<AgentLoopImpl>,
    recorder: Option<Arc<RequestRecorder>>,
    conversation: Vec<Message>,
}

impl std::fmt::Debug for ImageInputState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageInputState")
            .field("conversation", &self.conversation.len())
            .finish_non_exhaustive()
    }
}

fn switch_model(world: &mut QuectoWorld, model: &str) {
    let use_case = change_active_model_use_case(&mut world.active_model);
    let agent = world.image_input.agent.as_mut().expect("an agent runs");
    use_case
        .execute(agent, model)
        .expect("a catalogue model switch applies");
}

#[given(expr = "an agent recording its requests runs on {string}")]
fn given_recording_agent(world: &mut QuectoWorld, model: String) {
    let recorder = Arc::new(RequestRecorder::default());
    let agent = AgentLoopImpl::new(quecto::application::agent_loop::AgentLoopConfig {
        provider: recorder.clone(),
        tool_registry: Box::new(ToolRegistryImpl::new()),
        model: model.clone(),
        max_tokens: 1024,
        temperature: 0.7,
        retention: None,
        session_key: String::new(),
        max_context_tokens: 190_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_marks: Default::default(),
        model_context_window: None,
        tool_profile_context: quecto::domain::tool::ToolProfileContext::Parent,
    });
    world.image_input.agent = Some(agent);
    world.image_input.recorder = Some(recorder);
    // The startup model's limits come from the same use case (#1847).
    switch_model(world, &model);
}

async fn prompt(world: &mut QuectoWorld, message: Message) {
    let state = &mut world.image_input;
    state.conversation.push(message);
    state
        .agent
        .as_mut()
        .expect("an agent runs")
        .process(&mut state.conversation)
        .await
        .expect("the turn completes");
}

#[when(expr = "the agent is prompted {string} with a {string} image")]
async fn when_prompted_with_image(world: &mut QuectoWorld, text: String, mime: String) {
    let mut message = Message::user(text);
    message.user_image_blocks = vec![UserImageBlock {
        mime_type: mime,
        data: "cG5n".into(),
    }];
    prompt(world, message).await;
}

#[when(expr = "the agent is prompted {string}")]
async fn when_prompted(world: &mut QuectoWorld, text: String) {
    prompt(world, Message::user(text)).await;
}

#[when(expr = "the agent's active model is changed to {string}")]
fn when_agent_model_changed(world: &mut QuectoWorld, model: String) {
    switch_model(world, &model);
}

fn first_message_of_last_request(world: &QuectoWorld) -> (String, Vec<String>) {
    let recorder = world.image_input.recorder.as_ref().expect("a recorder");
    let requests = recorder.requests.lock().unwrap();
    requests.last().expect("a request was sent")[0].clone()
}

#[then(expr = "the last request sent {string} with the marker for {string} and no image")]
fn then_sent_marker(world: &mut QuectoWorld, text: String, model: String) {
    let (content, images) = first_message_of_last_request(world);
    assert_eq!(content, format!("{text}\n{}", not_sent_marker(&model)));
    assert!(images.is_empty(), "no image is sent: {images:?}");
    let kept = &world.image_input.conversation[0].user_image_blocks;
    assert_eq!(kept.len(), 1, "the conversation keeps the image");
}

#[then(expr = "the last request sent {string} with its {string} image")]
fn then_sent_image(world: &mut QuectoWorld, text: String, mime: String) {
    let (content, images) = first_message_of_last_request(world);
    assert_eq!(content, text);
    assert_eq!(images, vec![mime]);
}
