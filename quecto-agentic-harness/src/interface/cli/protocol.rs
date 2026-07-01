/// UDS protocol types and wire-format helpers for `quecto agent --mode uds`.
///
/// JSON-lines protocol over a Unix domain socket.  One JSON object per line.
/// All commands carry an optional `id` field for request/response correlation.
use serde::{Deserialize, Serialize};

// ─── Public parse helper ──────────────────────────────────────────────────────

/// Parse a single JSON line into an [`AgentCommand`].
/// Returns `Err` for invalid JSON or an unrecognised command type.
pub fn parse_command_line(line: &str) -> Result<AgentCommand, String> {
    if line.trim().is_empty() {
        return Err("empty line".to_string());
    }
    serde_json::from_str(line).map_err(|e| format!("parse error: {e}"))
}

// ─── Commands (stdin) ────────────────────────────────────────────────────────

/// A command received over the UDS socket.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentCommand {
    /// Send a user message to the agent.
    Prompt {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        message: String,
        /// Required when the agent is currently running.
        #[serde(rename = "streamingBehavior", skip_serializing_if = "Option::is_none")]
        streaming_behavior: Option<StreamingBehavior>,
    },
    /// Interrupt after the current tool, then deliver this message.
    Steer {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        message: String,
    },
    /// Deliver this message when the agent finishes.
    FollowUp {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        message: String,
    },
    /// Cancel the current agent run.
    Abort {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// Return current session state.
    GetState {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// Return conversation history. Optional `count` returns the last N messages.
    ///
    /// When `agent_id` is set, the request is forwarded to that spawned
    /// sub-agent and its history is returned instead of the connected agent's
    /// own — with `count` the tail, without `count` the full history (#843). It
    /// is never silently answered from the connected/parent agent's history.
    GetMessages {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        count: Option<usize>,
        #[serde(rename = "agent_id", default, skip_serializing_if = "Option::is_none")]
        agent_id: Option<String>,
    },
    /// Deprecated alias for `get_messages` with `count`.
    ///
    /// When `agent_id` is set, the request is forwarded to that spawned
    /// sub-agent (reusing the `agent_cmd get_messages` capability) and its
    /// message tail is returned instead of the connected agent's own history.
    GetMessagesTail {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        count: usize,
        #[serde(rename = "agent_id", default, skip_serializing_if = "Option::is_none")]
        agent_id: Option<String>,
    },
    /// Return token usage and cost statistics.
    GetSessionStats {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// Return configured and built-in models from the runtime registry.
    ListModels {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// Compatibility alias for clients that ask to get the runtime model list.
    GetModels {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// Return persisted CLI sessions available for resume.
    ListSessions {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// Switch to a fresh user-chat session.
    NewSession {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// Switch the active UDS session to a persisted CLI session.
    ResumeSession {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        session: String,
    },
    /// Switch the active model at runtime.
    ///
    /// Accepts either:
    /// - legacy `{ "model": "provider/modelId" }`, or
    /// - compatible `{ "provider": "...", "modelId": "..." }`.
    SetModel {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        provider: Option<String>,
        #[serde(rename = "modelId", skip_serializing_if = "Option::is_none")]
        model_id: Option<String>,
    },
    /// Return the list of registered extensions.
    GetExtensions {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// Force a provider/model config reload.
    Reload {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// Re-scan extension directories and reload script extensions.
    ReloadExtensions {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// Register tools provided by an extension client.
    RegisterTools {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        tools: Vec<ToolRegistration>,
    },
    /// Remove previously registered tools.
    UnregisterTools {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        tools: Vec<String>,
    },
    /// Return a tool execution result (response to an `execute_tool` event).
    ToolResult {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        content: String,
        #[serde(rename = "isError", default)]
        is_error: bool,
    },
    /// Clear conversation history in-place without restarting the agent.
    ClearHistory {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// Rewind conversation history to a selected user-message boundary.
    RewindTo {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(rename = "messageIndex")]
        message_index: usize,
    },
    /// Toggle core workflow automation for this UDS session.
    SetWorkflowAutomation {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(rename = "autoContinue", skip_serializing_if = "Option::is_none")]
        auto_continue: Option<bool>,
        #[serde(rename = "completionNudge", skip_serializing_if = "Option::is_none")]
        completion_nudge: Option<bool>,
    },
    /// Return the current list of spawned subagents and their live status (#524).
    GetSubagents {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
}

/// Tool registration payload for `register_tools`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ToolRegistration {
    pub name: String,
    pub description: String,
    #[serde(rename = "parametersSchema", default = "default_params_schema")]
    pub parameters_schema: String,
}

fn default_params_schema() -> String {
    r#"{"type":"object"}"#.to_string()
}

impl AgentCommand {
    /// Return the optional correlation id.
    pub fn id(&self) -> Option<&str> {
        match self {
            Self::Prompt { id, .. } => id.as_deref(),
            Self::Steer { id, .. } => id.as_deref(),
            Self::FollowUp { id, .. } => id.as_deref(),
            Self::Abort { id } => id.as_deref(),
            Self::GetState { id } => id.as_deref(),
            Self::GetMessages { id, .. } => id.as_deref(),
            Self::GetExtensions { id } => id.as_deref(),
            Self::Reload { id } => id.as_deref(),
            Self::ReloadExtensions { id } => id.as_deref(),
            Self::GetMessagesTail { id, .. } => id.as_deref(),
            Self::GetSessionStats { id } => id.as_deref(),
            Self::ListModels { id } => id.as_deref(),
            Self::GetModels { id } => id.as_deref(),
            Self::ListSessions { id } => id.as_deref(),
            Self::NewSession { id } => id.as_deref(),
            Self::ResumeSession { id, .. } => id.as_deref(),
            Self::SetModel { id, .. } => id.as_deref(),
            Self::RegisterTools { id, .. } => id.as_deref(),
            Self::UnregisterTools { id, .. } => id.as_deref(),
            Self::ToolResult { .. } => None,
            Self::ClearHistory { id } => id.as_deref(),
            Self::RewindTo { id, .. } => id.as_deref(),
            Self::SetWorkflowAutomation { id, .. } => id.as_deref(),
            Self::GetSubagents { id } => id.as_deref(),
        }
    }

    /// Return the command name string for use in responses.
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Prompt { .. } => "prompt",
            Self::Steer { .. } => "steer",
            Self::FollowUp { .. } => "follow_up",
            Self::Abort { .. } => "abort",
            Self::GetState { .. } => "get_state",
            Self::GetMessages { .. } => "get_messages",
            Self::GetMessagesTail { .. } => "get_messages_tail",
            Self::GetSessionStats { .. } => "get_session_stats",
            Self::ListModels { .. } => "list_models",
            Self::GetModels { .. } => "get_models",
            Self::ListSessions { .. } => "list_sessions",
            Self::NewSession { .. } => "new_session",
            Self::ResumeSession { .. } => "resume_session",
            Self::SetModel { .. } => "set_model",
            Self::GetExtensions { .. } => "get_extensions",
            Self::Reload { .. } => "reload",
            Self::ReloadExtensions { .. } => "reload_extensions",
            Self::RegisterTools { .. } => "register_tools",
            Self::UnregisterTools { .. } => "unregister_tools",
            Self::ToolResult { .. } => "tool_result",
            Self::ClearHistory { .. } => "clear_history",
            Self::RewindTo { .. } => "rewind_to",
            Self::SetWorkflowAutomation { .. } => "set_workflow_automation",
            Self::GetSubagents { .. } => "get_subagents",
        }
    }
}

/// How to handle a `prompt` command when the agent is already running.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum StreamingBehavior {
    /// Interrupt after the current tool; deliver the message next.
    Steer,
    /// Wait until the agent finishes; then deliver the message.
    FollowUp,
}

// ─── Events (stdout) ─────────────────────────────────────────────────────────

/// An event emitted over the UDS socket.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    /// Agent begins processing a prompt.
    AgentStart,
    /// Agent finished processing.  Contains messages from this run as JSON values.
    AgentEnd { messages: Vec<serde_json::Value> },
    /// An incremental text token from the LLM during streaming.
    Token { token: String },
    /// A new LLM call begins.
    TurnStart,
    /// LLM call completed.
    TurnEnd {
        message: TurnMessage,
        #[serde(rename = "toolResults")]
        tool_results: Vec<ToolResultEvent>,
    },
    /// A tool began executing.
    ToolExecutionStart {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        args: serde_json::Value,
    },
    /// A tool finished executing.
    ToolExecutionEnd {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        result: ToolResultContent,
        #[serde(rename = "isError")]
        is_error: bool,
    },
    /// Response to a command.
    Response {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        command: String,
        success: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        data: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// Extension list changed (after reload or hot-reload).
    ExtensionsChanged { extensions: Vec<ExtensionInfo> },
    /// Request sent to an extension client to execute a tool.
    ///
    /// Routed only to the client that registered the tool — not broadcast.
    ExecuteTool {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        arguments: String,
    },
    /// Broadcast when a subagent emits a passive notification for human/UI visibility.
    #[serde(rename_all = "camelCase")]
    SubagentNotification {
        agent_id: String,
        sequence: u64,
        message: String,
    },
    /// Broadcast when a subagent's status changes (#524).
    /// Contains the full list of subagents (clients do a simple replace).
    SubagentStateChanged { subagents: Vec<SubagentInfo> },
    /// Emitted when an agent completes a turn, carrying the messages appended
    /// during that turn (assistant message + any tool results). A sub-agent
    /// emits this on its own stream with an empty `agent_id`; the parent's
    /// monitor re-stamps it with the child's id and forwards it so the TUI
    /// inspector can stream the sub-agent's output turn-by-turn (#797).
    SubagentMessagesAppended {
        agent_id: String,
        messages: Vec<serde_json::Value>,
    },
    /// Broadcast when workflow state changes.
    #[serde(rename_all = "camelCase")]
    WorkflowState {
        enabled: bool,
        guards_enabled: bool,
        mode: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        active_template: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        active_issue: Option<serde_json::Value>,
        progress: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        current_step: Option<serde_json::Value>,
        steps: Vec<serde_json::Value>,
        available_templates: Vec<serde_json::Value>,
    },
}

/// Snapshot of a single subagent's state, used in `get_subagents` responses
/// and `subagent_state_changed` events (#524).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentInfo {
    /// Unique agent identifier (matches the spawn `agent_id`).
    pub agent_id: String,
    /// Live status: "starting", "idle", "running", "error", "exited".
    pub status: String,
    /// Name of the last tool being executed, or `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_tool: Option<String>,
    /// Description of the last error, or `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// Child process PID.
    pub pid: u32,
    /// Filesystem path to this sub-agent's own UDS socket, surfaced so the TUI
    /// can open a direct connect-on-select connection to its live stream (#800).
    /// Local use only — sub-agent sockets are filesystem-permission scoped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub socket_path: Option<String>,
    /// The spawning agent's id, for reconstructing the unit tree (PRD Stage B).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    /// Latest workflow snapshot for this subagent, if any (PRD Stage B).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow: Option<crate::infrastructure::tools::subagent_registry::WorkflowSnapshot>,
}

/// Metadata for a registered extension, used in `ExtensionsChanged` events
/// and `get_extensions` responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionInfo {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TurnUsage>,
    #[serde(rename = "stopReason", skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    /// Estimated tokens currently occupying the active, pruned context.
    #[serde(rename = "contextTokens", skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<u64>,
    /// The active model's context-window limit (tokens), so the TUI footer can
    /// render context usage against it. `None` when unknown.
    #[serde(rename = "maxContextTokens", skip_serializing_if = "Option::is_none")]
    pub max_context_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnUsage {
    pub input: u32,
    pub output: u32,
    pub total: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultEvent {
    #[serde(rename = "toolCallId")]
    pub tool_call_id: String,
    pub content: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultContent {
    pub content: Vec<serde_json::Value>,
}

// ─── Subagent helpers (#524) ──────────────────────────────────────────────────

/// Build a sorted list of [`SubagentInfo`] from the shared registry.
///
/// Acquires the registry mutex briefly and maps each entry to the protocol type.
/// Returns an empty vec if the registry is `None` or empty.
pub fn build_subagent_info_list(
    registry: &Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
) -> Vec<SubagentInfo> {
    let Some(reg) = registry else {
        return Vec::new();
    };
    let mut list: Vec<SubagentInfo> = {
        let guard = reg.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .iter()
            .map(|(id, entry)| SubagentInfo {
                agent_id: id.clone(),
                status: entry.status.to_wire_str().to_string(),
                last_tool: entry.last_tool.clone(),
                last_error: entry.last_error.clone(),
                pid: entry.pid,
                socket_path: Some(entry.socket_path.to_string_lossy().into_owned()),
                parent_id: entry.parent_id.clone(),
                workflow: entry.workflow.clone(),
            })
            .collect()
    }; // guard dropped here — sort happens outside critical section
    list.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));
    list
}

/// The unit tree reconstructed purely from an identity-tagged event stream
/// (PRD Stage B / R-B4). Each `workflow_state` / subagent event carries
/// `agent_id` + `parent_id`, so any consumer can rebuild the parent→child
/// structure without a side channel.
#[derive(Debug, Default, Clone)]
pub struct UnitTree {
    /// agent_id → its parent_id (None at the root).
    parents: std::collections::HashMap<String, Option<String>>,
}

impl UnitTree {
    /// Build the tree from a slice of identity-tagged events. Any event with an
    /// `agent_id` contributes a node; later events overwrite earlier parentage.
    pub fn from_events(events: &[serde_json::Value]) -> Self {
        let mut parents = std::collections::HashMap::new();
        for ev in events {
            if let Some(agent) = ev.get("agent_id").and_then(|v| v.as_str()) {
                let parent = ev
                    .get("parent_id")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                parents.insert(agent.to_string(), parent);
            }
        }
        Self { parents }
    }

    /// Direct parent of `agent`, or `None` if it is a root (or unknown).
    pub fn parent_of(&self, agent: &str) -> Option<&str> {
        self.parents.get(agent).and_then(|p| p.as_deref())
    }
}

// ─── Response helpers ────────────────────────────────────────────────────────

impl AgentEvent {
    /// Build a success response.
    pub fn ok(id: Option<&str>, command: &str, data: Option<serde_json::Value>) -> Self {
        Self::Response {
            id: id.map(str::to_owned),
            command: command.to_owned(),
            success: true,
            data,
            error: None,
        }
    }

    /// Build an error response.
    pub fn err(id: Option<&str>, command: &str, message: impl Into<String>) -> Self {
        Self::Response {
            id: id.map(str::to_owned),
            command: command.to_owned(),
            success: false,
            data: None,
            error: Some(message.into()),
        }
    }

    /// Serialize the event to a JSON line (no trailing newline).
    pub fn to_json_line(&self) -> String {
        serde_json::to_string(self).expect("AgentEvent is always serializable")
    }
}

// ─── Session state snapshot ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionState {
    pub model: String,
    pub is_streaming: bool,
    pub session_key: String,
    pub message_count: usize,
    pub pending_message_count: usize,
    /// The active model's context-window limit (tokens). `0` when unknown.
    pub max_context_tokens: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow: Option<serde_json::Value>,
}

// ─── Session statistics ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStats {
    pub session_key: String,
    pub user_messages: usize,
    pub assistant_messages: usize,
    pub tool_calls: usize,
    pub tool_results: usize,
    pub total_messages: usize,
    pub tokens: TokenStats,
    pub cost: f64,
    /// Estimated tokens currently occupying the active, pruned context.
    pub context_tokens: usize,
    /// The active model's context-window limit (tokens). `0` when unknown.
    pub max_context_tokens: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TokenStats {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub total: u64,
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "protocol_shape_tests.rs"]
mod shape_tests;
