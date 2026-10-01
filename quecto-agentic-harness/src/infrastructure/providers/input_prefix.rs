//! Where a request's serialized input first differs from the previous
//! request of the same session (#2398): the cache diagnostic that tells
//! "the harness changed input item N in place" from "the provider missed
//! the cache".
//!
//! A request is measured (one 64-bit digest and a token estimate per input
//! item, and for its instructions and tools) before any lock is taken, and
//! compared with the session's last accepted request. Its digests replace
//! that baseline only once the provider accepts a send of it, so a refused
//! or failed send never becomes the baseline. Each session keeps digests
//! only, never the items, and never its key: what a request records is
//! counts, indices, a kind and token estimates.
use crate::domain::request_observation::{InputItemKind, InputPrefix, InputPrefixParts};
use crate::domain::token_estimate::estimate_tokens;
use std::sync::{Arc, LazyLock, Mutex};

/// The most sessions whose last input is kept; the least recently accepted
/// is forgotten first.
pub(crate) const SESSIONS_RETAINED: usize = 32;

/// The baselines every provider of this process compares with: a provider
/// rebuilt (an OAuth refresh, a recomposed runtime) keeps comparing with
/// what its predecessor sent, as the service's cache, keyed by session,
/// does.
static SHARED: LazyLock<Arc<InputDigests>> = LazyLock::new(Default::default);

/// The per-session digests of the input each session last had accepted.
#[derive(Debug, Default)]
pub(crate) struct InputDigests {
    sessions: Mutex<Vec<SessionDigests>>,
}

/// One session's last accepted request, as digests.
#[derive(Debug)]
#[expect(dead_code, reason = "red stub (#2398)")]
struct SessionDigests {
    /// The session's digest, not its key.
    session: u64,
    /// The instructions' and tools' digest.
    head: u64,
    items: Vec<u64>,
}

/// One request's input measured, for comparing and then keeping.
#[derive(Debug)]
pub(crate) struct MeasuredInput {
    session: u64,
    head: u64,
    head_tokens: usize,
    items: Vec<MeasuredItem>,
}

#[derive(Debug)]
struct MeasuredItem {
    digest: u64,
    tokens: usize,
    kind: Option<InputItemKind>,
}

impl MeasuredInput {
    /// Measure a Responses request `body` of `session`: its `instructions`
    /// and `tools`, then each item of its `input`.
    pub(crate) fn of(session: &str, body: &serde_json::Value) -> Self {
        Self::measure(digest(session.as_bytes()), body)
    }

    /// Measure another body of the same session (the request resent
    /// without replayed reasoning).
    pub(crate) fn for_body(&self, body: &serde_json::Value) -> Self {
        Self::measure(self.session, body)
    }

    fn measure(session: u64, body: &serde_json::Value) -> Self {
        let mut bytes = Vec::new();
        let mut serialized = |value: &serde_json::Value| {
            bytes.clear();
            serde_json::to_writer(&mut bytes, value).expect("a JSON value serializes");
            let text = std::str::from_utf8(&bytes).expect("JSON is UTF-8");
            (digest(&bytes), estimate_tokens(text))
        };
        let head = serde_json::json!([&body["instructions"], &body["tools"]]);
        let (head, head_tokens) = serialized(&head);
        let input = body["input"].as_array();
        debug_assert!(input.is_some(), "a Responses body carries an input list");
        let items = input
            .map_or(&[][..], Vec::as_slice)
            .iter()
            .map(|item| {
                let (digest, tokens) = serialized(item);
                MeasuredItem {
                    digest,
                    tokens,
                    kind: kind(item),
                }
            })
            .collect();
        Self {
            session,
            head,
            head_tokens,
            items,
        }
    }
}

impl InputDigests {
    /// The baselines every provider of this process shares.
    pub(crate) fn shared() -> Arc<Self> {
        let _ = &*SHARED;
        Arc::default()
    }

    /// How `measured` relates to its session's last accepted request.
    pub(crate) fn compare(&self, measured: &MeasuredInput) -> InputPrefix {
        let _ = (
            &self.sessions,
            measured.session,
            measured.head,
            measured.head_tokens,
        );
        let _ = measured
            .items
            .iter()
            .map(|i| (i.digest, i.tokens, i.kind))
            .count();
        InputPrefix::new(InputPrefixParts {
            input_items: measured.items.len(),
            previous_items: None,
            first_changed_item: None,
            first_changed_kind: None,
            prefix_tokens_estimate: 0,
            unchanged_prefix_tokens_estimate: 0,
            request_tokens_estimate: 0,
        })
        .expect("stub")
    }

    /// The provider accepted a send of `measured`: it is its session's
    /// baseline now, and the session the most recently used.
    pub(crate) fn commit(&self, measured: MeasuredInput) {
        let _ = measured;
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
