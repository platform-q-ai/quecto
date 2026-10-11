//! A discussion (#2481's artifact model): a conversation about work, such
//! as one held in Pocket or a comment thread brought in from the host. It
//! may begin before there is a task and be linked to one later; its
//! parent, once linked, is that task. It is open until someone closes it.
//! Messages are kept in the order they were said, each with its author
//! and time; they are added in batches, never one write per message. A
//! message is one to [`MAX_MESSAGE_LINES`] lines, each a single line of
//! text (blank lines are dropped where messages come in): its file holds
//! one row per line.
use super::super::value_objects::schema_error::{SchemaError, line};
use super::super::value_objects::slug::Slug;
use super::super::value_objects::timestamp::Timestamp;
use super::super::value_objects::vocabulary::DiscussionState;
use super::claim::Identity;
use super::task::MAX_TITLE_CHARS;

/// The most messages a discussion holds.
pub const MAX_MESSAGES: usize = 1024;
/// The most lines in one message.
pub const MAX_MESSAGE_LINES: usize = 64;
/// The most characters in one line of a message.
pub const MAX_MESSAGE_LINE_CHARS: usize = 1_000;
/// The most lines in all of a discussion's messages together: one row
/// each in its file.
pub const MAX_DISCUSSION_LINES: usize = 16_000;
/// All of a discussion's text together, in UTF-8 bytes: its title, who
/// started it, and each message line with its author (written on every
/// row of the message). This budget keeps every discussion inside the one
/// file it is stored in (its codec, M2, proves the fit).
pub const MAX_DISCUSSION_TEXT_BYTES: usize = 320 * 1024;

/// One message: who said what, and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub author: Identity,
    pub at: Timestamp,
    /// One to [`MAX_MESSAGE_LINES`] lines joined by `\n`; inline
    /// markdown.
    pub text: String,
}

impl Message {
    fn lines(&self) -> usize {
        self.text.split('\n').count()
    }
}

/// A discussion's content, unchecked: what [`Discussion::new`] validates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscussionFields {
    pub id: Slug,
    pub title: String,
    /// The task it is about, once linked.
    pub task: Option<Slug>,
    pub state: DiscussionState,
    /// Who began it: who said its first message, or who filed it.
    pub started_by: Identity,
    /// Oldest first; at least one, the message that began it.
    pub messages: Vec<Message>,
    /// When its first message was said.
    pub created: Timestamp,
    pub updated: Timestamp,
}

impl DiscussionFields {
    /// The UTF-8 bytes of every free text in the discussion: its title,
    /// who started it, and its messages with their authors' names and
    /// addresses, once per line (as its file writes them).
    pub fn text_bytes(&self) -> usize {
        let identity = |who: &Identity| who.name.len() + who.email.len();
        let messages: usize = self
            .messages
            .iter()
            .map(|message| message.text.len() + identity(&message.author) * message.lines())
            .sum();
        self.title.len() + identity(&self.started_by) + messages
    }
}

/// A discussion whose fields keep every rule of its schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discussion(DiscussionFields);

impl Discussion {
    pub fn new(fields: DiscussionFields) -> Result<Self, SchemaError> {
        validate(&fields)?;
        Ok(Self(fields))
    }

    pub fn fields(&self) -> &DiscussionFields {
        &self.0
    }

    pub fn into_fields(self) -> DiscussionFields {
        self.0
    }
}

fn validate(discussion: &DiscussionFields) -> Result<(), SchemaError> {
    match discussion.created <= discussion.updated {
        true => {}
        false => return Err(SchemaError::new("updated", "must not be before created")),
    }
    line("title", &discussion.title, MAX_TITLE_CHARS)?;
    discussion.started_by.validate("started_by")?;
    match discussion.messages.len() {
        1..=MAX_MESSAGES => {}
        _ => {
            return Err(SchemaError::new(
                "messages",
                format!("must hold from 1 to {MAX_MESSAGES} messages"),
            ));
        }
    }
    // It began with its first message.
    match discussion.messages.first().map(|first| &first.at) {
        Some(at) if *at == discussion.created => {}
        _ => {
            return Err(SchemaError::new(
                "messages/0/at",
                "must be when the discussion was created",
            ));
        }
    }
    let mut last = &discussion.created;
    for (index, message) in discussion.messages.iter().enumerate() {
        let field = format!("messages/{index}");
        message.author.validate(&format!("{field}/author"))?;
        text(&field, message)?;
        // Messages are said in order, while the discussion exists.
        match *last <= message.at && message.at <= discussion.updated {
            true => last = &message.at,
            false => {
                return Err(SchemaError::new(
                    format!("{field}/at"),
                    "must follow the message before, by the discussion's last change",
                ));
            }
        }
    }
    let lines: usize = discussion.messages.iter().map(Message::lines).sum();
    match lines <= MAX_DISCUSSION_LINES {
        true => {}
        false => {
            return Err(SchemaError::new(
                "messages",
                format!("its messages hold {lines} lines; the most is {MAX_DISCUSSION_LINES}"),
            ));
        }
    }
    let text = discussion.text_bytes();
    match text <= MAX_DISCUSSION_TEXT_BYTES {
        true => Ok(()),
        false => Err(SchemaError::new(
            "text",
            format!(
                "the discussion's text is {text} bytes; the most is {MAX_DISCUSSION_TEXT_BYTES}"
            ),
        )),
    }
}

/// A message's text: one to [`MAX_MESSAGE_LINES`] lines, each a single
/// line with something visible.
fn text(field: &str, message: &Message) -> Result<(), SchemaError> {
    match message.lines() {
        1..=MAX_MESSAGE_LINES => {}
        _ => {
            return Err(SchemaError::new(
                format!("{field}/text"),
                format!("must be 1 to {MAX_MESSAGE_LINES} lines"),
            ));
        }
    }
    let mut lines = message.text.split('\n').enumerate();
    lines.try_for_each(|(index, text)| {
        line(
            &format!("{field}/text/{index}"),
            text,
            MAX_MESSAGE_LINE_CHARS,
        )
    })
}

#[cfg(test)]
#[path = "discussion_tests.rs"]
mod tests;
