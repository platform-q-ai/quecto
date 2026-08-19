use crate::domain::message::Message;

pub(super) fn assign_missing_ordinals(mut messages: Vec<Message>) -> Vec<Message> {
    let mut next = messages
        .iter()
        .filter_map(|message| message.ordinal)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    for message in &mut messages {
        if message.ordinal.is_none() {
            message.ordinal = Some(next);
            next = next.saturating_add(1);
        }
    }
    messages
}

pub(super) fn messages_with_assigned_ordinals(messages: &[Message]) -> Vec<Message> {
    assign_missing_ordinals(messages.to_vec())
}
