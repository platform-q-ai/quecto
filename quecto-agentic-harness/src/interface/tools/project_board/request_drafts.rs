//! An edit of a draft in place, as a request names it, moved beside
//! `request.rs` as it grew past its room (#2705 B9).
use serde_json::Value;

use super::{Read, RequestError, description, slug};
use crate::application::project_board::dto::DraftUpdate;
use crate::application::project_board::operations::BoardRequest;
use crate::domain::project_board::value_objects::telemetry::FaultKind;

pub(super) fn update_draft(read: &Read<'_>) -> Result<BoardRequest, RequestError> {
    let parent = match read.value("parent") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(value) => Some(Some(slug("parent", value)?)),
    };
    let expected = read.pin()?;
    let (described, entry) = description::description(read)?;
    let update = DraftUpdate {
        id: read.task()?,
        title: read.text("title")?,
        description: described,
        parent,
        depends_on: read.slugs("depends_on")?,
        expected,
        entry,
    };
    let sets_something = update.title.is_some()
        || update.description.is_some()
        || update.parent.is_some()
        || update.depends_on.is_some();
    match sets_something {
        true => Ok(BoardRequest::UpdateDraft(update)),
        // No field to change was given.
        false => Err(RequestError::of(
            "op",
            "update_draft sets nothing: name a field to change",
            FaultKind::Missing,
        )),
    }
}
