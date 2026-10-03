//! Where a request's serialized input first differs from the previous
//! request of the same session (#2398): the cache diagnostic that tells
//! "the harness changed input item N in place" from "the provider missed
//! the cache".
//!
//! A request is measured (one 64-bit digest and a token estimate per input
//! item, and for its instructions and tools) before any lock is taken, and
//! compared with the session's baseline, which the session owns and each
//! request's trace carries. The request replaces that baseline only once
//! the provider accepts a send of it, so a refused or failed send never
//! becomes it. The baseline holds digests only, never the items or the
//! session key: what a request records is counts, indices, a kind and token
//! estimates.
use crate::domain::conversation::image_tokens::{UNREADABLE_IMAGE_TOKENS, estimate_image_tokens};
use crate::domain::request_observation::{
    InputBaseline, InputItemKind, InputPrefix, InputPrefixParts, RequestTrace,
};
use crate::domain::token_estimate::{estimate_opaque_tokens, estimate_tokens};
use std::hash::Hasher;

/// A session's last accepted request, as its baseline keeps it.
#[derive(Debug)]
struct AcceptedInput {
    /// The session's digest, not its key.
    session: u64,
    /// The digest of what precedes the input: instructions, tools, model
    /// and endpoint.
    head: u64,
    items: Vec<u64>,
}

/// One request's input measured, for comparing and then keeping.
#[derive(Debug)]
pub(crate) struct MeasuredInput {
    session: u64,
    endpoint: u64,
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
    /// Measure a Responses request `body` of `session` sent to `endpoint`:
    /// its instructions, tools and model, then each item of its `input`.
    pub(crate) fn of(session: &str, endpoint: &str, body: &serde_json::Value) -> Self {
        Self::measure(
            digest(session.as_bytes()),
            digest(endpoint.as_bytes()),
            body,
        )
    }

    /// Measure another body of the same request (resent without replayed
    /// reasoning).
    pub(crate) fn for_body(&self, body: &serde_json::Value) -> Self {
        Self::measure(self.session, self.endpoint, body)
    }

    fn measure(session: u64, endpoint: u64, body: &serde_json::Value) -> Self {
        let mut bytes = Vec::new();
        // The model and endpoint select the cache: a change in either
        // leaves nothing cached, though neither is a token sent.
        let mut head = std::hash::DefaultHasher::new();
        head.write_u64(endpoint);
        head.write(body["model"].as_str().unwrap_or_default().as_bytes());
        let mut head_tokens = 0usize;
        for part in [&body["instructions"], &body["tools"]] {
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
        let input = body["input"].as_array();
        debug_assert!(input.is_some(), "a Responses body carries an input list");
        let items = input
            .map_or(&[][..], Vec::as_slice)
            .iter()
            .map(|item| {
                let kind = kind(item);
                let text = serialized(item, &mut bytes);
                let digest = digest(text.as_bytes());
                // Encrypted reasoning is opaque: estimated at the plain
                // ASCII rate, not as dense high-entropy text. An image is
                // estimated from its pixel size (#2420), not as the text of
                // its data URL (#2421 review L5).
                let tokens = match (kind, image_tokens(item)) {
                    (Some(InputItemKind::Reasoning), _) => estimate_opaque_tokens(text),
                    (Some(_) | None, None) => estimate_tokens(text),
                    (Some(_) | None, Some(images)) => {
                        estimate_tokens(serialized(&without_images(item), &mut bytes))
                            .saturating_add(images)
                    }
                };
                MeasuredItem {
                    digest,
                    tokens,
                    kind,
                }
            })
            .collect();
        Self {
            session,
            endpoint,
            head: head.finish(),
            head_tokens,
            items,
        }
    }
}

/// The fields of an input item that can hold `input_image` parts: a user
/// message's `content` and a tool result's `output` (#2421).
const IMAGE_FIELDS: [&str; 2] = ["content", "output"];

fn is_image(part: &serde_json::Value) -> bool {
    part["type"] == "input_image"
}

/// The estimate of the `input_image` parts `item` carries, each from its
/// pixel size; `None` for an item that carries none.
fn image_tokens(item: &serde_json::Value) -> Option<usize> {
    let images: Vec<&serde_json::Value> = IMAGE_FIELDS
        .iter()
        .filter_map(|field| item.get(field).and_then(serde_json::Value::as_array))
        .flatten()
        .filter(|part| is_image(part))
        .collect();
    match images.is_empty() {
        true => None,
        false => Some(images.into_iter().map(image_part_tokens).sum()),
    }
}

/// One `input_image` part's estimate, from the `data:<mime>;base64,<data>`
/// URL it carries; one whose URL is not that costs the most an image can.
fn image_part_tokens(part: &serde_json::Value) -> usize {
    part["image_url"]
        .as_str()
        .and_then(|url| url.strip_prefix("data:"))
        .and_then(|url| url.split_once(";base64,"))
        .map_or(UNREADABLE_IMAGE_TOKENS, |(mime, data)| {
            estimate_image_tokens(mime, data)
        })
}

/// `item` with only the `input_text` parts of its part lists: what it says
/// beside its images (an allowlist, #2421 round 2 nit 2).
fn without_images(item: &serde_json::Value) -> serde_json::Value {
    let mut text = item.clone();
    for field in IMAGE_FIELDS {
        if let Some(parts) = text
            .get_mut(field)
            .and_then(serde_json::Value::as_array_mut)
        {
            parts.retain(|part| part["type"] == "input_text");
        }
    }
    text
}

/// `value` serialized into `bytes`, as text.
fn serialized<'a>(value: &serde_json::Value, bytes: &'a mut Vec<u8>) -> &'a str {
    bytes.clear();
    let written = serde_json::to_writer(&mut *bytes, value);
    debug_assert!(written.is_ok(), "a JSON value serializes: {written:?}");
    let text = std::str::from_utf8(bytes);
    debug_assert!(text.is_ok(), "JSON is UTF-8");
    text.unwrap_or_default()
}

/// How `measured` relates to the request `baseline` keeps; `None` (and a
/// debug assertion) should the counts ever disagree, so telemetry never
/// stops a request.
pub(crate) fn compare(baseline: &InputBaseline, measured: &MeasuredInput) -> Option<InputPrefix> {
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
pub(crate) fn commit(baseline: &InputBaseline, measured: MeasuredInput) {
    baseline.keep(AcceptedInput {
        session: measured.session,
        head: measured.head,
        items: measured.items.iter().map(|item| item.digest).collect(),
    });
}

/// A request measured against its session's baseline, kept as that
/// baseline once a send of it is accepted.
#[derive(Debug)]
pub(crate) struct PendingInput {
    baseline: InputBaseline,
    measured: MeasuredInput,
}

impl PendingInput {
    /// Measure `body`, a request of `session` to `endpoint`, compare it with
    /// the baseline `trace` carries, and record how they relate on `trace`;
    /// `None` when the trace carries no baseline.
    pub(crate) fn begin(
        trace: &RequestTrace,
        session: &str,
        endpoint: &str,
        body: &serde_json::Value,
    ) -> Option<Self> {
        let baseline = trace.input_baseline()?;
        let measured = MeasuredInput::of(session, endpoint, body);
        if let Some(prefix) = compare(&baseline, &measured) {
            trace.record_input_prefix(prefix);
        }
        Some(Self { baseline, measured })
    }

    /// The same request as `body` sends it instead.
    pub(crate) fn for_body(self, body: &serde_json::Value) -> Self {
        Self {
            measured: self.measured.for_body(body),
            baseline: self.baseline,
        }
    }

    /// A send of it was accepted.
    pub(crate) fn accept(self) {
        commit(&self.baseline, self.measured);
    }
}

/// A 64-bit digest of `bytes`, stable within the process.
fn digest(bytes: &[u8]) -> u64 {
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
