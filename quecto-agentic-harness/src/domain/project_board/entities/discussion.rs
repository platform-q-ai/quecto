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
/// The largest discussion file (the codec's cap, S13).
pub const MAX_DISCUSSION_FILE_BYTES: usize = 1024 * 1024;
/// The fixed bytes of one message row in its file. The codec writes a
/// row as `"| "`, its cells joined by `" | "`, then `" |"` and a newline;
/// the cells are the message number, its author as ``Name (`email`)``
/// (the email in a code span), its time and the line. Besides the line
/// and the author's name and email ([`DiscussionFields::text_bytes`]
/// counts those), a row takes `| ` (2), the number (at most 4 digits,
/// [`MAX_MESSAGES`]), ` | ` (3), `` (` `` and `` `) `` around the email (3
/// and 2), ` | ` (3), the time (20), ` | ` (3) and ` |` with its newline
/// (3): 43.
pub const ROW_OVERHEAD_BYTES: usize = 43;
/// Room in the file for what [`DiscussionFields::text_bytes`] does not
/// count: the kind marker, the fields table's rows other than its title
/// and starter (an id and a task of at most 64 bytes each, a state word,
/// two times, and their labels and pipes), the headings and the message
/// table's header. Together they are well under 1 KiB; 4 KiB keeps room
/// to spare.
pub const FILE_RESERVE_BYTES: usize = 4 * 1024;
/// The most bytes a discussion's text takes in its file, as
/// [`DiscussionFields::text_bytes`] counts them. With the reserve, every
/// discussion fits its file: `text_bytes + FILE_RESERVE_BYTES <=
/// 1_044_480 + 4_096 = 1_048_576 = MAX_DISCUSSION_FILE_BYTES`.
pub const MAX_DISCUSSION_TEXT_BYTES: usize = MAX_DISCUSSION_FILE_BYTES - FILE_RESERVE_BYTES;

/// Why a message line is refused for a space other than U+0020: no file
/// holds one. Placeholder wording, pending the owner (#2481 S7).
pub const PLAIN_SPACES_ONLY: &str = "may space its words with plain spaces (U+0020) only";

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
    /// How many lines it holds: one row each in its file.
    pub fn lines(&self) -> usize {
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

/// The bytes `text` takes in a table cell, where each `|` is written `\|`.
fn escaped(text: &str) -> usize {
    text.len() + text.matches('|').count()
}

fn identity(who: &Identity) -> usize {
    escaped(&who.name) + escaped(&who.email)
}

impl DiscussionFields {
    /// The bytes the discussion's text takes in its file: its title, who
    /// started it, and each message line as its row writes it (the line,
    /// its author, and [`ROW_OVERHEAD_BYTES`]), every `|` counted twice as
    /// it is escaped. The `\n` between a message's lines is not written:
    /// each line is a row of its own.
    pub fn text_bytes(&self) -> usize {
        let rows: usize = self
            .messages
            .iter()
            .flat_map(|message| {
                let author = identity(&message.author);
                message
                    .text
                    .split('\n')
                    .map(move |line| escaped(line) + author + ROW_OVERHEAD_BYTES)
            })
            .sum();
        escaped(&self.title) + identity(&self.started_by) + rows
    }

    /// The lines in all of its messages: its file's rows.
    pub fn lines(&self) -> usize {
        self.messages.iter().map(Message::lines).sum()
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
    // The cheap caps first: counts and sizes, before any text is read.
    match discussion.messages.len() {
        1..=MAX_MESSAGES => {}
        _ => {
            return Err(SchemaError::new(
                "messages",
                format!("must hold from 1 to {MAX_MESSAGES} messages"),
            ));
        }
    }
    let lines = discussion.lines();
    match lines <= MAX_DISCUSSION_LINES {
        true => {}
        false => {
            return Err(SchemaError::new(
                "messages",
                format!("its messages hold {lines} lines; the most is {MAX_DISCUSSION_LINES}"),
            ));
        }
    }
    let bytes = discussion.text_bytes();
    match bytes <= MAX_DISCUSSION_TEXT_BYTES {
        true => {}
        false => {
            return Err(SchemaError::new(
                "text",
                format!(
                    "the discussion's text takes {bytes} bytes; the most is {MAX_DISCUSSION_TEXT_BYTES}"
                ),
            ));
        }
    }
    line("title", &discussion.title, MAX_TITLE_CHARS)?;
    discussion.started_by.validate("started_by")?;
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
        message_shape(&field, message)?;
    }
    Ok(())
}

/// A message's own shape, `field` naming it: its author, and its text of
/// one to [`MAX_MESSAGE_LINES`] lines, each a single line with something
/// visible. A batch is judged by it before the room it takes.
///
/// # Errors
/// The first rule the message breaks.
pub fn message_shape(field: &str, message: &Message) -> Result<(), SchemaError> {
    message.author.validate(&format!("{field}/author"))?;
    text(field, message)
}

/// A message's text: one to [`MAX_MESSAGE_LINES`] lines, each a single
/// line with something visible, spaced with plain spaces only.
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
        let field = format!("{field}/text/{index}");
        line(&field, text, MAX_MESSAGE_LINE_CHARS)?;
        plain_spaced(&field, text)
    })
}

/// Every space in `text` is a plain one (U+0020): the spaces chat writes
/// between words are made plain where messages come in
/// (`board_text::message`), and no file holds any other.
fn plain_spaced(field: &str, text: &str) -> Result<(), SchemaError> {
    match text.chars().all(|c| c == ' ' || !c.is_whitespace()) {
        true => Ok(()),
        false => Err(SchemaError::new(field, PLAIN_SPACES_ONLY)),
    }
}

#[cfg(test)]
#[path = "discussion_tests.rs"]
mod tests;
