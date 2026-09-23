//! What the agent loop tells the commander, and the port it tells it through.
use serde::Serialize;

/// One observed event. Everything here is raw fact from the loop; the
/// commander decides what to ask about it.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CommanderEvent {
    /// An agent's own turn ended (#5, #22; #6 when the agent is a child).
    TurnEnd {
        turn: u32,
        /// The user/parent message that started this run of the loop.
        prompt: String,
        final_text: String,
        stop_reason: Option<String>,
        tool_rounds: u32,
        /// `iteration_limit` when the loop stopped at its tool-round cap.
        ended_by: String,
        output_tokens: Option<u32>,
        max_tokens: u32,
    },
    /// A provider request failed (#12, #22).
    ProviderFailure {
        turn: u32,
        provider: String,
        class: String,
        http_status: Option<u16>,
        error: String,
        /// `transient_retry`, `malformed_retry` or `terminal`.
        outcome: String,
        attempt: u32,
    },
    /// A tool call returned an error (#22).
    ToolError {
        turn: u32,
        tool: String,
        arguments: String,
        result: String,
    },
    /// A sub-agent notification reached this (parent) agent (#22).
    SubagentNotice {
        child: String,
        child_uuid: Option<String>,
        notice: String,
        detail: Option<String>,
    },
}

/// Fire-and-forget: `observe` must return immediately and never fail the
/// caller; the implementation queues the event and judges it elsewhere.
pub trait CommanderSink: Send + Sync + std::fmt::Debug {
    fn observe(&self, session_key: &str, model: &str, event: CommanderEvent);
}
