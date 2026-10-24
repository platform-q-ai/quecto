//! A sectioned description's goal, why, outcomes and links as a board file holds
//! them (#2705 T2): checked as they enter, through the file format's own
//! port ([`BoardTextCells`]), so one the codec would refuse is refused on
//! its own field (`done_when/{i}`, `links/{i}`) instead of failing the
//! write. A legacy entry is checked as before T2: at write.
use crate::application::project_board::ports::BoardTextCells;
use crate::domain::project_board::entities::task_parts::{Description, Entry, PARTS};
use crate::domain::project_board::value_objects::schema_error::SchemaError;
use regex::Regex;
use std::sync::LazyLock;

/// Why an outcome or link the board's file cannot hold is refused.
const NOT_HELD: &str =
    "is text the board's file cannot hold: use plain words, `code` and [text](https://…) links";
/// Why a goal or why the file cannot hold is refused when it shows a bare
/// address, which a board file holds only as a link.
const BARE_ADDRESS: &str =
    "is text the board's file cannot hold: write the address as [text](https://…)";

/// A link's address or a code span: what may hold an address the file
/// keeps, left out when looking for a bare one.
static LINKED_OR_CODE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\]\([^)\s]*\)|`[^`]*`").expect("links and code"));

/// Why the file cannot hold `text`, a goal or why: with the address hint
/// when it shows a bare `http://` or `https://` address outside links and
/// code, else the general one.
fn not_held_prose(text: &str) -> &'static str {
    let outside = LINKED_OR_CODE.replace_all(text, "");
    match outside.contains("https://") || outside.contains("http://") {
        true => BARE_ADDRESS,
        false => NOT_HELD,
    }
}

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
    let [goal, why, done_when, links] = &PARTS;
    let prose = [
        (goal.field, Some(sectioned.goal())),
        (why.field, sectioned.why()),
    ];
    for (field, text) in prose {
        if let Some(text) = text.filter(|text| !cells.holds_prose(text)) {
            return Err(SchemaError::new(field, not_held_prose(text)));
        }
    }
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
