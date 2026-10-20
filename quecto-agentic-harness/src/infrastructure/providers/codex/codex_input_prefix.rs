//! A Responses request's input as the cache diagnostic measures it (#2398):
//! its instructions, tools and model, then each item of its `input`, as the
//! transport's `input_prefix` compares and keeps them (#2410: each wire
//! measures its own requests).
use crate::domain::conversation::value_objects::image_tokens::{
    UNREADABLE_IMAGE_TOKENS, estimate_image_tokens,
};
use crate::domain::inference::events::request_observation::InputItemKind;
use crate::domain::inference::value_objects::token_estimate::{
    estimate_opaque_tokens, estimate_tokens,
};
use crate::infrastructure::providers::transport::input_prefix::{
    InputItem, MeasuredInput, digest, serialized,
};

/// Measure a Responses request `body` of `session` sent to `endpoint`: its
/// instructions, tools and model, then each item of its `input`.
pub(super) fn measured(session: &str, endpoint: &str, body: &serde_json::Value) -> MeasuredInput {
    MeasuredInput::of_items(session, endpoint, model(body), &head(body), items(body))
}

/// Measure another body of the request `measured` measured (resent without
/// replayed reasoning): the same session and endpoint.
pub(super) fn remeasured(measured: &MeasuredInput, body: &serde_json::Value) -> MeasuredInput {
    measured.with_items(model(body), &head(body), items(body))
}

fn model(body: &serde_json::Value) -> &str {
    body["model"].as_str().unwrap_or_default()
}

/// What precedes the input: the instructions and the tools.
fn head(body: &serde_json::Value) -> [&serde_json::Value; 2] {
    [&body["instructions"], &body["tools"]]
}

/// Each item of the body's `input`, measured.
fn items(body: &serde_json::Value) -> Vec<InputItem> {
    let mut bytes = Vec::new();
    let input = body["input"].as_array();
    debug_assert!(input.is_some(), "a Responses body carries an input list");
    input
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .map(|item| {
            let kind = kind(item);
            let text = serialized(item, &mut bytes);
            let digest = digest(text.as_bytes());
            // Encrypted reasoning is opaque: estimated at the plain ASCII
            // rate, not as dense high-entropy text. An image is estimated
            // from its pixel size (#2420), not as the text of its data URL
            // (#2421 review L5).
            let tokens = match (kind, image_tokens(item)) {
                (Some(InputItemKind::Reasoning), _) => estimate_opaque_tokens(text),
                (Some(_) | None, None) => estimate_tokens(text),
                (Some(_) | None, Some(images)) => {
                    estimate_tokens(serialized(&without_images(item), &mut bytes))
                        .saturating_add(images)
                }
            };
            InputItem {
                digest,
                tokens,
                kind,
            }
        })
        .collect()
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
/// URL it carries; one whose URL is not that, or whose type is off the
/// allowlist, costs the most an image can.
fn image_part_tokens(part: &serde_json::Value) -> usize {
    part["image_url"]
        .as_str()
        .and_then(|url| url.strip_prefix("data:"))
        .and_then(|url| url.split_once(";base64,"))
        .and_then(|(mime, data)| Some((quecto_image::ImageMime::parse_exact(mime)?, data)))
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

/// The kind of an input item the Responses API is sent; `None` for a shape
/// it is not one of.
pub(super) fn kind(item: &serde_json::Value) -> Option<InputItemKind> {
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
#[path = "codex_input_prefix_tests.rs"]
mod tests;
