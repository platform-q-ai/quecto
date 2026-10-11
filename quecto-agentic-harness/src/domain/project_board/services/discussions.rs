//! A discussion's rules (#2481's artifact model): anyone begins one, with
//! the messages said so far, about a task or none yet; adds messages to an
//! open one; links it to a task later; and closes it. Each rule returns
//! the discussion anew, so its schema holds after it.
//!
//! Messages are added in batches: a client such as Pocket gathers what is
//! said and the use case (M4) writes a discussion when it is linked or
//! closed, or a batch is due, never one write per message. Each message
//! keeps the time it was said; the discussion's `updated` is when it was
//! last written. A message's time comes from a client, so none is taken
//! later than the caller's time (or the outside time, when one bounds the
//! clock) plus the stamp's slack.
//!
//! What the use cases must bring (M4): `begin` does not know the board, so
//! the use case refuses an id a discussion already has before it begins
//! one.
use super::super::entities::claim::Identity;
use super::super::entities::discussion::{
    Discussion, DiscussionFields, MAX_DISCUSSION_LINES, MAX_DISCUSSION_TEXT_BYTES, MAX_MESSAGES,
    Message,
};
use super::super::entities::task::Task;
use super::super::value_objects::rule_error::BoardRuleError;
use super::super::value_objects::schema_error::SchemaError;
use super::super::value_objects::slug::Slug;
use super::super::value_objects::vocabulary::DiscussionState;
use super::claims::{after_last, within_slack};
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
/// No message (`invalid`); a message stamped ahead; or a discussion its
/// schema refuses (a title or message out of shape, messages out of
/// order, past its limits).
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
/// than the batch's last message). A refusal of one of the batch's
/// messages names it by its place in the batch (`batch/{i}/…`).
///
/// # Errors
/// A discussion closed or stamped ahead; an empty batch; a message stamped
/// ahead; a batch that would fill the discussion past its most messages,
/// lines or bytes; or a discussion its schema refuses (a message out of
/// shape, or said before the last one).
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
    let before = fields.messages.len();
    let batch = messages.len();
    let mut next = fields.clone();
    next.updated = after_last(&fields.updated, &when.now)?.max(last);
    next.messages.extend(messages);
    room(&next)?;
    assert_eq!(
        next.messages.len(),
        before + batch,
        "the batch, every message of it"
    );
    let added =
        Discussion::new(next).map_err(|error| BoardRuleError::Invalid(in_batch(error, before)))?;
    assert!(
        added.fields().updated > fields.updated,
        "a change moves updated forward"
    );
    Ok(added)
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
    next.updated = after_last(&fields.updated, &when.now)?;
    Discussion::new(next).map_err(BoardRuleError::Invalid)
}

/// `discussion` closed, written `when`.
///
/// # Errors
/// A discussion closed already, or stamped ahead.
pub fn close(discussion: &Discussion, when: &Moment) -> Result<Discussion, BoardRuleError> {
    open(discussion)?;
    built_on(discussion, when)?;
    let fields = discussion.fields();
    let mut next = fields.clone();
    next.state = DiscussionState::Closed;
    next.updated = after_last(&fields.updated, &when.now)?;
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

/// Every message in `messages` was said by `when`'s `now`, and by its
/// latest time when one bounds the clock, each plus the stamp's slack: a
/// message from a clock running far ahead would stop every later one.
fn not_ahead(id: &Slug, messages: &[Message], when: &Moment) -> Result<(), BoardRuleError> {
    let in_time = |message: &&Message| {
        within_slack(&message.at, Some(&when.now))
            && within_slack(&message.at, when.latest.as_ref())
    };
    let (_, ahead): (Vec<&Message>, Vec<&Message>) = messages.iter().partition(in_time);
    match ahead.first() {
        None => Ok(()),
        Some(message) => Err(BoardRuleError::MessageAhead {
            discussion: id.clone(),
            at: message.at.clone(),
        }),
    }
}

/// `next` has room for all its messages: at most [`MAX_MESSAGES`], their
/// lines at most [`MAX_DISCUSSION_LINES`], their text at most
/// [`MAX_DISCUSSION_TEXT_BYTES`].
fn room(next: &DiscussionFields) -> Result<(), BoardRuleError> {
    let (messages, lines, bytes) = (next.messages.len(), next.lines(), next.text_bytes());
    let reason = match (
        messages <= MAX_MESSAGES,
        lines <= MAX_DISCUSSION_LINES,
        bytes <= MAX_DISCUSSION_TEXT_BYTES,
    ) {
        (true, true, true) => return Ok(()),
        (false, _, _) => format!("{messages} messages; the most is {MAX_MESSAGES}"),
        (true, false, _) => format!("{lines} lines; the most is {MAX_DISCUSSION_LINES}"),
        (true, true, false) => {
            format!("{bytes} bytes of text; the most is {MAX_DISCUSSION_TEXT_BYTES}")
        }
    };
    Err(BoardRuleError::DiscussionFull {
        discussion: next.id.clone(),
        reason,
    })
}

/// `error`, a refusal of the discussion, named by the place in the batch
/// when it is one of the batch's messages (those from `before` on).
fn in_batch(error: SchemaError, before: usize) -> SchemaError {
    let rest = error.field.strip_prefix("messages/");
    let place = rest.and_then(|rest| {
        let (index, tail) = rest.split_once('/').unwrap_or((rest, ""));
        index.parse::<usize>().ok().map(|index| (index, tail))
    });
    match place {
        Some((index, tail)) if index >= before => {
            let field = match tail {
                "" => format!("batch/{}", index - before),
                tail => format!("batch/{}/{tail}", index - before),
            };
            SchemaError::new(field, error.problem)
        }
        _ => error,
    }
}

#[cfg(test)]
#[path = "discussions_tests.rs"]
mod tests;
