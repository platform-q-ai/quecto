//! A sectioned description's outcomes and links as a board file holds
//! them (#2705 T2): checked as they enter, through the file format's own
//! port ([`BoardTextCells`]), so one the codec would refuse is refused on
//! its own field (`done_when/{i}`, `links/{i}`) instead of failing the
//! write. A legacy entry is checked as before T2: at write.
use crate::application::project_board::ports::BoardTextCells;
use crate::domain::project_board::entities::task_parts::{Description, Entry, PARTS};
use crate::domain::project_board::value_objects::schema_error::SchemaError;

/// Why an outcome or link the board's file cannot hold is refused.
const NOT_HELD: &str =
    "is text the board's file cannot hold: use plain words, `code` and [text](https://…) links";

/// Whether a board file holds each outcome and link of `description`,
/// entered as `entry`.
///
/// # Errors
/// The first outcome or link it cannot hold, by its field.
pub(super) fn held(
    description: &str,
    entry: Entry,
    cells: &dyn BoardTextCells,
) -> Result<(), SchemaError> {
    let sectioned = match entry {
        Entry::Sectioned => Description::read(description),
        Entry::Legacy => None,
    };
    let Some(sectioned) = sectioned else {
        return Ok(());
    };
    let [_, _, done_when, links] = &PARTS;
    let outcomes = sectioned.done_when().iter().map(|outcome| &outcome.text);
    let lines = [
        (done_when.field, outcomes.collect::<Vec<_>>()),
        (links.field, sectioned.links().iter().collect()),
    ];
    for (field, texts) in lines {
        if let Some(index) = texts.iter().position(|text| !cells.holds(text)) {
            return Err(SchemaError::new(format!("{field}/{index}"), NOT_HELD));
        }
    }
    Ok(())
}
