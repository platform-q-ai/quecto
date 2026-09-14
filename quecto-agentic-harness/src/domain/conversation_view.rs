//! Read-side vocabulary of an ordered conversation (#1856, #1858): which of
//! its messages are the user-visible transcript, and how one message is
//! located by its stable id.
//!
//! Pure rules over [`Message`] values; the application's history and
//! recovery use cases and the interface's turn loop both consult them, so
//! neither re-implements the injected-prompt filter or the id lookup.
use super::ids::MessageId;
use super::message::{Message, Role};

/// Whether `message` is the system prompt the run injected at the head of
/// the conversation: a non-manifest system message whose content is
/// exactly `prompt`. An empty `prompt` injects nothing, so nothing matches.
pub fn is_injected_system_prompt(message: &Message, prompt: &str) -> bool {
    !prompt.is_empty()
        && message.role == Role::System
        && !message.is_manifest
        && message.content == prompt
}

/// The user-visible transcript: every message except the injected system
/// prompt, in conversation order.
pub fn user_visible_messages(messages: &[Message], injected_prompt: &str) -> Vec<Message> {
    messages
        .iter()
        .filter(|m| !is_injected_system_prompt(m, injected_prompt))
        .cloned()
        .collect()
}

/// Locate a message by its stable id (a stringified UUID). The id is parsed
/// once and compared as a typed UUID rather than stringifying every
/// candidate (#1061 review); an id that is not a UUID matches nothing.
pub fn position_by_id(messages: &[Message], message_id: &MessageId) -> Option<usize> {
    let target = uuid::Uuid::parse_str(message_id.as_str()).ok()?;
    messages.iter().position(|m| m.id() == target)
}

#[cfg(test)]
#[path = "conversation_view_tests.rs"]
mod tests;
