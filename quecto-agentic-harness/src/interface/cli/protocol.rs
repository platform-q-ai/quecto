/// UDS protocol types and wire-format helpers for `quecto agent --mode uds`.
/// Length-prefixed UTF-8 JSON protocol over a Unix domain socket.
use serde::{Deserialize, Serialize};
/// Authoritative protocol page size for paged conversation history (#1061).
///
/// The single definition every producer, consumer, and test suite shares —
/// per-call-site literals are forbidden by the #1061 constraint. `pub` (not
/// crate-private) so the BDD integration targets assert against the same value.
pub const HISTORY_PAGE_SIZE: usize = 64;
// ─── Public parse helper ──────────────────────────────────────────────────────
/// Parse a single JSON line into an [`AgentCommand`].
/// Returns `Err` for invalid JSON or an unrecognised command type.
pub fn parse_command_line(line: &str) -> Result<AgentCommand, String> {
    if line.trim().is_empty() {
        return Err("empty line".to_string());
    }
    serde_json::from_str(line).map_err(|e| format!("parse error: {e}"))
}
#[path = "protocol_commands.rs"]
mod protocol_commands;
pub use protocol_commands::{
    AgentCommand, StreamingBehavior, ToolPolicyApplyModeCommand, ToolPolicyMutationCommand,
    ToolRegistration,
};

// ─── Events (stdout) ─────────────────────────────────────────────────────────

/// An event emitted over the UDS socket.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    /// Agent begins processing a prompt.
    AgentStart,
    /// Announces the harness workspace path to socket clients (#1350).
    #[serde(rename_all = "camelCase")]
    Workspace { path: String },
    /// Agent finished processing. Identifies this run's messages by stable refs
    /// (#1060 / ADR-0008 part 2). Legacy full-content `messages` is emptied;
    /// clients that already hold stream tokens need no fetch; partial observers
    /// use `get_message` by ref. This is a turn boundary only; workflow
    /// continuation may still follow.
    AgentEnd {
        /// Legacy field: always empty after #1060 (content is not re-carried).
        #[serde(default)]
        messages: Vec<serde_json::Value>,
        /// Stable domain message ids for messages appended during this run.
        #[serde(rename = "messageRefs", default)]
        message_refs: Vec<String>,
    },
    /// The post-turn drain made no further workflow continuation runnable.
    /// Emitted only after pending work and automatic workflow nudges settle.
    /// `reason` distinguishes intervention-worthy exhaustion from deliberate
    /// outcomes (explicit abort, completion) so supervisors don't raise a
    /// stall alert for a stop they requested (#1082 review). Optional for
    /// wire back-compat: events from older producers deserialize with
    /// `reason: None` and must never be classified as a stall.
    WorkflowIdle {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<WorkflowIdleReason>,
    },
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
    /// Rich catalogue changed after tool registration/unregistration.
    #[serde(rename_all = "camelCase")]
    ToolCatalogueChanged {
        changed_tools: Vec<String>,
        before: Vec<serde_json::Value>,
        after: Vec<serde_json::Value>,
        reason: String,
    },
    /// Runtime tool policy changed after live reconciliation.
    #[serde(rename_all = "camelCase")]
    ToolPolicyChanged {
        changed_tools: Vec<String>,
        results: Vec<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        child_propagation: Vec<serde_json::Value>,
        apply_mode: String,
        reason: String,
    },
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
        /// Legacy field: always empty after #1060 (content is not re-carried).
        #[serde(default)]
        messages: Vec<serde_json::Value>,
        /// Stable domain message ids for messages appended during this child turn.
        #[serde(rename = "messageRefs", default)]
        message_refs: Vec<String>,
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

/// Why a `workflow_idle` boundary was reached (#1082 review). Only
/// intervention-worthy reasons (`Exhausted`) should be classified as a stall
/// by supervising monitors; deliberate stops must stay silent. `Unknown`
/// absorbs reason strings from newer producers so typed deserialization
/// never fails across versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowIdleReason {
    /// Auto-continuation gave up: no-progress tolerance or nudge cap reached,
    /// or no nudge was applicable while the workflow is still unfinished.
    Exhausted,
    /// The parent explicitly aborted; this stop was requested, not a stall.
    ExplicitAbort,
    /// The bound workflow reached a terminal state (or no workflow is bound).
    Completed,
    /// A reason this build does not recognize (newer producer). Never
    /// classified as a stall.
    #[serde(other)]
    Unknown,
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
    /// Whether this sub-agent was spawned read-only (`write` + `edit` disabled).
    /// The TUI renders an observer marker next to its name when true (#966).
    #[serde(default)]
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnMessage {
    pub role: String,
    /// Legacy full content — emptied on the wire after #1060 (ADR-0008 part 2).
    /// Clients reconstruct from stream tokens or `get_message` by ref.
    #[serde(default)]
    pub content: String,
    /// Stable domain message ids for messages completed in this LLM call.
    #[serde(rename = "messageRefs", default, skip_serializing_if = "Vec::is_empty")]
    pub message_refs: Vec<String>,
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
    /// Byte length of the assistant text for this turn (not the body). Clients
    /// use this to detect truncated streams without re-carrying content (#1060).
    #[serde(rename = "contentLength", skip_serializing_if = "Option::is_none")]
    pub content_length: Option<u64>,
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
                read_only: entry.read_only,
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

// ─── Event line cap (#1047) ──────────────────────────────────────────────────
//
// The authoritative value lives in `crate::infrastructure::line_cap` so
// interface and infrastructure emitters share the reader-side framing bound.
pub use crate::infrastructure::line_cap::{EVENT_LINE_CAP_BYTES, EVENT_LINE_JSON_BUDGET};

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
    /// The session's effective reasoning-effort level (#1067): the level
    /// string when set (config default or runtime `set_effort`), an explicit
    /// `null` when unset — so clients can distinguish "provider default"
    /// from a missing capability. Always serialized, on both the live-query
    /// and busy-connect snapshot paths.
    #[serde(default)]
    pub effort: Option<String>,
    /// The effort vocabulary valid for the active model's provider (#1067).
    /// Agent-side source of truth so clients never duplicate the
    /// provider→levels rule.
    #[serde(default)]
    pub effort_levels: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution: Option<super::uds_execution_state::ExecutionSnapshot>,
    #[serde(default)]
    pub sync: u8,
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
    #[serde(skip_serializing, default)]
    pub cost: f64,
    /// User-facing context occupancy: provider-reported prompt tokens when
    /// available, otherwise the local active-message estimate fallback.
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
#[path = "protocol_event_tests.rs"]
mod event_tests;

#[cfg(test)]
#[path = "protocol_policy_tests.rs"]
mod policy_tests;
#[cfg(test)]
#[path = "protocol_shape_tests.rs"]
mod shape_tests;

#[cfg(test)]
#[path = "protocol_1060_tests.rs"]
mod protocol_1060_tests;
