/// Agent events — the canonical domain representation.
///
/// These mirror the quecto UDS protocol events but are owned by quecto-api's
/// domain layer, decoupled from the wire format. The infrastructure layer
/// is responsible for converting to/from the UDS framed JSON format.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    AgentStart,
    AgentEnd {
        /// Legacy field: empty after harness #1060 (content is not re-carried).
        #[serde(default)]
        messages: Vec<serde_json::Value>,
        /// #1060: stable domain message ids identifying the run's messages.
        /// Preserved so WS/API clients can resolve them via `get_message`.
        #[serde(rename = "messageRefs", default)]
        message_refs: Vec<String>,
    },
    Token {
        token: String,
    },
    TurnStart,
    TurnEnd {
        message: serde_json::Value,
        #[serde(rename = "toolResults", default)]
        tool_results: Vec<serde_json::Value>,
    },
    ToolExecutionStart {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        args: serde_json::Value,
    },
    ToolExecutionEnd {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        result: serde_json::Value,
        #[serde(rename = "isError")]
        is_error: bool,
    },
    Response {
        #[serde(default)]
        id: Option<String>,
        command: String,
        success: bool,
        #[serde(default)]
        data: Option<serde_json::Value>,
        #[serde(default)]
        error: Option<String>,
    },
    /// Rich catalogue changed after tool registration/unregistration.
    #[serde(rename_all = "camelCase")]
    ToolCatalogueChanged {
        changed_tools: Vec<String>,
        #[serde(default)]
        before: Vec<serde_json::Value>,
        #[serde(default)]
        after: Vec<serde_json::Value>,
        reason: String,
    },
    /// A spawned child appended messages this turn (#1060: refs-based, so the
    /// full content is not re-carried). Preserved rather than falling through to
    /// `Unknown` so clients keep the child's message identity.
    SubagentMessagesAppended {
        #[serde(rename = "agent_id", alias = "agentId")]
        agent_id: String,
        #[serde(default)]
        messages: Vec<serde_json::Value>,
        #[serde(rename = "messageRefs", default)]
        message_refs: Vec<String>,
    },
    /// Catch-all for unknown/future event types.
    #[serde(other)]
    Unknown,
}
