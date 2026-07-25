//! End-of-turn transcript recovery policy (#1060/#1221).
//!
//! Pure decision logic for whether a completed turn's rendered text is
//! trustworthy, or whether the turn must be rebuilt from the server's message
//! refs. It holds no chat, client, JSON, or runtime, so the trigger heuristics
//! and the batch's atomicity invariant can be exercised directly.

use std::collections::HashMap;

/// Facts about a just-completed turn, as observed at the presentation edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnOutcome<'a> {
    /// Server message refs for the turn, in order.
    pub refs: &'a [String],
    /// The assistant text currently rendered for the turn.
    pub assistant_text: &'a str,
    /// Tool boxes observed since this turn's `AgentStart`.
    pub tools_this_turn: usize,
    /// Tool starts not yet matched by an end.
    pub open_tool_calls: usize,
    /// Content length the server advertised for the assistant body, if any.
    pub expected_content_len: Option<u64>,
}

impl TurnOutcome<'_> {
    /// Whether this turn must be rebuilt from its refs.
    ///
    /// An unmatched tool call FORCES recovery: the stream was cut mid-turn, so
    /// the rendered text cannot be trusted however plausible it looks. Empty
    /// refs can never trigger recovery — there is nothing to rebuild from.
    pub fn needs_recovery(&self) -> bool {
        if self.refs.is_empty() {
            return false;
        }
        if self.open_tool_calls > 0 {
            return true;
        }
        let trimmed = self.assistant_text.trim();
        if trimmed.is_empty() || trimmed == "…" || trimmed == "..." {
            return true;
        }
        if let Some(expected) = self.expected_content_len
            && (self.assistant_text.len() as u64) < expected
        {
            return true;
        }
        // Each tool contributes a call and a result message, plus the final
        // assistant message; any other count means the stream lost messages.
        let expected_refs = self.tools_this_turn.saturating_mul(2).saturating_add(1);
        self.refs.len() != expected_refs
    }
}

/// A turn awaiting rebuild: every ref must respond before the chat range is
/// replaced, so a partial batch can never leave a half-rebuilt transcript.
#[derive(Debug)]
pub struct RecoveryBatch<T> {
    pub refs: Vec<String>,
    pub responses: HashMap<String, T>,
    /// Chat entry range this turn occupies, replaced atomically on completion.
    pub target_start: usize,
    pub target_end: usize,
    /// Owning session: `None` = master, `Some(id)` = that sub-agent's chat.
    pub agent_id: Option<String>,
}

impl<T> RecoveryBatch<T> {
    pub fn new(
        refs: Vec<String>,
        target_start: usize,
        target_end: usize,
        agent_id: Option<String>,
    ) -> Self {
        Self {
            refs,
            responses: HashMap::new(),
            target_start,
            target_end,
            agent_id,
        }
    }

    /// Whether every ref has responded and the range may now be replaced.
    pub fn is_complete(&self) -> bool {
        self.responses.len() == self.refs.len()
    }

    /// Responses in REF order — the order the turn was streamed in, which is
    /// not the arrival order.
    pub fn ordered_responses(&self) -> impl Iterator<Item = &T> {
        self.refs.iter().filter_map(|r| self.responses.get(r))
    }
}

#[cfg(test)]
#[path = "turn_recovery_tests.rs"]
mod turn_recovery_tests;
