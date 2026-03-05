/// UDS session state — in-memory tracker and statistics for an active UDS connection.
use crate::domain::message::{Message, Role};

use super::protocol::{SessionState, SessionStats, TokenStats};

// ─── Public parse helper (used by unit tests) ────────────────────────────────

/// Parse a single JSON line into an `AgentCommand`.  Returns `Err` for invalid
/// JSON or an unrecognised command type.
pub fn parse_command_line(line: &str) -> Result<super::protocol::AgentCommand, String> {
    if line.trim().is_empty() {
        return Err("empty line".to_string());
    }
    serde_json::from_str(line).map_err(|e| format!("parse error: {e}"))
}

// ─── Session state tracker ────────────────────────────────────────────────────

/// In-memory state for an active UDS session.
pub struct AgentSession {
    model: String,
    session_key: String,
    streaming: bool,
    pending: Vec<String>,
}

impl AgentSession {
    pub fn new(model: String, session_key: String) -> Self {
        Self {
            model,
            session_key,
            streaming: false,
            pending: Vec::new(),
        }
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn is_streaming(&self) -> bool {
        self.streaming
    }

    pub fn set_model(&mut self, model: String) {
        self.model = model;
    }

    pub fn set_streaming(&mut self, v: bool) {
        self.streaming = v;
    }

    /// Maximum number of pending (steer/follow_up) messages buffered at once.
    /// Prevents OOM from a flood of pending messages from a misbehaving client.
    pub const MAX_PENDING: usize = 64;

    pub fn enqueue_pending(&mut self, msg: String) {
        if self.pending.len() < Self::MAX_PENDING {
            self.pending.push(msg);
        }
        // Silently drop if the queue is full — caller already got a success ack.
    }

    /// Prepend a message to the front of the pending queue so it runs before
    /// any earlier-enqueued follow-ups.  Used by `steer` for interrupt semantics.
    pub fn prepend_pending(&mut self, msg: String) {
        if self.pending.len() < Self::MAX_PENDING {
            self.pending.insert(0, msg);
        }
    }

    pub fn drain_pending(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending)
    }

    pub fn state_snapshot(&self, message_count: usize) -> SessionState {
        SessionState {
            model: self.model.clone(),
            is_streaming: self.streaming,
            session_key: self.session_key.clone(),
            message_count,
            pending_message_count: self.pending.len(),
        }
    }
}

// ─── Session statistics ───────────────────────────────────────────────────────

/// Compute session statistics from the current message history.
///
/// Note: token counts are not available from `Message` objects (usage is only
/// on `LlmResponse` which is not persisted on `Message`).  The token fields
/// are zeroed for now; a future enhancement can thread usage through the loop.
pub fn compute_session_stats(session_key: &str, messages: &[Message]) -> SessionStats {
    let mut user_messages = 0usize;
    let mut assistant_messages = 0usize;
    let mut tool_calls_count = 0usize;
    let mut tool_results_count = 0usize;

    for msg in messages {
        match msg.role {
            Role::User => user_messages += 1,
            Role::Assistant => {
                assistant_messages += 1;
                tool_calls_count += msg.tool_calls.len();
            }
            Role::Tool => tool_results_count += 1,
            Role::System => {}
        }
    }

    SessionStats {
        session_key: session_key.to_owned(),
        user_messages,
        assistant_messages,
        tool_calls: tool_calls_count,
        tool_results: tool_results_count,
        total_messages: messages.len(),
        tokens: TokenStats::default(),
        cost: 0.0,
    }
}

/// Return a JSON value containing the last `count` messages in chronological order.
pub fn messages_tail_json(messages: &[Message], count: usize) -> serde_json::Value {
    let skip = messages.len().saturating_sub(count);
    let msgs_json: Vec<serde_json::Value> = messages[skip..].iter().map(message_to_json).collect();
    serde_json::json!({ "messages": msgs_json })
}

/// Serialize a `Message` to a JSON value for protocol emission.
pub fn message_to_json(msg: &Message) -> serde_json::Value {
    serde_json::json!({
        "role": format!("{:?}", msg.role).to_lowercase(),
        "content": msg.content,
        "toolCalls": msg.tool_calls.iter().map(|tc| serde_json::json!({
            "id": tc.id,
            "name": tc.name,
            "arguments": tc.arguments,
        })).collect::<Vec<_>>(),
        "toolCallId": msg.tool_call_id,
        "toolName": msg.tool_name,
    })
}
