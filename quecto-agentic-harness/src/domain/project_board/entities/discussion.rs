//! A discussion (#2481's artifact model): a conversation about work, such
//! as one held in Pocket or a comment thread brought in from the host. It
//! may begin before there is a task and be linked to one later; its
//! parent, once linked, is that task. It is open until someone closes it.
//! Messages are kept in the order they were said, each with its author
//! and time; they are added in batches, never one write per message.
use super::super::value_objects::schema_error::{SchemaError, filled_markdown, line};
use super::super::value_objects::slug::Slug;
use super::super::value_objects::timestamp::Timestamp;
use super::super::value_objects::vocabulary::DiscussionState;
use super::claim::Identity;
use super::task::MAX_TITLE_CHARS;

/// The most messages a discussion holds.
pub const MAX_MESSAGES: usize = 1024;
/// The most characters in one message (the host's comment limit).
pub const MAX_MESSAGE_CHARS: usize = 65_536;
/// All of a discussion's text together, in UTF-8 bytes: its title, and
/// its messages with their authors. This budget keeps every discussion
/// inside the one file it is stored in, with room to spare (its codec,
/// M2, proves the fit).
pub const MAX_DISCUSSION_TEXT_BYTES: usize = 320 * 1024;

/// One message: who said what, and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub author: Identity,
    pub at: Timestamp,
    /// Markdown.
    pub text: String,
}

/// A discussion's content, unchecked: what [`Discussion::new`] validates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscussionFields {
    pub id: Slug,
    pub title: String,
    /// The task it is about, once linked.
    pub task: Option<Slug>,
    /// Oldest first; at least one, the message that began it.
    pub messages: Vec<Message>,
    /// Who closed it and when.
    pub closed: Option<(Identity, Timestamp)>,
    /// When its first message was said.
    pub created: Timestamp,
    pub updated: Timestamp,
}

impl DiscussionFields {
    /// The UTF-8 bytes of every free text in the discussion: its title,
    /// and its messages with their authors' names and addresses.
    pub fn text_bytes(&self) -> usize {
        let messages: usize = self
            .messages
            .iter()
            .map(|message| {
                message.text.len() + message.author.name.len() + message.author.email.len()
            })
            .sum();
        self.title.len() + messages
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

    pub fn state(&self) -> DiscussionState {
        match self.0.closed {
            None => DiscussionState::Open,
            Some(_) => DiscussionState::Closed,
        }
    }
}

fn validate(discussion: &DiscussionFields) -> Result<(), SchemaError> {
    match discussion.created <= discussion.updated {
        true => {}
        false => return Err(SchemaError::new("updated", "must not be before created")),
    }
    line("title", &discussion.title, MAX_TITLE_CHARS)?;
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
        filled_markdown(&format!("{field}/text"), &message.text, MAX_MESSAGE_CHARS)?;
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
    match &discussion.closed {
        None => {}
        Some((by, at)) => {
            by.validate("closed/by")?;
            match *last <= *at && *at <= discussion.updated {
                true => {}
                false => {
                    return Err(SchemaError::new(
                        "closed/at",
                        "must be after the last message, by the discussion's last change",
                    ));
                }
            }
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

#[cfg(test)]
#[path = "discussion_tests.rs"]
mod tests;
