// UDS extension tool: a Tool implementation that routes execution requests
// to a connected UDS client and waits for the result.
//
// When the LLM calls a UDS-registered tool, the agent sends an `execute_tool`
// event to the specific client that registered it, then waits for a
// `tool_result` response (with timeout).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};

/// A tool provided by an external UDS extension client.
///
/// Execution sends an `execute_tool` event to the owning client and waits
/// for a `tool_result` response via a oneshot channel.
pub struct UdsExtensionTool {
    definition: ToolDefinition,
    /// Sender to deliver execute_tool requests to the client.
    /// Each request contains (tool_call_id, arguments, result_sender).
    exec_tx: tokio::sync::mpsc::Sender<UdsToolRequest>,
    /// Maximum time to wait for a tool_result response.
    timeout: std::time::Duration,
}

/// A pending tool execution request.
pub struct UdsToolRequest {
    pub tool_call_id: String,
    pub tool_name: String,
    pub arguments: String,
    pub result_tx: tokio::sync::oneshot::Sender<ToolResult>,
}

impl std::fmt::Debug for UdsExtensionTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UdsExtensionTool")
            .field("name", &self.definition.name)
            .finish()
    }
}

impl UdsExtensionTool {
    /// Create a new UDS extension tool.
    pub fn new(
        definition: ToolDefinition,
        exec_tx: tokio::sync::mpsc::Sender<UdsToolRequest>,
        timeout: std::time::Duration,
    ) -> Self {
        Self {
            definition,
            exec_tx,
            timeout,
        }
    }
}

impl Tool for UdsExtensionTool {
    fn definition(&self) -> ToolDefinition {
        self.definition.clone()
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let arguments = arguments.to_string();
        let tool_name = self.definition.name.to_string();
        let timeout = self.timeout;
        let exec_tx = self.exec_tx.clone();

        Box::pin(async move {
            let tool_call_id = format!("uds-{}", uuid_v4());
            let (result_tx, result_rx) = tokio::sync::oneshot::channel();

            let request = UdsToolRequest {
                tool_call_id,
                tool_name: tool_name.clone(),
                arguments,
                result_tx,
            };

            // Send the execution request to the client handler.
            if exec_tx.send(request).await.is_err() {
                return Ok(ToolResult {
                    content: format!(
                        "Extension disconnected: tool '{}' is no longer available",
                        tool_name
                    ),
                    is_error: true,
                    image_blocks: vec![],
                });
            }

            // Wait for the result with timeout.
            match tokio::time::timeout(timeout, result_rx).await {
                Ok(Ok(result)) => Ok(result),
                Ok(Err(_)) => Ok(ToolResult {
                    content: format!(
                        "Extension disconnected during execution of tool '{}'",
                        tool_name
                    ),
                    is_error: true,
                    image_blocks: vec![],
                }),
                Err(_) => Ok(ToolResult {
                    content: format!(
                        "Extension timed out after {}s executing tool '{}'",
                        timeout.as_secs(),
                        tool_name
                    ),
                    is_error: true,
                    image_blocks: vec![],
                }),
            }
        })
    }
}

/// Generate a simple v4-style UUID (random hex).
fn uuid_v4() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    // Mix timestamp with a counter for uniqueness within a process.
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{:016x}-{:08x}", nanos, count)
}

/// Create a new UDS extension tool with a shared execution channel.
///
/// Returns `(tool, request_receiver)`:
/// - `tool`: an `Arc<dyn Tool>` to register in the tool registry
/// - `request_receiver`: the receiver end — the client handler listens on
///   this to route `execute_tool` events to the correct UDS client
pub fn create_uds_tool(
    definition: ToolDefinition,
    timeout: std::time::Duration,
) -> (Arc<dyn Tool>, tokio::sync::mpsc::Receiver<UdsToolRequest>) {
    let (tx, rx) = tokio::sync::mpsc::channel(16);
    let tool = Arc::new(UdsExtensionTool::new(definition, tx, timeout));
    (tool, rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_def(name: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.to_string().into(),
            description: "Test tool".into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
        }
    }

    #[tokio::test]
    async fn test_execute_returns_result_from_extension() {
        let (tool, mut rx) =
            create_uds_tool(test_def("weather"), std::time::Duration::from_secs(5));

        // Simulate extension responding.
        let handle = tokio::spawn(async move {
            let req = rx.recv().await.unwrap();
            assert_eq!(req.tool_name, "weather");
            let _ = req.result_tx.send(ToolResult {
                content: "22°C, sunny".into(),
                is_error: false,
                image_blocks: vec![],
            });
        });

        let result = tool.execute(r#"{"city":"London"}"#).await.unwrap();
        assert_eq!(result.content, "22°C, sunny");
        assert!(!result.is_error);
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn test_execute_timeout_returns_error() {
        let (tool, _rx) = create_uds_tool(test_def("slow"), std::time::Duration::from_millis(50));

        // Don't respond — let it timeout.
        let result = tool.execute("{}").await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("timed out"));
    }

    #[tokio::test]
    async fn test_execute_disconnected_sender_returns_error() {
        let (tool, rx) = create_uds_tool(test_def("gone"), std::time::Duration::from_secs(5));

        // Drop the receiver to simulate disconnect.
        drop(rx);

        let result = tool.execute("{}").await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("disconnected"));
    }

    #[tokio::test]
    async fn test_execute_receiver_dropped_during_wait() {
        let (tool, mut rx) =
            create_uds_tool(test_def("drop_mid"), std::time::Duration::from_secs(5));

        // Receive the request but drop the result_tx without sending.
        let handle = tokio::spawn(async move {
            let req = rx.recv().await.unwrap();
            drop(req.result_tx);
        });

        let result = tool.execute("{}").await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("disconnected"));
        handle.await.unwrap();
    }

    #[test]
    fn test_definition_returns_correct_name() {
        let (tool, _rx) = create_uds_tool(test_def("weather"), std::time::Duration::from_secs(5));
        assert_eq!(tool.definition().name.as_ref(), "weather");
    }

    #[test]
    fn test_uuid_v4_uniqueness() {
        let a = uuid_v4();
        let b = uuid_v4();
        assert_ne!(a, b);
    }
}
