//! A Messages request's input as the cache diagnostic measures it (#2410,
//! extending #2398): what precedes the conversation (the system prompt and
//! the tools) and then each content block of each message, in order — the
//! units Anthropic's prompt cache works in. A block's cache marker is not
//! part of what it sends to the cache, so it is left out of the block's
//! digest: the markers move forward every request (#2410), and a moved
//! marker must not read as an edit. A string `content` is the one text block
//! it stands for, so a message whose last block gains a marker (and is sent
//! as a list) reads as unchanged.
use crate::application::providers::ports::ChatRequest;
use crate::domain::conversation::value_objects::image_tokens::{
    UNREADABLE_IMAGE_TOKENS, estimate_image_tokens,
};
use crate::domain::inference::events::request_observation::InputItemKind;
use crate::domain::inference::value_objects::provider::StreamEvent;
use crate::domain::inference::value_objects::token_estimate::{
    estimate_opaque_tokens, estimate_tokens,
};
use crate::infrastructure::providers::transport::input_prefix::{
    InputItem, MeasuredInput, PendingInput, digest, serialized,
};
use serde_json::Value;

/// The request's input measured against its session's baseline, for the
/// cache diagnostic (#2410, extending #2398): only an observed request of a
/// named session whose trace carries the baseline.
pub(super) fn begin(request: &ChatRequest<'_>, url: &str, body: &Value) -> Option<PendingInput> {
    match (&request.trace, request.session_id) {
        (Some(trace), Some(session)) => PendingInput::begin(trace, measured(session, url, body)),
        (None, _) | (_, None) => None,
    }
}

/// The sender a stream's events are sent to, and the task that forwards
/// them to `tx`: the first event that is not an error shows the send
/// accepted (#2398), so the measured body becomes the session's baseline
/// then. Without a measured body, `tx` itself.
pub(super) fn forward_accepting(
    tx: tokio::sync::mpsc::Sender<StreamEvent>,
    measured: Option<PendingInput>,
) -> (
    tokio::sync::mpsc::Sender<StreamEvent>,
    std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>,
) {
    let Some(measured) = measured else {
        return (tx, Box::pin(std::future::ready(())));
    };
    let (inner, mut events) = tokio::sync::mpsc::channel(64);
    let forward = async move {
        let mut measured = Some(measured);
        while let Some(event) = events.recv().await {
            let accepted = !matches!(event, StreamEvent::Error(_));
            accept(measured.take_if(|_| accepted), accepted);
            if tx.send(event).await.is_err() {
                break;
            }
        }
    };
    (inner, Box::pin(forward))
}

/// The provider accepted a send of the measured body (`accepted`): it
/// becomes the session's baseline; a refused or failed send never does.
pub(super) fn accept(measured: Option<PendingInput>, accepted: bool) {
    if let (Some(measured), true) = (measured, accepted) {
        measured.accept();
    }
}

/// Measure the Messages request `body` of `session` sent to `endpoint`.
pub(super) fn measured(session: &str, endpoint: &str, body: &Value) -> MeasuredInput {
    let system = without_markers(&body["system"]);
    let messages = body["messages"].as_array();
    debug_assert!(messages.is_some(), "a Messages body carries its messages");
    let items: Vec<InputItem> = messages
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .flat_map(|message| {
            let role = message["role"].as_str().unwrap_or_default();
            blocks(&message["content"])
                .into_iter()
                .map(move |block| item(role, block))
        })
        .collect();
    MeasuredInput::of_items(
        session,
        endpoint,
        body["model"].as_str().unwrap_or_default(),
        &[&system, &body["tools"]],
        items,
    )
}

/// The content blocks of a message's `content`: a string is one text block.
fn blocks(content: &Value) -> Vec<Value> {
    match content {
        Value::String(text) => vec![serde_json::json!({"type": "text", "text": text})],
        Value::Array(blocks) => blocks.iter().map(without_markers).collect(),
        _ => Vec::new(),
    }
}

/// `value` without the cache markers of its blocks (one block, or a list).
fn without_markers(value: &Value) -> Value {
    let mut value = value.clone();
    let strip = |block: &mut Value| {
        if let Some(block) = block.as_object_mut() {
            block.remove("cache_control");
        }
    };
    match &mut value {
        Value::Array(blocks) => blocks.iter_mut().for_each(strip),
        block => strip(block),
    }
    value
}

/// One content block of a `role` message, measured.
fn item(role: &str, block: Value) -> InputItem {
    let kind = kind(role, &block);
    let tokens = tokens(kind, &block);
    let mut bytes = Vec::new();
    let sent = serde_json::json!({"role": role, "block": block});
    InputItem {
        digest: digest(serialized(&sent, &mut bytes).as_bytes()),
        tokens,
        kind,
    }
}

/// The kind of a Messages content block: the user's text and images, the
/// assistant's text, a tool call, a tool result, or thinking; `None` for a
/// block that is none of these.
fn kind(role: &str, block: &Value) -> Option<InputItemKind> {
    match (role, block["type"].as_str()) {
        ("user", Some("text" | "image")) => Some(InputItemKind::User),
        ("user", Some("tool_result")) => Some(InputItemKind::FunctionCallOutput),
        ("assistant", Some("text")) => Some(InputItemKind::Assistant),
        ("assistant", Some("tool_use")) => Some(InputItemKind::FunctionCall),
        ("assistant", Some("thinking" | "redacted_thinking")) => Some(InputItemKind::Reasoning),
        _ => None,
    }
}

/// A block's token estimate: an image from its pixel size (#2420), also in
/// a tool result; a thinking block's signature and a redacted thinking
/// block as opaque text; anything else as the text it is sent as.
fn tokens(kind: Option<InputItemKind>, block: &Value) -> usize {
    match (kind, block["type"].as_str()) {
        (_, Some("image")) => image_tokens(block),
        (Some(InputItemKind::Reasoning), Some("redacted_thinking")) => {
            estimate_opaque_tokens(&block.to_string())
        }
        (Some(InputItemKind::Reasoning), _) => {
            let text = block["thinking"].as_str().unwrap_or_default();
            let signature = block["signature"].as_str().unwrap_or_default();
            estimate_tokens(text).saturating_add(estimate_opaque_tokens(signature))
        }
        (_, Some("tool_result")) => {
            let images: Vec<&Value> = block["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|part| part["type"] == "image")
                .collect();
            if images.is_empty() {
                return estimate_tokens(&block.to_string());
            }
            // What the result says beside its images (an allowlist of its
            // text parts), then each image from its pixel size.
            let mut text = block.clone();
            if let Some(parts) = text["content"].as_array_mut() {
                parts.retain(|part| part["type"] == "text");
            }
            images
                .into_iter()
                .map(image_tokens)
                .fold(estimate_tokens(&text.to_string()), usize::saturating_add)
        }
        _ => estimate_tokens(&block.to_string()),
    }
}

/// An `image` block's estimate, from the base64 source it carries; one whose
/// source is not that, or whose type is off the allowlist, costs the most an
/// image can.
fn image_tokens(block: &Value) -> usize {
    let source = &block["source"];
    match (
        source["type"].as_str(),
        source["media_type"]
            .as_str()
            .and_then(quecto_image::ImageMime::parse_exact),
        source["data"].as_str(),
    ) {
        (Some("base64"), Some(mime), Some(data)) => estimate_image_tokens(mime, data),
        _ => UNREADABLE_IMAGE_TOKENS,
    }
}

#[cfg(test)]
#[path = "anthropic_input_tests.rs"]
mod tests;
