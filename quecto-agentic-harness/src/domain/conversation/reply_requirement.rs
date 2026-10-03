//! What a model's reply to a conversation must hold (#2434).
//!
//! A reply to something new — a prompt, a steer, a follow-up, a harness
//! note or the loop's own feedback — must have output: an empty one is an
//! empty stream, retried and then an error. A reply to tool results may be
//! empty: the model has acted and has nothing to add (a background tool
//! that said "started, end your turn" is answered so), which ends the turn
//! as a final answer with no text.

use crate::domain::message::Message;

/// Whether the reply to a conversation must have output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyRequirement {
    /// The latest message is something to answer: output is required.
    Output,
    /// The latest message is a tool result: an empty reply ends the turn.
    MayBeEmpty,
}

impl ReplyRequirement {
    /// The requirement of the reply to `messages`.
    pub fn for_conversation(_messages: &[Message]) -> Self {
        Self::Output
    }
}

#[cfg(test)]
#[path = "reply_requirement_tests.rs"]
mod tests;
