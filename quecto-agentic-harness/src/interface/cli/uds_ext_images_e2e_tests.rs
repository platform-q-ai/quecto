//! #2423 end to end: an extension tool's image travels from its UDS
//! `tool_result` through the agent loop into the provider request's tool
//! result, for a model that takes images.
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use quecto_image::samples::{encode, png};
use serde_json::{Value, json};

use super::*;
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::application::agent_turn::ports::AgentLoop;
use crate::application::catalogue::dto::ModelLimits;
use crate::application::catalogue::ports::ModelRuntime;
use crate::application::providers::ports::{ChatRequest, LlmProvider};
use crate::domain::conversation::services::image_input::ImageInput;
use crate::domain::conversation::value_objects::message::{LlmResponse, Message, ToolCall};
use crate::domain::error::DomainError;

/// Each request's tool-result images, as (MIME type, base64).
type ToolImages = Vec<(String, String)>;

#[derive(Debug, Default)]
struct SeeingProvider {
    requests: Mutex<Vec<ToolImages>>,
}

impl LlmProvider for SeeingProvider {
    fn route_order(&self) -> Vec<String> {
        vec![self.name().to_string()]
    }
    fn name(&self) -> &str {
        "seeing"
    }

    fn chat(
        &self,
        request: ChatRequest<'_>,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<LlmResponse, DomainError>> + Send + '_>>
    {
        let images: ToolImages = request
            .messages
            .iter()
            .filter(|m| m.tool_call_id.is_some())
            .flat_map(|m| m.image_blocks.iter())
            .map(|image| (image.mime_type().to_owned(), image.data().to_owned()))
            .collect();
        let mut requests = self.requests.lock().unwrap();
        let first = requests.is_empty();
        requests.push(images);
        let reply = LlmResponse {
            content: (!first).then(|| "a small image".to_string()),
            tool_calls: match first {
                true => vec![ToolCall {
                    id: "call_shot".into(),
                    name: "shot".into(),
                    arguments: "{}".into(),
                }],
                false => vec![],
            },
            usage: None,
            stop_reason: None,
            thinking_blocks: vec![],
        };
        Box::pin(async move { Ok(reply) })
    }
}

fn seeing_agent(
    provider: Arc<SeeingProvider>,
    registry: crate::infrastructure::tools::registry::ToolRegistryImpl,
) -> AgentLoopImpl {
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(registry),
        model: "acme/seeing".into(),
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
    });
    agent.apply_model(
        "acme/seeing".into(),
        ModelLimits {
            image_input: ImageInput::AllImages,
            ..ModelLimits::default()
        },
    );
    agent
}

/// The extension side: answer the one `execute_tool` with `image_blocks`.
async fn answer_once(
    client_registry: ClientToolRegistry,
    mut requests: tokio::sync::mpsc::Receiver<
        crate::application::extensions::ports::PendingToolInvocation,
    >,
    image_blocks: Value,
) {
    let (writer_tx, mut writer_rx) = tokio::sync::mpsc::channel::<String>(1);
    register_client_writer(&client_registry, 1, writer_tx.clone());
    let request = requests.recv().await.expect("the model calls the tool");
    handle_one_request("shot", request, 1, &client_registry, &Some(writer_tx)).await;
    let line = writer_rx
        .recv()
        .await
        .expect("execute_tool reaches the client");
    let event: Value = serde_json::from_str(line.trim()).unwrap();
    let call_id = event["toolCallId"].as_str().unwrap();
    handle_tool_result(ToolResultArgs {
        client_id: 1,
        tool_call_id: call_id,
        content: "took a screenshot",
        is_error: false,
        image_blocks: serde_json::from_value(image_blocks).ok(),
        registry: &client_registry,
    });
}

#[tokio::test]
async fn an_extension_tools_image_reaches_the_provider_request() {
    let client_registry = new_client_tool_registry();
    let registration = ToolRegistration {
        name: "shot".into(),
        description: "Take a screenshot".into(),
        parameters_schema: r#"{"type":"object"}"#.into(),
        stable_id: None,
        timeout_seconds: None,
    };
    let (ok, _, tools) = handle_register_tools(RegisterToolsArgs {
        client_id: 1,
        id: None,
        tools: std::slice::from_ref(&registration),
        registry: &client_registry,
        core_tool_names: &HashSet::new(),
    });
    assert!(ok);
    let requests = client_registry
        .lock()
        .unwrap()
        .get_mut(&1)
        .unwrap()
        .tool_request_rxs
        .remove("shot")
        .unwrap();
    let mut registry = crate::infrastructure::tools::registry::ToolRegistryImpl::new();
    registry.register(tools[0].clone());

    let data = encode(&png(4, 4));
    let blocks = json!([{"mimeType": "image/png", "data": data}]);
    let extension = tokio::spawn(answer_once(client_registry.clone(), requests, blocks));
    let provider = Arc::new(SeeingProvider::default());
    let mut agent = seeing_agent(provider.clone(), registry);
    let mut messages = vec![Message::user("what is on the screen?")];
    agent.process(&mut messages).await.unwrap();
    extension.await.unwrap();

    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 2, "the tool call, then its result");
    assert_eq!(requests[1], vec![("image/png".to_owned(), data)]);
}
