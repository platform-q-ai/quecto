//! #2422: image attachments on a UDS prompt, at the entry point.

use cucumber::{then, when};

use super::*;

/// A prompt whose one image is declared `declared` and holds a real 1x1
/// image of type `holding` (the same type for a good attachment).
#[when(
    expr = "I send prompt {string} with id {string} and an image declared {string} holding a {string} image"
)]
fn when_send_prompt_with_image(
    world: &mut QuectoWorld,
    message: String,
    id: String,
    declared: String,
    holding: String,
) {
    let mime = quecto_image::ImageMime::parse_exact(&holding).expect("an admitted type");
    let data = quecto_image::encode(&quecto_image::samples::sample(mime));
    let cmd = serde_json::json!({
        "type": "prompt",
        "id": id,
        "message": message,
        "images": [{"mimeType": declared, "data": data}],
    });
    world.uds_commands.push(cmd.to_string());
}

#[then(expr = "the model should have been sent {int} request(s)")]
fn then_model_requests(world: &mut QuectoWorld, count: usize) {
    let bodies = crate::uds_steps::captured_anthropic_bodies(world);
    assert_eq!(bodies.len(), count, "requests: {bodies:#?}");
}

#[then(expr = "request {int} should carry the text {string} and a {string} image")]
fn then_request_carries_image(world: &mut QuectoWorld, index: usize, text: String, mime: String) {
    let bodies = crate::uds_steps::captured_anthropic_bodies(world);
    let body = &bodies[index - 1];
    let user = body["messages"]
        .as_array()
        .and_then(|messages| messages.iter().rev().find(|m| m["role"] == "user"))
        .unwrap_or_else(|| panic!("no user message in {body}"));
    let blocks = user["content"]
        .as_array()
        .unwrap_or_else(|| panic!("a content array: {user}"));
    assert!(
        blocks
            .iter()
            .any(|b| b["type"] == "text" && b["text"] == text),
        "{user}"
    );
    assert!(
        blocks
            .iter()
            .any(|b| b["type"] == "image" && b["source"]["media_type"] == mime),
        "{user}"
    );
}
