//! What a model's reply to a conversation must hold (#2434).
//!
//! A reply to something new — a prompt, a steer, a follow-up, a harness
//! note or the loop's own feedback — must have output: an empty one is an
//! empty stream, retried and then an error. A reply to tool results may be
//! empty: the model has acted and has nothing to add (a background tool
//! that said "started, end your turn" is answered so), which ends the turn
//! as a final answer with no text.

use crate::domain::message::{Message, Role};

/// Whether the reply to a conversation must have output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyRequirement {
    /// The latest message is something to answer: output is required.
    Output,
    /// The latest message is a tool result: an empty reply ends the turn.
    MayBeEmpty,
}

impl ReplyRequirement {
    /// The requirement of the reply to `messages`: decided by the latest
    /// message the model is to answer, a user message or a tool result.
    /// System messages (the spill manifest) and the model's own are not
    /// something to answer, so they decide nothing.
    pub fn for_conversation(messages: &[Message]) -> Self {
        let latest = messages
            .iter()
            .rev()
            .find(|message| matches!(message.role, Role::User | Role::Tool));
        match latest.map(|message| &message.role) {
            Some(Role::Tool) => Self::MayBeEmpty,
            _ => Self::Output,
        }
    }
}

#[cfg(test)]
#[path = "reply_requirement_tests.rs"]
mod tests;
