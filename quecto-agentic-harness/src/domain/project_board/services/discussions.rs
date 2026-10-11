//! A discussion's rules (#2481's artifact model): anyone begins one, with
//! the messages said so far, about a task or none yet; adds messages to an
//! open one; links it to a task later; and closes it. Each rule returns
//! the discussion anew, so its schema holds after it.
//!
//! Messages are added in batches: a client such as Pocket gathers what is
//! said and the use case (M4) writes a discussion when it is linked or
//! closed, or a batch is due, never one write per message. Each message
//! keeps the time it was said; the discussion's `updated` is when it was
//! last written.
use super::super::entities::claim::Identity;
use super::super::entities::discussion::{Discussion, DiscussionFields, Message};
use super::super::entities::task::Task;
use super::super::value_objects::rule_error::BoardRuleError;
use super::super::value_objects::schema_error::SchemaError;
use super::super::value_objects::slug::Slug;
use super::super::value_objects::timestamp::Timestamp;
use super::super::value_objects::vocabulary::DiscussionState;
use super::claims::within_slack;
use super::plans::Moment;

/// What a discussion begins with: its id and title, the task it is about
/// if there is one yet, who began it, and the messages said so far,
/// oldest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opening<'a> {
    pub id: Slug,
    pub title: String,
    pub task: Option<&'a Task>,
    pub started_by: Identity,
    pub messages: Vec<Message>,
}

/// A discussion begun `when` with `opening`: created when its first
/// message was said, and written at `when`'s `now`, or at its last message
/// if that is later.
///
/// # Errors
/// No message (`invalid`); a message said past `when`'s latest time plus the stamp's
/// slack; or a discussion its schema refuses (a title or message out of
/// shape, messages out of order, past its limits).
pub fn begin(opening: Opening<'_>, when: &Moment) -> Result<Discussion, BoardRuleError> {
    not_ahead(&opening.id, &opening.messages, when)?;
    let (created, last) = match (opening.messages.first(), opening.messages.last()) {
        (Some(first), Some(last)) => (first.at.clone(), last.at.clone()),
        _ => {
            return Err(BoardRuleError::Invalid(SchemaError::new(
                "messages",
                "a discussion begins with a message",
            )));
        }
    };
    Discussion::new(DiscussionFields {
        id: opening.id,
        title: opening.title,
        task: opening.task.map(|task| task.fields().id.clone()),
        state: DiscussionState::Open,
        started_by: opening.started_by,
        messages: opening.messages,
        created,
        updated: when.now.clone().max(last),
    })
    .map_err(BoardRuleError::Invalid)
}

/// `discussion` with the batch `messages` added after its own, written
/// `when` (a second past its last change at the earliest, and no earlier
/// than the batch's last message).
///
/// # Errors
/// A discussion closed or stamped ahead; an empty batch; a message said
/// past `when`'s latest time plus the stamp's slack; or a discussion its
/// schema refuses (a message out of shape, or said before the last one).
pub fn append(
    discussion: &Discussion,
    messages: Vec<Message>,
    when: &Moment,
) -> Result<Discussion, BoardRuleError> {
    let fields = discussion.fields();
    open(discussion)?;
    built_on(discussion, when)?;
    not_ahead(&fields.id, &messages, when)?;
    let last = match messages.last() {
        Some(last) => last.at.clone(),
        None => return Err(BoardRuleError::NoEdit),
    };
    let mut next = fields.clone();
    next.updated = after(discussion, when)?.max(last);
    next.messages.extend(messages);
    Discussion::new(next).map_err(BoardRuleError::Invalid)
}

/// `discussion` linked to `task`, written `when`: the task it is about.
/// A closed discussion is linked too (a conversation filed afterwards).
///
/// # Errors
/// A discussion stamped ahead; one linked to that task already, or to
/// another.
pub fn link(
    discussion: &Discussion,
    task: &Task,
    when: &Moment,
) -> Result<Discussion, BoardRuleError> {
    let fields = discussion.fields();
    built_on(discussion, when)?;
    match &fields.task {
        None => {}
        Some(linked) if *linked == task.fields().id => return Err(BoardRuleError::NoEdit),
        Some(linked) => {
            return Err(BoardRuleError::DiscussionLinked {
                discussion: fields.id.clone(),
                task: linked.clone(),
            });
        }
    }
    let mut next = fields.clone();
    next.task = Some(task.fields().id.clone());
    next.updated = after(discussion, when)?;
    Discussion::new(next).map_err(BoardRuleError::Invalid)
}

/// `discussion` closed, written `when`.
///
/// # Errors
/// A discussion closed already, or stamped ahead.
pub fn close(discussion: &Discussion, when: &Moment) -> Result<Discussion, BoardRuleError> {
    open(discussion)?;
    built_on(discussion, when)?;
    let mut next = discussion.fields().clone();
    next.state = DiscussionState::Closed;
    next.updated = after(discussion, when)?;
    Discussion::new(next).map_err(BoardRuleError::Invalid)
}

/// `discussion` is open: messages are added to it, and it is closed once.
fn open(discussion: &Discussion) -> Result<(), BoardRuleError> {
    match discussion.fields().state {
        DiscussionState::Open => Ok(()),
        DiscussionState::Closed => Err(BoardRuleError::DiscussionClosed(
            discussion.fields().id.clone(),
        )),
    }
}

/// `discussion` is no later than `when` may build on.
fn built_on(discussion: &Discussion, when: &Moment) -> Result<(), BoardRuleError> {
    let fields = discussion.fields();
    match within_slack(&fields.updated, when.latest.as_ref()) {
        true => Ok(()),
        false => Err(BoardRuleError::DiscussionAhead {
            discussion: fields.id.clone(),
            updated: fields.updated.clone(),
        }),
    }
}

/// No message in `messages` was said past `when`'s latest time plus the
/// stamp's slack.
fn not_ahead(id: &Slug, messages: &[Message], when: &Moment) -> Result<(), BoardRuleError> {
    let ahead = messages
        .iter()
        .find(|message| !within_slack(&message.at, when.latest.as_ref()));
    match ahead {
        None => Ok(()),
        Some(message) => Err(BoardRuleError::DiscussionAhead {
            discussion: id.clone(),
            updated: message.at.clone(),
        }),
    }
}

/// `when`'s `now`, or a second past `discussion`'s last change if that is
/// later.
fn after(discussion: &Discussion, when: &Moment) -> Result<Timestamp, BoardRuleError> {
    let next = discussion
        .fields()
        .updated
        .plus_seconds(1)
        .map_err(|error| BoardRuleError::Invalid(error.at("updated")))?;
    Ok(when.now.clone().max(next))
}

#[cfg(test)]
#[path = "discussions_tests.rs"]
mod tests;
