//! Images on the chat-completions wire (#2421): the content parts a user
//! message's images become, and the user message that carries a tool
//! batch's images.
//!
//! Which images a model is sent is the application's decision
//! (`domain::conversation::services::image_input`): these functions serialize every
//! image they are given. A message with no images is never touched, so a
//! text-only conversation goes exactly as before.

use std::collections::HashMap;

use super::provider_images::{DETAIL, data_url, images};
use crate::domain::conversation::value_objects::message::{Message, Role};

fn text_part(text: &str) -> serde_json::Value {
    serde_json::json!({"type": "text", "text": text})
}

fn image_part(mime_type: &str, data: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "image_url",
        "image_url": {"url": data_url(mime_type, data), "detail": DETAIL},
    })
}

/// A user message's content parts when it carries images: its text (when it
/// has any), then each image. `None` for a message with no images, whose
/// content stays a string.
pub(super) fn user_content(message: &Message) -> Option<serde_json::Value> {
    let images = images(message);
    let text = match message.content.is_empty() {
        true => None,
        false => Some(text_part(&message.content)),
    };
    match images.is_empty() {
        true => None,
        false => Some(serde_json::Value::Array(
            text.into_iter()
                .chain(images.iter().map(|(mime, data)| image_part(mime, data)))
                .collect(),
        )),
    }
}

/// `sent` (each message beside what it became) with each tool batch's
/// images after the batch: chat completions refuses an image in a `tool`
/// message, and a batch's results must follow their calls with nothing
/// between them, so one `user` message after the last result carries every
/// image of the batch, each labelled with the call it came from. What a
/// batch becomes depends on the batch alone.
pub(super) fn with_tool_images(sent: Vec<(&Message, serde_json::Value)>) -> Vec<serde_json::Value> {
    let mut wire = Vec::with_capacity(sent.len());
    let mut call_names: HashMap<&str, &str> = HashMap::new();
    let mut batch_images: Vec<serde_json::Value> = Vec::new();
    for (message, item) in sent {
        match message.role {
            Role::Tool => batch_images.extend(labelled_images(message, &call_names)),
            Role::System | Role::User | Role::Assistant => {
                wire.extend(images_message(std::mem::take(&mut batch_images)));
            }
        }
        for call in &message.tool_calls {
            call_names.insert(call.id.as_str(), call.name.as_str());
        }
        wire.push(item);
    }
    wire.extend(images_message(batch_images));
    wire
}

/// A tool result's images, each after the label naming its call and tool.
fn labelled_images(message: &Message, call_names: &HashMap<&str, &str>) -> Vec<serde_json::Value> {
    let id = message.tool_call_id.as_deref().unwrap_or_default();
    let name = call_names
        .get(id)
        .copied()
        .or(message.tool_name.as_deref())
        .unwrap_or("tool");
    let label = format!("Image from tool call {id} ({name}):");
    images(message)
        .into_iter()
        .flat_map(|(mime, data)| [text_part(&label), image_part(mime, data)])
        .collect()
}

/// The user message carrying a batch's image parts; none for a batch that
/// returned no image.
fn images_message(parts: Vec<serde_json::Value>) -> Option<serde_json::Value> {
    match parts.is_empty() {
        true => None,
        false => Some(serde_json::json!({"role": "user", "content": parts})),
    }
}
