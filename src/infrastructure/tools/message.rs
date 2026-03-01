// Message tool: sends a message to the user on their channel.

use std::future::Future;
use std::pin::Pin;

use tokio::sync::mpsc;

use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
use crate::infrastructure::bus::OutboundMessage;

/// Tool that lets the agent send a message to the user via their channel.
#[derive(Debug)]
pub struct MessageTool {
    outbound_tx: mpsc::Sender<OutboundMessage>,
    /// Default target (e.g. "telegram:12345") for the current conversation.
    default_target: Option<String>,
}

impl MessageTool {
    pub fn new(outbound_tx: mpsc::Sender<OutboundMessage>, default_target: Option<String>) -> Self {
        Self {
            outbound_tx,
            default_target,
        }
    }
}

impl Tool for MessageTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "message".to_string(),
            description: "Send a message to the user on their channel".to_string(),
            parameters_schema: r#"{"type":"object","properties":{"text":{"type":"string","description":"The message text to send"},"target":{"type":"string","description":"Optional target in 'channel:chat_id' format (defaults to current conversation)"}},"required":["text"]}"#.to_string(),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let args = arguments.to_string();
        Box::pin(async move {
            let parsed: serde_json::Value = serde_json::from_str(&args)
                .map_err(|e| DomainError::Tool(format!("invalid JSON: {}", e)))?;

            let text = parsed
                .get("text")
                .and_then(|v| v.as_str())
                .ok_or_else(|| DomainError::Tool("missing required field: text".to_string()))?
                .to_string();

            let target = parsed
                .get("target")
                .and_then(|v| v.as_str())
                .map(String::from)
                .or_else(|| self.default_target.clone())
                .ok_or_else(|| {
                    DomainError::Tool("no target specified and no default target set".to_string())
                })?;

            self.outbound_tx
                .send(OutboundMessage {
                    target: target.clone(),
                    text: text.clone(),
                })
                .await
                .map_err(|e| DomainError::Tool(format!("failed to send message: {}", e)))?;

            Ok(ToolResult {
                content: format!("Message sent to {}.", target),
                is_error: false,
                image_blocks: vec![],
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::bus::MessageBus;

    #[tokio::test]
    async fn test_send_message_with_default_target() {
        let mut bus = MessageBus::new(16);
        let sender = bus.outbound_sender();
        let mut receiver = bus.take_outbound_receiver().unwrap();

        let tool = MessageTool::new(sender, Some("telegram:12345".to_string()));
        let result = tool.execute(r#"{"text":"Hello user!"}"#).await.unwrap();

        assert!(!result.is_error);
        assert!(result.content.contains("telegram:12345"));

        let msg = receiver.recv().await.unwrap();
        assert_eq!(msg.target, "telegram:12345");
        assert_eq!(msg.text, "Hello user!");
    }

    #[tokio::test]
    async fn test_send_message_with_explicit_target() {
        let mut bus = MessageBus::new(16);
        let sender = bus.outbound_sender();
        let mut receiver = bus.take_outbound_receiver().unwrap();

        let tool = MessageTool::new(sender, None);
        let result = tool
            .execute(r#"{"text":"Hi there","target":"telegram:99999"}"#)
            .await
            .unwrap();

        assert!(!result.is_error);
        let msg = receiver.recv().await.unwrap();
        assert_eq!(msg.target, "telegram:99999");
        assert_eq!(msg.text, "Hi there");
    }

    #[tokio::test]
    async fn test_send_message_no_target() {
        let mut bus = MessageBus::new(16);
        let sender = bus.outbound_sender();
        let _receiver = bus.take_outbound_receiver().unwrap();

        let tool = MessageTool::new(sender, None);
        let result = tool.execute(r#"{"text":"Hello"}"#).await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no target"));
    }

    #[tokio::test]
    async fn test_missing_text_field() {
        let mut bus = MessageBus::new(16);
        let sender = bus.outbound_sender();
        let _receiver = bus.take_outbound_receiver().unwrap();

        let tool = MessageTool::new(sender, Some("telegram:123".to_string()));
        let result = tool.execute(r#"{"wrong":"field"}"#).await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("text"));
    }

    #[test]
    fn test_definition() {
        let (tx, _rx) = mpsc::channel(1);
        let tool = MessageTool::new(tx, None);
        let def = tool.definition();
        assert_eq!(def.name, "message");
        assert!(def.description.contains("message"));
    }
}
