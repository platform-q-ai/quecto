//! A resumed session's images reach the model (#2424): a session saved with
//! a user's image and a `read` image is resumed through the dispatch over the
//! real file store, and the provider request of the next prompt carries both.
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use super::super::fixture_tests::Fixture;
use super::super::{dispatch_command, handle_resume_session};
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::application::providers::ports::{ChatRequest, LlmProvider};
use crate::application::sessions::ports::SessionStore;
use crate::domain::conversation::value_objects::message::{
    LlmResponse, Message, Role, ToolCall, UserImageBlock,
};
use crate::domain::error::DomainError;
use crate::domain::sessions::entities::session::Session;
use crate::domain::sessions::entities::session_identity::SessionIdentity;
use crate::domain::tool_policy::value_objects::tool::ImageBlock;
use crate::interface::cli::protocol::AgentCommand;

use quecto_image::{ImageMime, samples};

fn png() -> String {
    samples::encode(&samples::png(2, 3))
}

fn jpeg() -> String {
    samples::encode(&samples::jpeg(2, 2))
}

/// A provider that keeps every request's messages.
#[derive(Debug, Default)]
struct RecordingProvider {
    requests: Mutex<Vec<Vec<Message>>>,
}

impl LlmProvider for RecordingProvider {
    fn name(&self) -> &str {
        "recording"
    }

    fn route_order(&self) -> Vec<String> {
        vec![self.name().to_string()]
    }

    fn chat(
        &self,
        request: ChatRequest<'_>,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<LlmResponse, DomainError>> + Send + '_>>
    {
        self.requests
            .lock()
            .unwrap()
            .push(request.messages.to_vec());
        Box::pin(async {
            Ok(LlmResponse {
                content: Some("it is the same picture".into()),
                tool_calls: vec![],
                usage: None,
                stop_reason: None,
                thinking_blocks: vec![],
            })
        })
    }
}

fn agent_over(provider: Arc<RecordingProvider>) -> AgentLoopImpl {
    AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(crate::infrastructure::tools::registry::ToolRegistryImpl::new()),
        model: "stub".into(),
        max_tokens: 100,
        temperature: 0.0,
        retention: None,
        session_key: "cli:test".into(),
        max_context_tokens: 190_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_marks: Default::default(),
        model_context_window: None,
        tool_profile_context:
            crate::domain::tool_policy::value_objects::tool::ToolProfileContext::Parent,
    })
    // A model whose catalogue entry takes images (#2421).
    .with_model_limits(crate::application::catalogue::dto::ModelLimits {
        image_input: crate::domain::conversation::services::image_input::ImageInput::AllImages,
        ..Default::default()
    })
}

/// A user's photo, then `read` of a screenshot, as a session holds them.
fn conversation_with_images() -> Vec<Message> {
    let mut photo = Message::user("what is this?");
    photo.user_image_blocks = vec![UserImageBlock::restore(ImageMime::Jpeg, jpeg()).unwrap()];
    let call = ToolCall {
        id: "call-1".into(),
        name: "read".into(),
        arguments: r#"{"path":"shot.png"}"#.into(),
    };
    let mut screenshot = Message::tool("call-1", "Read image file [image/png] (30 B)");
    screenshot.tool_name = Some("read".into());
    screenshot.image_blocks = vec![ImageBlock::unchecked_for_tests(
        quecto_image::ImageMime::Png,
        png(),
    )];
    vec![
        photo,
        Message::assistant("a photo; let me read the screenshot", vec![call]),
        screenshot,
        Message::assistant("the screenshot is a red square", vec![]),
    ]
}

#[tokio::test]
async fn a_resumed_session_sends_its_images_in_the_next_provider_request() {
    let mut fx = Fixture::new();
    let provider = Arc::new(RecordingProvider::default());
    fx.set_agent(agent_over(provider.clone()));
    let key = Session::build_key("cli", "pictures");
    crate::interface::cli::uds::dispatch_session_roster_tests::seed_home(&fx.store, &key).await;
    fx.store
        .save(&Session {
            key: SessionIdentity::from_persisted_key(key.clone()),
            messages: conversation_with_images(),
            workflow_run: None,
            subagent_roster: Vec::new(),
        })
        .await
        .unwrap();

    let mut ctx = fx.ctx();
    assert!(
        !handle_resume_session(&mut ctx, Some("rs"), "resume_session", "pictures".into()).await
    );
    assert_eq!(ctx.sessions.current_session_key().await, key);
    let prompt = AgentCommand::Prompt {
        id: Some("p".into()),
        message: "is it the same picture?".into(),
        images: Vec::new(),
        streaming_behavior: None,
    };
    assert!(!dispatch_command(prompt, &mut ctx).await);

    let requests = provider.requests.lock().unwrap();
    let request = requests.last().expect("the prompt reached the provider");
    let photo = request
        .iter()
        .find(|m| m.role == Role::User && m.content == "what is this?")
        .expect("the resumed photo message is sent");
    assert_eq!(photo.user_image_blocks.len(), 1, "with its image");
    assert_eq!(photo.user_image_blocks[0].mime_type(), "image/jpeg");
    assert_eq!(photo.user_image_blocks[0].data(), jpeg());
    let screenshot = request
        .iter()
        .find(|m| m.role == Role::Tool)
        .expect("the resumed tool result is sent");
    assert_eq!(screenshot.image_blocks.len(), 1, "with its image");
    assert_eq!(screenshot.image_blocks[0].mime_type(), "image/png");
    assert_eq!(screenshot.image_blocks[0].data(), png());
}

#[tokio::test]
async fn an_image_a_resume_cannot_read_is_a_marker_in_the_request_and_never_in_the_file() {
    let mut fx = Fixture::new();
    let provider = Arc::new(RecordingProvider::default());
    fx.set_agent(agent_over(provider.clone()));
    let key = Session::build_key("cli", "lost");
    let identity = SessionIdentity::from_persisted_key(key.clone());
    crate::interface::cli::uds::dispatch_session_roster_tests::seed_home(&fx.store, &key).await;
    fx.store
        .save(&Session {
            key: identity.clone(),
            messages: conversation_with_images(),
            workflow_run: None,
            subagent_roster: Vec::new(),
        })
        .await
        .unwrap();
    let layout =
        crate::infrastructure::persistence::session_layout::FlatSessionLayout::new(fx._tmp.path());
    let png = png();
    let sha256 =
        crate::domain::conversation::value_objects::stored_images::sha256_hex(png.as_bytes());
    std::fs::remove_file(layout.image_dir(&identity).join(&sha256)).unwrap();

    let mut ctx = fx.ctx();
    assert!(!handle_resume_session(&mut ctx, Some("rs"), "resume_session", "lost".into()).await);
    let prompt = AgentCommand::Prompt {
        id: Some("p".into()),
        message: "and the screenshot?".into(),
        images: Vec::new(),
        streaming_behavior: None,
    };
    assert!(!dispatch_command(prompt, &mut ctx).await);

    let marker =
        crate::domain::conversation::value_objects::stored_images::unavailable_marker(&sha256);
    {
        let requests = provider.requests.lock().unwrap();
        let request = requests.last().expect("the prompt reached the provider");
        let screenshot = request.iter().find(|m| m.role == Role::Tool).unwrap();
        assert!(screenshot.image_blocks.is_empty());
        assert_eq!(
            screenshot.content,
            format!("Read image file [image/png] (30 B)\n{marker}")
        );
        let photo = request
            .iter()
            .find(|m| m.content == "what is this?")
            .unwrap();
        assert_eq!(
            photo.user_image_blocks[0].data(),
            jpeg(),
            "the photo is read"
        );
    }
    let saved = std::fs::read_to_string(layout.session_file(&identity)).unwrap();
    assert!(saved.contains(&sha256), "the reference is kept: {saved}");
    assert!(!saved.contains("image unavailable"), "{saved}");
}
