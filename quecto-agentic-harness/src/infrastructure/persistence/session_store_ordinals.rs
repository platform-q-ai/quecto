use crate::domain::message::Message;
use crate::domain::session::Session;

pub(super) fn with_assigned_ordinals(mut session: Session) -> Session {
    assign_missing_ordinals_in_place(&mut session.messages);
    session
}

pub(crate) fn assign_missing_ordinals_in_place(messages: &mut [Message]) {
    let mut next = messages
        .iter()
        .filter_map(|message| message.ordinal)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    for message in messages {
        if message.ordinal.is_none() {
            message.ordinal = Some(next);
            next = next.saturating_add(1);
        }
    }
}

pub(super) fn assign_missing_ordinals(mut messages: Vec<Message>) -> Vec<Message> {
    assign_missing_ordinals_in_place(&mut messages);
    messages
}

pub(super) fn messages_with_assigned_ordinals(messages: &[Message]) -> Vec<Message> {
    let mut assigned = messages.to_vec();
    assign_missing_ordinals_in_place(&mut assigned);
    assigned
}
