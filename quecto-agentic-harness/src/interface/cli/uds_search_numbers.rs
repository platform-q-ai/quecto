//! The lenient numbers of `search_session_metadata` (R1-H5). `limit` and
//! `generation` are decoded as any JSON value: a number is brought into
//! range — negative is 0, a fraction is truncated, anything beyond `u64` is
//! `u64::MAX` — and an absent or `null` field is absent. Anything else is
//! named, so the edge can refuse it in a CORRELATED answer: a strict decode
//! failure is an uncorrelated `parse_error`, and a client awaiting its id
//! would wait for ever.

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

/// The number in `value`: `Ok(None)` when absent, `Err(())` when not a number.
fn lenient_u64(value: &serde_json::Value) -> Result<Option<u64>, ()> {
    match value {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::Number(number) => Ok(Some(match number.as_u64() {
            Some(exact) => exact,
            // Saturating: negative and NaN are 0, too large is `u64::MAX`.
            None => number.as_f64().map_or(0, |float| float as u64),
        })),
        _ => Err(()),
    }
}

/// The typed request the wire fields stand for, and — when `generation` or
/// `limit` was not a number — why it is refused instead of searched. The
/// request still carries everything that could be read, for the echo.
pub(super) fn request_of(
    fields: SearchFields,
) -> (SearchSessionMetadataRequest, Option<QueryRefusal>) {
    let (generation, limit) = (lenient_u64(&fields.generation), lenient_u64(&fields.limit));
    let malformed = match (&generation, &limit) {
        (Err(()), _) => Some("generation"),
        (_, Err(())) => Some("limit"),
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
        malformed.map(|field| QueryRefusal::NotANumber { field }),
    )
}

#[cfg(test)]
#[path = "uds_search_numbers_tests.rs"]
mod tests;
