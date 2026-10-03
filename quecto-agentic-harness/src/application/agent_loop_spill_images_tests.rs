//! Images in session memory (#2424): a message spilled with images keeps
//! their references, and a recall of it brings back its text and says how
//! many images it did not bring back (recall stays text). Over the real
//! spill file and the composed recall tool.
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use base64::Engine;

use super::super::tests::{MockProvider, MockRegistry, text_response, tool_call_response};
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::application::sessions::ports::ContextSpillStore;
use crate::application::tools::ports::Tool;
use crate::domain::error::DomainError;
use crate::domain::message::{Message, Role, UserImageBlock};
use crate::domain::session_identity::SessionIdentity;
use crate::domain::tool::{ImageBlock, ToolDefinition, ToolResult};
use crate::infrastructure::persistence::context_spill::FileContextSpillStore;
use crate::infrastructure::persistence::session_layout::FlatSessionLayout;
use crate::infrastructure::tools::recall::RecallTool;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x02\0\0\0\x03\x08\x02\0\0\0";
const KEY: &str = "cli:recall-images";

fn base64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// `read` of an image: a line of text and the image.
#[derive(Debug)]
struct ReadImage;

impl Tool for ReadImage {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "read".into(),
            description: "read".into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
        }
    }

    fn execute(
        &self,
        _arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        Box::pin(async {
            Ok(ToolResult {
                content: "Read image file [image/png] (30 B)".into(),
                is_error: false,
                image_blocks: vec![ImageBlock::new("image/png", base64(PNG))],
                delivery_metadata: None,
            })
        })
    }
}

async fn recall(spill: Arc<dyn ContextSpillStore>, id: &str) -> String {
    let recall = crate::composition::retention::retention_handles_over(spill).recall;
    let tool = RecallTool::new(recall, KEY.to_string());
    let arguments = serde_json::json!({ "id": id }).to_string();
    tool.execute(&arguments).await.unwrap().content
}

#[tokio::test]
async fn a_recalled_message_with_images_says_how_many_it_did_not_bring_back() {
    let tmp = tempfile::TempDir::new().unwrap();
    let spill: Arc<dyn ContextSpillStore> = Arc::new(FileContextSpillStore::new(
        FlatSessionLayout::new(tmp.path()),
    ));
    let mut registry = MockRegistry::new();
    registry.register(Arc::new(ReadImage));
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: Arc::new(MockProvider::new(vec![
            tool_call_response("read", r#"{"path":"shot.png"}"#),
            text_response("a red square"),
        ])),
        tool_registry: Box::new(registry),
        model: "test-model".to_string(),
        max_tokens: 1024,
        temperature: 0.7,
        retention: Some(crate::composition::retention::context_retention_over(
            spill.clone(),
        )),
        session_key: KEY.to_string(),
        max_context_tokens: 190_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_marks: Default::default(),
        model_context_window: None,
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
    });
    // A prompt that is only an image: no text, still retained.
    let mut prompt = Message::user("");
    prompt.user_image_blocks = vec![UserImageBlock::sample(quecto_image::ImageMime::Png)];
    let mut messages = vec![prompt];
    agent.run_loop(&mut messages).await.unwrap();

    let tool_message = messages.iter().find(|m| m.role == Role::Tool).unwrap();
    let tool_id = tool_message
        .spill_id
        .clone()
        .expect("the tool result was retained");
    assert_eq!(
        recall(spill.clone(), &tool_id).await,
        "Read image file [image/png] (30 B)\n[1 image(s) not recalled]"
    );
    let prompt = messages
        .iter()
        .find(|m| m.role == Role::User && !m.user_image_blocks.is_empty())
        .expect("the prompt is kept");
    let prompt_id = prompt
        .spill_id
        .clone()
        .expect("an image-only prompt is retained");
    assert_eq!(
        recall(spill.clone(), &prompt_id).await,
        "[1 image(s) not recalled]"
    );
    // Listed with what it costs and a preview that says what it is.
    let index = spill.list_entries(&SessionIdentity::from_persisted_key(KEY));
    let index = index.await.unwrap();
    let listed = index.iter().find(|e| e.id == prompt_id).unwrap();
    assert_eq!(listed.input_preview, "[image]");
    assert!(listed.tokens > 0, "an image-only prompt costs its image");
}
