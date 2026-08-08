use crate::domain::tool_descriptor::ProfileAvailabilityScope;
use serde::{Deserialize, Serialize};

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
    /// own. Omit `count` for the default protocol page, set `count` for an
    /// older-client newest-slice request, and set `before` to page backward.
    /// It is never silently answered from the connected/parent agent's history.
    GetMessages {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        count: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before: Option<String>,
        #[serde(rename = "agent_id", default, skip_serializing_if = "Option::is_none")]
        agent_id: Option<String>,
    },
    /// Return committed transcript changes after `sinceRev` in `epoch`.
    Sync {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        epoch: u64,
        #[serde(rename = "sinceRev")]
        since_rev: u64,
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
    /// Switch the active reasoning-effort level at runtime (#1067).
    /// Session-scoped: validated against the active model's provider
    /// vocabulary and applied to every subsequent turn.
    SetEffort {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        effort: String,
    },
    /// Return the complete rich tool catalogue for control/query clients.
    #[serde(alias = "list_tools")]
    GetToolCatalogue {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// Mutate profile-owned live tool policy through the catalogue-backed path.
    #[serde(rename_all = "camelCase")]
    SetToolPolicy {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        mutations: Vec<ToolPolicyMutationCommand>,
        #[serde(default = "default_tool_policy_apply_mode")]
        mode: ToolPolicyApplyModeCommand,
        #[serde(default = "default_tool_policy_operation")]
        operation: ToolPolicyOperationCommand,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unlisted_scope: Option<ProfileAvailabilityScope>,
    },
    /// Force a provider/model config reload.
    Reload {
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
    ///
    /// Prefer `messageId`. `messageIndex` is retained for one-window-older
    /// clients (#1059) and honoured only while the conversation fits in one
    /// history page; beyond that it is rejected as ambiguous.
    RewindTo {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(
            rename = "messageIndex",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        message_index: Option<usize>,
        #[serde(rename = "messageId", default, skip_serializing_if = "Option::is_none")]
        message_id: Option<String>,
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
    /// Terminate and remove every tracked sub-agent.
    DeleteAllSubagents {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    GetMessage {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        /// Stable domain message UUID (wire: camelCase `messageId`).
        #[serde(rename = "messageId")]
        message_id: String,
        /// When set, forward lookup to a spawned child (same as get_messages).
        #[serde(skip_serializing_if = "Option::is_none")]
        agent_id: Option<String>,
        /// Select a tool call whose arguments should be recovered instead of message content.
        #[serde(
            rename = "toolCallId",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        tool_call_id: Option<String>,
        /// Byte offset for ranged content or tool-call argument recovery (#1094/#1107).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        offset: Option<usize>,
        /// Maximum bytes of content to return for ranged recovery (#1094).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<usize>,
    },
}
/// Tool registration payload for `register_tools`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ToolRegistration {
    pub name: String,
    pub description: String,
    #[serde(rename = "parametersSchema", default = "default_params_schema")]
    pub parameters_schema: String,
    #[serde(rename = "stableId", default, skip_serializing_if = "Option::is_none")]
    pub stable_id: Option<String>,
}
fn default_params_schema() -> String {
    r#"{"type":"object"}"#.to_string()
}

/// One requested catalogue-backed tool policy mutation.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolPolicyMutationCommand {
    /// Stable catalogue id when known by the caller. Current registry mutation
    /// keys are tool names, so this is accepted as an alias for `name` until the
    /// domain mutator grows stable-id addressing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub scope: ProfileAvailabilityScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Wire spelling for live tool policy reconciliation timing.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ToolPolicyApplyModeCommand {
    ImmediateIfIdle,
    AtNextTurnBoundary,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ToolPolicyOperationCommand {
    Patch,
    Replace,
}

fn default_tool_policy_apply_mode() -> ToolPolicyApplyModeCommand {
    ToolPolicyApplyModeCommand::ImmediateIfIdle
}

fn default_tool_policy_operation() -> ToolPolicyOperationCommand {
    ToolPolicyOperationCommand::Patch
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
            Self::Sync { id, .. } => id.as_deref(),
            Self::GetToolCatalogue { id } => id.as_deref(),
            Self::SetToolPolicy { id, .. } => id.as_deref(),
            Self::Reload { id } => id.as_deref(),
            Self::GetMessagesTail { id, .. } => id.as_deref(),
            Self::GetSessionStats { id } => id.as_deref(),
            Self::ListModels { id } => id.as_deref(),
            Self::ListSessions { id } => id.as_deref(),
            Self::NewSession { id } => id.as_deref(),
            Self::ResumeSession { id, .. } => id.as_deref(),
            Self::SetModel { id, .. } => id.as_deref(),
            Self::SetEffort { id, .. } => id.as_deref(),
            Self::RegisterTools { id, .. } => id.as_deref(),
            Self::UnregisterTools { id, .. } => id.as_deref(),
            Self::ToolResult { .. } => None,
            Self::ClearHistory { id } => id.as_deref(),
            Self::RewindTo { id, .. } => id.as_deref(),
            Self::SetWorkflowAutomation { id, .. } => id.as_deref(),
            Self::GetSubagents { id } => id.as_deref(),
            Self::DeleteAllSubagents { id } => id.as_deref(),
            Self::GetMessage { id, .. } => id.as_deref(),
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
            Self::Sync { .. } => "sync",
            Self::GetMessagesTail { .. } => "get_messages_tail",
            Self::GetSessionStats { .. } => "get_session_stats",
            Self::ListModels { .. } => "list_models",
            Self::ListSessions { .. } => "list_sessions",
            Self::NewSession { .. } => "new_session",
            Self::ResumeSession { .. } => "resume_session",
            Self::SetModel { .. } => "set_model",
            Self::SetEffort { .. } => "set_effort",
            Self::GetToolCatalogue { .. } => "get_tool_catalogue",
            Self::SetToolPolicy { .. } => "set_tool_policy",
            Self::Reload { .. } => "reload",
            Self::RegisterTools { .. } => "register_tools",
            Self::UnregisterTools { .. } => "unregister_tools",
            Self::ToolResult { .. } => "tool_result",
            Self::ClearHistory { .. } => "clear_history",
            Self::RewindTo { .. } => "rewind_to",
            Self::SetWorkflowAutomation { .. } => "set_workflow_automation",
            Self::GetSubagents { .. } => "get_subagents",
            Self::DeleteAllSubagents { .. } => "delete_all_subagents",
            Self::GetMessage { .. } => "get_message",
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
