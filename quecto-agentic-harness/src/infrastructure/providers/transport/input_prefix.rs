//! Where a request's serialized input first differs from the previous
//! request of the same session (#2398): the cache diagnostic that tells
//! "the harness changed input item N in place" from "the provider missed
//! the cache". Each wire measures its own requests (#2410: what precedes
//! the input, then each input item); this is what every wire's measurement
//! is compared, kept and recorded by.
//!
//! A request is measured (one 64-bit digest and a token estimate per input
//! item, and for what precedes the input) before any lock is taken, and
//! compared with the session's baseline, which the session owns and each
//! request's trace carries. The request replaces that baseline only once
//! the provider accepts a send of it, so a refused or failed send never
//! becomes it. The baseline holds digests only, never the items or the
//! session key: what a request records is counts, indices, a kind and token
//! estimates.
use crate::domain::inference::events::request_observation::{
    InputBaseline, InputItemKind, InputPrefix, InputPrefixParts, RequestTrace,
};
use crate::domain::inference::value_objects::token_estimate::estimate_tokens;
use std::hash::Hasher;

/// A session's last accepted request, as its baseline keeps it.
#[derive(Debug)]
struct AcceptedInput {
    /// The session's digest, not its key.
    session: u64,
    /// The digest of what precedes the input: instructions or system
    /// prompt, tools, model and endpoint.
    head: u64,
    items: Vec<u64>,
}

/// One request's input measured, for comparing and then keeping.
#[derive(Debug)]
pub(in crate::infrastructure::providers) struct MeasuredInput {
    session: u64,
    endpoint: u64,
    head: u64,
    head_tokens: usize,
    items: Vec<InputItem>,
}

/// One input item as a wire measured it.
#[derive(Debug)]
pub(in crate::infrastructure::providers) struct InputItem {
    /// The digest of the item as it reaches the cache ([`digest`]): as
    /// sent, less any part that moves between requests without changing
    /// what is cached (a cache marker).
    pub digest: u64,
    /// Its token estimate.
    pub tokens: usize,
    pub kind: Option<InputItemKind>,
}

impl MeasuredInput {
    /// The request of `session` to `endpoint` for `model`: `head` is what
    /// precedes its input, as sent (each part counted when it is a string
    /// or a list), and `items` its input items, in order.
    pub(in crate::infrastructure::providers) fn of_items(
        session: &str,
        endpoint: &str,
        model: &str,
        head: &[&serde_json::Value],
        items: Vec<InputItem>,
    ) -> Self {
        Self::assemble(
            digest(session.as_bytes()),
            digest(endpoint.as_bytes()),
            model,
            head,
            items,
        )
    }

    /// Another body of the same request (resent without part of it): the
    /// same session and endpoint, measured anew.
    pub(in crate::infrastructure::providers) fn with_items(
        &self,
        model: &str,
        head: &[&serde_json::Value],
        items: Vec<InputItem>,
    ) -> Self {
        Self::assemble(self.session, self.endpoint, model, head, items)
    }

    fn assemble(
        session: u64,
        endpoint: u64,
        model: &str,
        head_parts: &[&serde_json::Value],
        items: Vec<InputItem>,
    ) -> Self {
        let mut bytes = Vec::new();
        // The model and endpoint select the cache: a change in either
        // leaves nothing cached, though neither is a token sent.
        let mut head = std::hash::DefaultHasher::new();
        head.write_u64(endpoint);
        head.write(model.as_bytes());
        let mut head_tokens = 0usize;
        for part in head_parts {
            let text = serialized(part, &mut bytes);
            head.write_u8(0);
            head.write(text.as_bytes());
            let sent = matches!(
                part,
                serde_json::Value::String(_) | serde_json::Value::Array(_)
            );
            if sent {
                head_tokens = head_tokens.saturating_add(estimate_tokens(text));
            }
        }
        Self {
            session,
            endpoint,
            head: head.finish(),
            head_tokens,
            items,
        }
    }
}

/// `value` serialized into `bytes`, as text.
pub(in crate::infrastructure::providers) fn serialized<'a>(
    value: &serde_json::Value,
    bytes: &'a mut Vec<u8>,
) -> &'a str {
    bytes.clear();
    let written = serde_json::to_writer(&mut *bytes, value);
    debug_assert!(written.is_ok(), "a JSON value serializes: {written:?}");
    let text = std::str::from_utf8(bytes);
    debug_assert!(text.is_ok(), "JSON is UTF-8");
    text.unwrap_or_default()
}

/// A 64-bit digest of `bytes`, stable within the process.
pub(in crate::infrastructure::providers) fn digest(bytes: &[u8]) -> u64 {
    let mut hasher = std::hash::DefaultHasher::new();
    hasher.write(bytes);
    hasher.finish()
}

/// How `measured` relates to the request `baseline` keeps; `None` (and a
/// debug assertion) should the counts ever disagree, so telemetry never
/// stops a request.
pub(in crate::infrastructure::providers) fn compare(
    baseline: &InputBaseline,
    measured: &MeasuredInput,
) -> Option<InputPrefix> {
    let items = &measured.items;
    let item_tokens = |count: usize| -> usize { items[..count].iter().map(|i| i.tokens).sum() };
    let request_tokens_estimate = measured
        .head_tokens
        .saturating_add(item_tokens(items.len()));
    let parts = baseline.read(|kept: Option<&AcceptedInput>| {
        match kept.filter(|kept| kept.session == measured.session) {
            Some(previous) => {
                let shared = previous.items.len().min(items.len());
                let diverged =
                    (0..shared).find(|&index| previous.items[index] != items[index].digest);
                // The previous input ran past this one: the first item this
                // one lacks is where they diverge.
                let truncated = (previous.items.len() > items.len()).then_some(items.len());
                let first_changed_item = diverged.or(truncated);
                let prefix_tokens_estimate =
                    item_tokens(first_changed_item.unwrap_or(previous.items.len()));
                let unchanged_prefix_tokens_estimate = match previous.head == measured.head {
                    true => measured.head_tokens.saturating_add(prefix_tokens_estimate),
                    false => 0,
                };
                InputPrefixParts {
                    input_items: items.len(),
                    previous_items: Some(previous.items.len()),
                    first_changed_item,
                    first_changed_kind: first_changed_item
                        .and_then(|index| items.get(index))
                        .and_then(|item| item.kind),
                    prefix_tokens_estimate,
                    unchanged_prefix_tokens_estimate,
                    request_tokens_estimate,
                }
            }
            None => InputPrefixParts {
                input_items: items.len(),
                previous_items: None,
                first_changed_item: None,
                first_changed_kind: None,
                prefix_tokens_estimate: 0,
                unchanged_prefix_tokens_estimate: 0,
                request_tokens_estimate,
            },
        }
    });
    let prefix = InputPrefix::new(parts);
    debug_assert!(prefix.is_ok(), "a comparison's counts agree: {prefix:?}");
    prefix.ok()
}

/// The provider accepted a send of `measured`: `baseline` keeps it now.
pub(in crate::infrastructure::providers) fn commit(
    baseline: &InputBaseline,
    measured: MeasuredInput,
) {
    baseline.keep(AcceptedInput {
        session: measured.session,
        head: measured.head,
        items: measured.items.iter().map(|item| item.digest).collect(),
    });
}

/// What `baseline` keeps, as its `Debug` shows it: for the wires' tests
/// that nothing kept carries content.
#[cfg(test)]
pub(in crate::infrastructure::providers) fn kept(baseline: &InputBaseline) -> Option<String> {
    baseline.read(|kept: Option<&AcceptedInput>| kept.map(|kept| format!("{kept:?}")))
}

/// A request measured against its session's baseline, kept as that
/// baseline once a send of it is accepted.
#[derive(Debug)]
pub(in crate::infrastructure::providers) struct PendingInput {
    baseline: InputBaseline,
    measured: MeasuredInput,
}

impl PendingInput {
    /// Compare `measured` with the baseline `trace` carries and record how
    /// they relate on `trace`; `None` when the trace carries no baseline.
    pub(in crate::infrastructure::providers) fn begin(
        trace: &RequestTrace,
        measured: MeasuredInput,
    ) -> Option<Self> {
        let baseline = trace.input_baseline()?;
        if let Some(prefix) = compare(&baseline, &measured) {
            trace.record_input_prefix(prefix);
        }
        Some(Self { baseline, measured })
    }

    /// The same request as another body sends it instead, measured by
    /// `remeasure` from this one.
    pub(in crate::infrastructure::providers) fn resent(
        self,
        remeasure: impl FnOnce(&MeasuredInput) -> MeasuredInput,
    ) -> Self {
        Self {
            measured: remeasure(&self.measured),
            baseline: self.baseline,
        }
    }

    /// A send of it was accepted.
    pub(in crate::infrastructure::providers) fn accept(self) {
        commit(&self.baseline, self.measured);
    }
}

#[cfg(test)]
#[path = "input_prefix_tests.rs"]
mod tests;
