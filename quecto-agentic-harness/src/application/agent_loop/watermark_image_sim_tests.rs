//! #2420: screenshots are priced by their pixel size. A session of about
//! 70k estimated text tokens takes 20 one-megabyte 1920x1080 screenshots
//! through the real agent loop and context code, at the owner's marks:
//! no request is cut and the emergency ladder never runs, so every request
//! extends the previous one and the last still holds every screenshot.
//! (At ASCII/4 each screenshot estimated ~350k tokens, over the 256k high
//! mark on its own.)

use super::ctx_mgmt_tests::MemSpillStore;
use crate::application::agent_loop::tests::MockRegistry;
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::application::providers::ports::{ChatRequest, LlmProvider};
use crate::application::tools::ports::Tool;
use crate::domain::conversation::UserKind;
use crate::domain::conversation::image_tokens::estimate_named_image_tokens;
use crate::domain::conversation::watermark::{DEFAULT_HIGH_TOKENS, Watermark};
use crate::domain::error::DomainError;
use crate::domain::message::{LlmResponse, Message, ToolCall};
use crate::domain::tool::{ImageBlock, ToolDefinition, ToolResult};
use crate::domain::turn_origin::prompt;
use quecto_image::samples::{encode, png_with_body};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

const SCREENSHOTS: usize = 20;
/// Per turn: the prompt, the screenshot's text, the answer (estimated).
const PROMPT_TOKENS: usize = 200;
const CAPTION_TOKENS: usize = 1_500;
const ANSWER_TOKENS: usize = 1_700;

const WORDS: &[&str] = &[
    "the", "window", "button", "dialog", "menu", "shows", "label", "field", "page", "browser",
    "click", "scroll", "text", "image", "toolbar", "panel",
];

/// About `tokens` estimated tokens of prose, the same for the same seed.
fn prose(seed: usize, tokens: usize) -> String {
    let mut text = String::new();
    let mut n = seed;
    while Message::estimate_tokens(&text) < tokens {
        n = n.wrapping_mul(31).wrapping_add(7);
        text.push_str(WORDS[n % WORDS.len()]);
        text.push(' ');
    }
    text
}

/// One request as the provider saw it: each message's id, content length
/// and image count (a cut or the ladder changes one of them), and whether
/// a cut's stub is in it.
type Shape = Vec<(uuid::Uuid, usize, usize)>;

#[derive(Debug, Default)]
struct ScreenshotProvider {
    /// The turn, and whether its screenshot was taken.
    cursor: Mutex<(usize, bool)>,
    requests: Mutex<Vec<(Shape, bool)>>,
}

impl LlmProvider for ScreenshotProvider {
    fn route_order(&self) -> Vec<String> {
        vec![self.name().to_string()]
    }
    fn name(&self) -> &str {
        "screenshot-sim"
    }

    fn chat(
        &self,
        request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        let shape = request
            .messages
            .iter()
            .map(|m| (m.id(), m.content.len(), m.image_blocks.len()))
            .collect();
        let stubbed = request
            .messages
            .iter()
            .any(|m| m.user_kind == UserKind::ArchiveStub);
        self.requests.lock().unwrap().push((shape, stubbed));
        let mut cursor = self.cursor.lock().unwrap();
        let (turn, taken) = *cursor;
        let (content, tool_calls) = match taken {
            false => {
                let call = ToolCall {
                    id: format!("shot-{turn}"),
                    name: "screenshot".to_string(),
                    arguments: format!(r#"{{"turn":{turn}}}"#),
                };
                (None, vec![call])
            }
            true => (Some(prose(turn * 3 + 2, ANSWER_TOKENS)), vec![]),
        };
        *cursor = match taken {
            false => (turn, true),
            true => (turn + 1, false),
        };
        let response = LlmResponse {
            content,
            tool_calls,
            usage: None,
            stop_reason: None,
            thinking_blocks: vec![],
        };
        Box::pin(async move { Ok(response) })
    }
}

/// `screenshot`: a 1 MB 1920x1080 PNG and a caption.
struct Screenshot(Arc<String>);

impl Tool for Screenshot {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "screenshot".into(),
            description: "Take a screenshot.".into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let content = prose(arguments.len() * 3 + 1, CAPTION_TOKENS);
        let data = self.0.as_str().to_owned();
        Box::pin(async move {
            Ok(ToolResult {
                content,
                is_error: false,
                image_blocks: vec![ImageBlock::new("image/png", data)],
                delivery_metadata: None,
            })
        })
    }
}

fn screenshot_agent(provider: Arc<ScreenshotProvider>, screenshot: Arc<String>) -> AgentLoopImpl {
    let mut registry = MockRegistry::new();
    registry.register(Arc::new(Screenshot(screenshot)));
    AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(registry),
        model: "gpt-sim".to_string(),
        max_tokens: 1024,
        temperature: 0.7,
        retention: Some(crate::composition::retention::context_retention_over(
            Arc::new(MemSpillStore::default()),
        )),
        session_key: "screenshot-sim".to_string(),
        max_context_tokens: 272_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 1,
        context_marks: Watermark::default(),
        model_context_window: None,
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
    })
}

#[tokio::test]
async fn twenty_screenshots_in_a_70k_conversation_trigger_no_cut() {
    let screenshot = Arc::new(encode(&png_with_body(1920, 1080, 1 << 20)));
    assert!(screenshot.len() > 1_300_000, "a 1 MB screenshot");
    let provider = Arc::new(ScreenshotProvider::default());
    let mut agent = screenshot_agent(provider.clone(), screenshot);
    let mut messages = vec![Message::system(prose(1, 2_000))];
    for turn in 0..SCREENSHOTS {
        messages.push(prompt(prose(turn * 3, PROMPT_TOKENS)));
        agent.run_loop(&mut messages).await.expect("the turn runs");
    }

    let requests = provider.requests.lock().unwrap();
    assert_eq!(
        requests.len(),
        SCREENSHOTS * 2,
        "a call and an answer a turn"
    );
    assert!(!requests.iter().any(|(_, stubbed)| *stubbed), "no cut");
    for (n, pair) in requests.windows(2).enumerate() {
        let (previous, request) = (&pair[0].0, &pair[1].0);
        assert!(
            request.starts_with(previous),
            "request {}: extends the previous one (no cut, no ladder)",
            n + 1
        );
    }

    let text: usize = messages
        .iter()
        .map(|m| Message::estimate_tokens(&m.content))
        .sum();
    assert!(
        (65_000..=75_000).contains(&text),
        "about 70k of text: {text}"
    );
    let images = messages.iter().map(|m| m.image_blocks.len()).sum::<usize>();
    assert_eq!(images, SCREENSHOTS, "every screenshot is still held");
    for image in messages.iter().flat_map(|m| &m.image_blocks) {
        let estimate = estimate_named_image_tokens(image.mime_type, image.data());
        assert_eq!(estimate, 2765, "1920x1080 by its pixels, not the fallback");
    }
    let total: usize = messages.iter().map(Message::estimated_tokens).sum();
    assert!(total < DEFAULT_HIGH_TOKENS, "under the high mark: {total}");
}
