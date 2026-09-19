//! The numbers of `search_session_metadata` (R1-H5, R2-H10), decoded as any
//! JSON value so that a bad one is a CORRELATED refusal, never an uncorrelated
//! `parse_error` a client would await for ever. `limit` is lenient: negative
//! is 0, a fraction is truncated, beyond `u64` is `u64::MAX`. `generation` is
//! exact — an integer `0..=u64::MAX` — so its echo is never a rounded value.

use crate::application::sessions::dto::{
    QueryGeneration, SearchLimit, SearchSessionMetadataRequest,
};
use crate::domain::session_metadata_search::QueryRefusal;
use crate::interface::cli::protocol::SessionListScopeCommand;

/// The wire fields of one search, as the protocol decoded them.
pub(in crate::interface::cli) struct SearchFields {
    pub(in crate::interface::cli) query: String,
    pub(in crate::interface::cli) scope: SessionListScopeCommand,
    pub(in crate::interface::cli) generation: serde_json::Value,
    pub(in crate::interface::cli) limit: serde_json::Value,
}

/// `Ok(None)`: absent. `Err(())`: no number — if `exact`, no integer in `u64`.
fn number(value: &serde_json::Value, exact: bool) -> Result<Option<u64>, ()> {
    match value {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::Number(number) => match (number.as_u64(), exact) {
            (Some(integer), _) => Ok(Some(integer)),
            (None, true) => Err(()),
            // Saturating: negative and NaN are 0, too large is `u64::MAX`.
            (None, false) => Ok(Some(number.as_f64().map_or(0, |float| float as u64))),
        },
        _ => Err(()),
    }
}

/// The typed request the wire fields stand for, and — when `generation` or
/// `limit` was malformed — why it is refused instead of searched. The
/// request still carries everything that could be read, for the echo.
pub(super) fn request_of(
    fields: SearchFields,
) -> (SearchSessionMetadataRequest, Option<QueryRefusal>) {
    let (generation, limit) = (&fields.generation, &fields.limit);
    let (generation, limit) = (number(generation, true), number(limit, false));
    let malformed = match (&generation, &limit) {
        (Err(()), _) => Some(("generation", "an integer from 0 to 18446744073709551615")),
        (_, Err(())) => Some(("limit", "a number")),
        _ => None,
    };
    let request = SearchSessionMetadataRequest {
        query: fields.query,
        scope: fields.scope.into(),
        generation: QueryGeneration(generation.unwrap_or_default().unwrap_or_default()),
        limit: SearchLimit::clamped(limit.unwrap_or_default()),
    };
    (
        request,
        malformed.map(|(field, expected)| QueryRefusal::Malformed { field, expected }),
    )
}

#[cfg(test)]
#[path = "uds_search_numbers_tests.rs"]
mod tests;
