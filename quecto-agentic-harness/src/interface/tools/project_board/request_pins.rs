//! The pin a request gives to what its caller read (#2705 B9):
//! `expected_rev`, the `rev` `show` answers, or the deprecated
//! `expected_updated`, never both.
use serde_json::Value;

use super::{Read, RequestError, misfit};
use crate::application::project_board::dto::ReadPin;
use crate::application::project_board::operations::{PlanPage, PlanView};
use crate::application::project_board::ports::BoardHead;
use crate::domain::project_board::value_objects::telemetry::FaultKind;

/// The field a pin by revision is given in (#2705 B9).
const EXPECTED_REV: &str = "expected_rev";
/// The deprecated field a pin by stamp is given in.
const EXPECTED_UPDATED: &str = "expected_updated";
/// The most bytes a rev takes: a git commit id (64 hex at most), or a
/// board server's `<epoch>:<revision>`, fits well within it.
const MAX_REV_BYTES: usize = 80;

/// A board revision as `show` answers it, `rev` (an allowlist): lowercase
/// letters, digits and `:`, at most [`MAX_REV_BYTES`].
fn rev(value: &Value) -> Result<BoardHead, RequestError> {
    let spelt = |text: &&str| {
        (1..=MAX_REV_BYTES).contains(&text.len())
            && text
                .bytes()
                .all(|b| b.is_ascii_digit() || b.is_ascii_lowercase() || b == b':')
    };
    value
        .as_str()
        .filter(spelt)
        .map(|text| BoardHead(text.to_string()))
        .ok_or_else(|| {
            RequestError::of(
                EXPECTED_REV,
                "must be the rev show gave you",
                misfit(value.is_string()),
            )
        })
}

/// What a `show` names of its plan: the version, the page, and the rev
/// its first page was read at, which a page alone takes (#2705 B9).
pub(super) fn shown(read: &Read<'_>) -> Result<PlanView, RequestError> {
    let page = PlanPage::paged(
        read.offset("approach_from")?,
        read.offset("handoff_from")?,
        read.offset("items_from")?,
    );
    let pin = read.value(EXPECTED_REV).map(rev).transpose()?;
    let page = match (pin, page) {
        (None, page) => page,
        (Some(rev), Some(page)) => Some(page.pinned(rev)),
        (Some(_), None) => {
            return Err(RequestError::of(
                EXPECTED_REV,
                "is taken by a page alone: give approach_from, handoff_from or items_from",
                FaultKind::InvalidValue,
            ));
        }
    };
    Ok(PlanView {
        version: read
            .value("version")
            .map(super::plans::version)
            .transpose()?,
        page,
    })
}

impl Read<'_> {
    /// The pin the request gives to what its caller read (#2705 B9):
    /// `expected_rev`, or the deprecated `expected_updated`, never both.
    pub(super) fn pin(&self) -> Result<Option<ReadPin>, RequestError> {
        let rev = self.value(EXPECTED_REV).map(rev).transpose()?;
        match (rev, self.timestamp(EXPECTED_UPDATED)?) {
            (Some(_), Some(_)) => Err(RequestError::of(
                EXPECTED_UPDATED,
                "is deprecated and not taken beside expected_rev: give expected_rev alone",
                FaultKind::InvalidValue,
            )),
            (Some(rev), None) => Ok(Some(ReadPin::Rev(rev))),
            (None, Some(updated)) => Ok(Some(ReadPin::Updated(updated))),
            (None, None) => Ok(None),
        }
    }
}
