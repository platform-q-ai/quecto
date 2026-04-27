/// UDS session state — in-memory tracker and statistics for an active UDS connection.
use crate::domain::message::{Message, Role};
use crate::infrastructure::tools::subagent_registry::SubagentNotification;

use super::protocol::{SessionState, SessionStats, TokenStats};

// ─── Session state tracker ────────────────────────────────────────────────────

/// In-memory state for an active UDS session.
pub struct AgentSession {
    model: String,
    session_key: String,
    streaming: bool,
    /// `VecDeque` supports O(1) push_back (enqueue) and push_front (prepend/steer).
    pending: std::collections::VecDeque<String>,
    seen_subagent_notifications: std::collections::HashSet<(String, u64)>,
}

impl AgentSession {
    pub fn new(model: String, session_key: String) -> Self {
        Self {
            model,
            session_key,
            streaming: false,
            pending: std::collections::VecDeque::new(),
            seen_subagent_notifications: std::collections::HashSet::new(),
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
            self.pending.push_back(msg);
        }
        // Silently drop if the queue is full — caller already got a success ack.
    }

    pub fn enqueue_subagent_notification(&mut self, notif: &SubagentNotification) -> bool {
        let key = notif.dedupe_key();
        if !self.seen_subagent_notifications.insert(key) {
            return false;
        }
        if self.pending.len() < Self::MAX_PENDING {
            self.pending.push_back(notif.to_message());
            true
        } else {
            false
        }
    }

    /// Prepend a message to the front of the pending queue so it runs before
    /// any earlier-enqueued follow-ups.  Used by `steer` for interrupt semantics.
    /// O(1) with `VecDeque`, unlike `Vec::insert(0)`.
    pub fn prepend_pending(&mut self, msg: String) {
        if self.pending.len() < Self::MAX_PENDING {
            self.pending.push_front(msg);
        }
    }

    pub fn drain_pending(&mut self) -> Vec<String> {
        // Vec::from(VecDeque) calls make_contiguous() then ptr::copy when the
        // deque's head != 0 — O(n) in the number of elements, same as the
        // previous .into_iter().collect().  Pending queue is capped at 64
        // entries so worst case is ~64 fat-pointer copies (~1.5 KiB).
        Vec::from(std::mem::take(&mut self.pending))
    }

    pub fn state_snapshot(
        &self,
        message_count: usize,
        workflow: Option<serde_json::Value>,
    ) -> SessionState {
        SessionState {
            model: self.model.clone(),
            is_streaming: self.streaming,
            session_key: self.session_key.clone(),
            message_count,
            pending_message_count: self.pending.len(),
            workflow,
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

/// Clear conversation history, preserving only the injected system prompt (non-manifest).
/// Uses `truncate` instead of `clone+clear` to avoid copying the system message.
pub fn clear_conversation(messages: &mut Vec<Message>) {
    let keep = messages
        .first()
        .is_some_and(|m| m.role == Role::System && !m.is_manifest);
    if keep {
        messages.truncate(1);
    } else {
        messages.clear();
    }
}

#[cfg(test)]
mod subagent_notification_dedupe_tests {
    use super::*;
    use crate::infrastructure::tools::subagent_registry::SubagentNotification;

    #[test]
    fn same_monotonic_subagent_notification_is_enqueued_once() {
        let mut session = AgentSession::new("m".into(), "s".into());
        let notification = SubagentNotification::Completed {
            agent_id: "worker".into(),
            sequence: 1,
            summary: "done".into(),
        };

        assert!(session.enqueue_subagent_notification(&notification));
        assert!(!session.enqueue_subagent_notification(&notification));

        assert_eq!(session.drain_pending().len(), 1);
    }

    #[test]
    fn later_monotonic_subagent_notification_is_enqueued() {
        let mut session = AgentSession::new("m".into(), "s".into());
        let first = SubagentNotification::Completed {
            agent_id: "worker".into(),
            sequence: 1,
            summary: "first".into(),
        };
        let second = SubagentNotification::Completed {
            agent_id: "worker".into(),
            sequence: 2,
            summary: "second".into(),
        };

        assert!(session.enqueue_subagent_notification(&first));
        assert!(session.enqueue_subagent_notification(&second));

        let pending = session.drain_pending();
        assert_eq!(pending.len(), 2);
        assert!(pending[0].contains("first"));
        assert!(pending[1].contains("second"));
    }
}
