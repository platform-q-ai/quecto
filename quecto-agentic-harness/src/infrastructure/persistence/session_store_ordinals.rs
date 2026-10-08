//! The file store's ordinal mechanics (`domain::sessions::entities::session::assign_missing_ordinals`)
//! on what it reads back and writes; a fully numbered write is borrowed, never copied (#2218).
use crate::domain::conversation::value_objects::message::Message;
use crate::domain::sessions::entities::session::{self, Session};
use std::borrow::Cow;

pub(super) fn with_assigned_ordinals(mut session: Session) -> Session {
    session::assign_missing_ordinals(&mut session.messages);
    session
}

pub(super) fn assign_missing_ordinals(mut messages: Vec<Message>) -> Vec<Message> {
    session::assign_missing_ordinals(&mut messages);
    messages
}

pub(super) fn messages_with_assigned_ordinals(messages: &[Message]) -> Cow<'_, [Message]> {
    let mut assigned = Cow::Borrowed(messages);
    if messages.iter().any(|m| m.ordinal.is_none()) {
        session::assign_missing_ordinals(assigned.to_mut());
    }
    assigned
}
