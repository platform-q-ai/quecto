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
use crate::domain::extension_tool::ToolInvocation;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};

/// A tool provided by an external UDS extension client.
///
/// Execution sends an `execute_tool` event to the owning client and waits
/// for a `tool_result` response via a oneshot channel.
pub struct UdsExtensionTool {
    definition: ToolDefinition,
    /// Sender delivers `ToolInvocation`s to whichever transport is registered
    /// for this tool.
    exec_tx: tokio::sync::mpsc::Sender<ToolInvocation>,
    /// Maximum time to wait for a tool_result response.
    timeout: std::time::Duration,
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
        exec_tx: tokio::sync::mpsc::Sender<ToolInvocation>,
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

            let request = ToolInvocation {
                tool_call_id,
                tool_name: tool_name.clone(),
                arguments,
                reply: result_tx,
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
                    delivery_metadata: None,
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
                    delivery_metadata: None,
                }),
                Err(_) => Ok(ToolResult {
                    content: format!(
                        "Extension timed out after {}s executing tool '{}'",
                        timeout.as_secs(),
                        tool_name
                    ),
                    is_error: true,
                    image_blocks: vec![],
                    delivery_metadata: None,
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
) -> (Arc<dyn Tool>, tokio::sync::mpsc::Receiver<ToolInvocation>) {
    let (tx, rx) = tokio::sync::mpsc::channel(16);
    let tool = Arc::new(UdsExtensionTool::new(definition, tx, timeout));
    (tool, rx)
}

#[cfg(test)]
#[path = "uds_tool_tests.rs"]
mod tests;
