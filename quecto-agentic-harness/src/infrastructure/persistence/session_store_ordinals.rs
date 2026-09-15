//! Ordinal mechanics of the file store: the domain's ordinal policy
//! (`domain::session::assign_missing_ordinals`) applied to what the store
//! reads back and to the records it is about to write.
use crate::domain::message::Message;
use crate::domain::session::{
    Session, assign_missing_ordinals as assign_missing_ordinals_in_place,
};

pub(super) fn with_assigned_ordinals(mut session: Session) -> Session {
    assign_missing_ordinals_in_place(&mut session.messages);
    session
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
