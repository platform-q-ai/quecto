//! Where a request's serialized input first differs from the previous
//! request of the same session (#2398): the cache diagnostic that tells
//! "the harness changed input item N in place" from "the provider missed
//! the cache".
//!
//! Each session keeps one 64-bit digest per input item it last sent, never
//! the items: what a request records is counts, an index and a kind.
use crate::domain::request_observation::{InputItemKind, InputPrefix};
use std::sync::Mutex;

/// The most sessions whose last input is kept; the least recently sent
/// is forgotten first.
pub(crate) const SESSIONS_RETAINED: usize = 32;

/// The per-session digests of the input each session last sent.
#[derive(Debug, Default)]
pub(crate) struct InputDigests {
    sessions: Mutex<Vec<SessionDigests>>,
}

/// One session's last input, as one digest per item.
#[expect(dead_code, reason = "red stub (#2398)")]
#[derive(Debug)]
struct SessionDigests {
    /// The session's digest, not its key.
    session: u64,
    items: Vec<u64>,
}

impl InputDigests {
    /// Compare `input`, the items a request of `session` sends, with the
    /// session's previous request, and keep its digests for the next.
    pub(crate) fn observe(
        &self,
        session: Option<&str>,
        input: &[serde_json::Value],
    ) -> InputPrefix {
        let _ = (session, &self.sessions);
        InputPrefix {
            input_items: input.len(),
            first_changed_item: Some(0),
            first_changed_kind: None,
            prefix_tokens_estimate: 0,
        }
    }
}

/// The kind of an input item the Responses API is sent; `None` for a shape
/// it is not one of.
pub(crate) fn kind(item: &serde_json::Value) -> Option<InputItemKind> {
    let _ = item;
    None
}

#[cfg(test)]
#[path = "input_prefix_tests.rs"]
mod tests;
