//! A discussion's file (`quecto-discussion`, epic #2481 S13). Its title is
//! the discussion's own. Its fields are Id, Task (when it is linked),
//! State, Started by, Created and Updated; its one section:
//!
//! - Messages `[Message | From | At | Text]`, one row per line of a
//!   message, oldest first: the message's number (from 1), author and time
//!   repeat on each of its rows, and its rows are contiguous.
//!
//! A row is written as `| `, its cells joined by ` | `, then ` |`: what the
//! domain's row overhead (`ROW_OVERHEAD_BYTES`) and file reserve
//! (`FILE_RESERVE_BYTES`) count, so every discussion the domain admits
//! fits this file (pinned byte for byte by its tests).
use super::cells::{self, column, refused};
use super::{ArtifactCodec, CodecError};
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::discussion::{
    Discussion, DiscussionFields, MAX_DISCUSSION_FILE_BYTES, Message,
};
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::DiscussionState;
use crate::infrastructure::persistence::project_board::markdown::{
    Body, BodyShape, CellKind, Column, Document, Limits, Section, SectionShape, Shape,
};

const ID: &str = "Id";
const TASK: &str = "Task";
const STATE: &str = "State";
const STARTED_BY: &str = "Started by";
const CREATED: &str = "Created";
const UPDATED: &str = "Updated";
const MESSAGES: &str = "Messages";

const FIELDS: &[Column<'static>] = &[
    column(ID, CellKind::Ascii),
    column(TASK, CellKind::Ascii),
    column(STATE, CellKind::Ascii),
    column(STARTED_BY, CellKind::Text),
    column(CREATED, CellKind::Ascii),
    column(UPDATED, CellKind::Ascii),
];

const SECTIONS: &[SectionShape<'static>] = &[SectionShape {
    heading: MESSAGES,
    body: BodyShape::Table(&[
        column("Message", CellKind::Ascii),
        column("From", CellKind::Text),
        column("At", CellKind::Ascii),
        column("Text", CellKind::Text),
    ]),
}];

/// A discussion's file caps: the domain's file cap, its 16,000 rows and
/// the rest of the file, and its longest row.
pub const DISCUSSION_LIMITS: Limits = Limits {
    file_bytes: MAX_DISCUSSION_FILE_BYTES,
    lines: 16_500,
    line_bytes: 8 * 1024,
};

pub struct DiscussionCodec;

impl ArtifactCodec for DiscussionCodec {
    type Value = Discussion;
    const SCHEMA: u32 = 1;
    const SHAPE: Shape<'static> = Shape {
        kind: "discussion",
        fields: FIELDS,
        sections: SECTIONS,
        limits: DISCUSSION_LIMITS,
    };

    fn to_document(discussion: &Discussion) -> Result<Document, CodecError> {
        let stub = true;
        if stub {
            return Err(CodecError::Schema);
        }
        let discussion = discussion.fields();
        let mut fields = vec![(ID, cells::slug(&discussion.id))];
        if let Some(task) = &discussion.task {
            fields.push((TASK, cells::slug(task)));
        }
        fields.extend([
            (STATE, cells::word(discussion.state.as_str())),
            (
                STARTED_BY,
                cells::identity(STARTED_BY, &discussion.started_by)?,
            ),
            (CREATED, cells::time(&discussion.created)),
            (UPDATED, cells::time(&discussion.updated)),
        ]);
        let mut rows = vec![];
        for (index, message) in discussion.messages.iter().enumerate() {
            let number = cells::number((index + 1) as u64);
            let from = cells::identity(MESSAGES, &message.author)?;
            let at = cells::time(&message.at);
            for line in message.text.split('\n') {
                rows.push(vec![
                    number.clone(),
                    from.clone(),
                    at.clone(),
                    line.to_string(),
                ]);
            }
        }
        Ok(Document {
            schema: Self::SCHEMA,
            title: discussion.title.clone(),
            fields: fields
                .into_iter()
                .map(|(name, cell)| (name.into(), cell))
                .collect(),
            sections: vec![Section {
                heading: MESSAGES.into(),
                body: Body::Table(rows),
            }],
        })
    }

    fn from_document(document: &Document) -> Result<Discussion, CodecError> {
        let stub = true;
        if stub {
            return Err(CodecError::Schema);
        }
        let fields = cells::Fields::of(document);
        let required = |name: &'static str| fields.required(name);
        let mut messages: Vec<Message> = vec![];
        for section in &document.sections {
            let Body::Table(rows) = &section.body else {
                return Err(refused("Discussion", "a table"));
            };
            for row in rows {
                let [number, from, at, line] = row.as_slice() else {
                    return Err(refused(MESSAGES, "a row of four cells"));
                };
                let number = cells::read_number(MESSAGES, number)?;
                let author = cells::read_identity(MESSAGES, from)?;
                let at = cells::read_time(MESSAGES, at)?;
                next_line(&mut messages, number, author, at, line)?;
            }
        }
        let fields = DiscussionFields {
            id: cells::read_slug(ID, required(ID)?)?,
            title: document.title.clone(),
            task: fields
                .get(TASK)
                .map(|cell| cells::read_slug(TASK, cell))
                .transpose()?,
            state: cells::read_word(STATE, required(STATE)?, DiscussionState::parse)?,
            started_by: cells::read_identity(STARTED_BY, required(STARTED_BY)?)?,
            messages,
            created: cells::read_time(CREATED, required(CREATED)?)?,
            updated: cells::read_time(UPDATED, required(UPDATED)?)?,
        };
        Discussion::new(fields)
            .map_err(|error| refused("Discussion", format!("{}: {}", error.field, error.problem)))
    }
}

/// `line`, the next row: another line of the last message when it repeats
/// that message's number, author and time, else the first of the next
/// message, numbered one past it. Anything else is refused: a message's
/// rows are contiguous and alike, and messages numbered from 1 in order.
fn next_line(
    messages: &mut Vec<Message>,
    number: u64,
    author: Identity,
    at: Timestamp,
    line: &str,
) -> Result<(), CodecError> {
    let count = messages.len() as u64;
    match messages.last_mut() {
        Some(last) if number == count => match last.author == author && last.at == at {
            true => {
                last.text.push('\n');
                last.text.push_str(line);
                Ok(())
            }
            false => Err(refused(
                MESSAGES,
                "a message's rows repeat its author and time",
            )),
        },
        _ if number == count + 1 => {
            messages.push(Message {
                author,
                at,
                text: line.to_string(),
            });
            Ok(())
        }
        _ => Err(refused(
            MESSAGES,
            "messages numbered from 1 in order, each message's rows together",
        )),
    }
}

#[cfg(test)]
#[path = "discussion_tests.rs"]
mod tests;
