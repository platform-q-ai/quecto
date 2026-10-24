//! A task's description as `create` and `update_draft` take it (#2705
//! T2): as its text (`description`), or as its sections (`goal`, `why`,
//! `done_when`, `links`) composed into that text, never both. The use
//! case normalises the text and the task's entry rules check it, so both
//! ways in are refused alike, with the section's field.
//!
//! How a request's title and description enter is its front end's policy
//! ([`entry_policy`]), resolved by whether the request gives the sections.
use super::{Read, RequestError};
use crate::domain::project_board::entities::task_parts::{Description, Entry, EntryPolicy};
use crate::domain::project_board::value_objects::telemetry::{FaultKind, FrontEnd};

/// Why a request giving the description both ways is refused.
const BOTH: &str = "give the description as text or as its sections (goal, why, done_when, \
     links), not both";

/// The entries each front end lets in (#2705 T2), an allowlist: the agent
/// tool writes sections only; the CLI, which the Pocket relay writes free
/// text and long titles through until its task page sends sections, also
/// takes legacy text as the board took it before.
pub fn entry_policy(front_end: FrontEnd) -> EntryPolicy {
    match front_end {
        FrontEnd::Tool => EntryPolicy::Sectioned,
        FrontEnd::Cli => EntryPolicy::SectionedOrLegacy,
    }
}

/// The description the request gives, when it gives one, and how its
/// title and description enter: a section it leaves out is left out of
/// the text (`update_draft` replaces the whole description, so give every
/// section it keeps).
///
/// # Errors
/// A section of the wrong type, or the description given both ways.
pub(super) fn description(read: &Read<'_>) -> Result<(Option<String>, Entry), RequestError> {
    let text = read.text("description")?;
    let goal = read.text("goal")?;
    let why = read.text("why")?;
    let done_when = lines(read, "done_when")?;
    let links = lines(read, "links")?;
    let sectioned = goal.is_some() || why.is_some() || done_when.is_some() || links.is_some();
    let entry = read.policy.entry(sectioned);
    let described = match (text, sectioned) {
        (text, false) => Ok(text),
        (None, true) => Description::compose(
            goal.as_deref(),
            why.as_deref(),
            done_when.as_deref(),
            links.as_deref(),
        )
        .map(Some)
        // Each section is checked on its own field before it is composed.
        .map_err(|error| RequestError::of(&error.field, error.problem, FaultKind::InvalidValue)),
        (Some(_), true) => Err(RequestError::of(
            "description",
            BOTH,
            FaultKind::InvalidValue,
        )),
    };
    Ok((described?, entry))
}

/// A list of lines, when given, each as sent.
fn lines(read: &Read<'_>, name: &str) -> Result<Option<Vec<String>>, RequestError> {
    let Some(value) = read.value(name) else {
        return Ok(None);
    };
    let items = value
        .as_array()
        .ok_or_else(|| RequestError::of(name, "must be a list of lines", FaultKind::WrongType))?;
    let line = |(index, item): (usize, &serde_json::Value)| {
        item.as_str().map(str::to_owned).ok_or_else(|| {
            RequestError::of(
                &format!("{name}/{index}"),
                "must be text",
                FaultKind::WrongType,
            )
        })
    };
    items
        .iter()
        .enumerate()
        .map(line)
        .collect::<Result<_, _>>()
        .map(Some)
}
