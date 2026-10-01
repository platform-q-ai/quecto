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
#[derive(Debug)]
struct SessionDigests {
    /// The session's digest, not its key; `None` for requests naming none.
    session: Option<u64>,
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
        let session = session.map(|key| digest(key.as_bytes()));
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        let previous = match sessions.iter().position(|kept| kept.session == session) {
            Some(index) => sessions.remove(index).items,
            None => Vec::new(),
        };
        let mut items = Vec::with_capacity(input.len());
        let mut first_changed = None;
        let mut prefix_tokens_estimate = 0usize;
        let mut bytes = Vec::new();
        for (index, item) in input.iter().enumerate() {
            bytes.clear();
            serde_json::to_writer(&mut bytes, item).expect("a JSON value serializes");
            let item_digest = digest(&bytes);
            items.push(item_digest);
            let still_shared = first_changed.is_none() && index < previous.len();
            if still_shared {
                match previous[index] == item_digest {
                    true => {
                        let text = std::str::from_utf8(&bytes).expect("JSON is UTF-8");
                        prefix_tokens_estimate = prefix_tokens_estimate
                            .saturating_add(crate::domain::token_estimate::estimate_tokens(text));
                    }
                    false => first_changed = Some(index),
                }
            }
        }
        // The previous input ran past this one: the first item this one
        // lacks is where they diverge.
        let truncated = first_changed.is_none() && previous.len() > input.len();
        if truncated {
            first_changed = Some(input.len());
        }
        sessions.push(SessionDigests { session, items });
        let excess = sessions.len().saturating_sub(SESSIONS_RETAINED);
        sessions.drain(..excess);
        debug_assert!(sessions.len() <= SESSIONS_RETAINED);
        InputPrefix {
            input_items: input.len(),
            first_changed_item: first_changed,
            first_changed_kind: first_changed
                .and_then(|index| input.get(index))
                .and_then(kind),
            prefix_tokens_estimate,
        }
    }
}

/// A 64-bit digest of `bytes`, stable within the process.
fn digest(bytes: &[u8]) -> u64 {
    use std::hash::Hasher;
    let mut hasher = std::hash::DefaultHasher::new();
    hasher.write(bytes);
    hasher.finish()
}

/// The kind of an input item the Responses API is sent; `None` for a shape
/// it is not one of.
pub(crate) fn kind(item: &serde_json::Value) -> Option<InputItemKind> {
    let field = |name: &str| item.get(name).and_then(serde_json::Value::as_str);
    match (field("type"), field("role")) {
        (Some("function_call"), _) => Some(InputItemKind::FunctionCall),
        (Some("function_call_output"), _) => Some(InputItemKind::FunctionCallOutput),
        (Some("reasoning"), _) => Some(InputItemKind::Reasoning),
        (None | Some("message"), Some("user")) => Some(InputItemKind::User),
        (None | Some("message"), Some("assistant")) => Some(InputItemKind::Assistant),
        _ => None,
    }
}

#[cfg(test)]
#[path = "input_prefix_tests.rs"]
mod tests;
