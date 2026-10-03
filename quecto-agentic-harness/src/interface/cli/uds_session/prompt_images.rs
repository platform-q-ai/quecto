//! What a `prompt`, `steer` or `follow_up` puts in the conversation (#2403,
//! #2422): its text and the images admitted at dispatch, and what history
//! says about those images (how many, of which types, never their base64).
use crate::domain::message::{Message, UserImageBlock};
use crate::domain::turn_origin::{harness_note, instruction, prompt};

/// Whether `command` is one a user or a parent sends (#2403): what it
/// carries is a prompt. Anything else (a swarm wake) is the harness's.
fn is_sent_command(command: &str) -> bool {
    matches!(command, "prompt" | "steer" | "follow_up")
}

/// The message a `command` puts in the conversation as it runs at once
/// (#2403 review H1): a prompt for a command a user or a parent sends, a
/// harness note (of the phase it lands in) for any other, an idle wake.
/// Only a sent command carries images.
pub(crate) fn command_message(
    command: &str,
    body: PromptBody,
    conversation: &[Message],
) -> Message {
    let PromptBody { text, images } = body;
    match is_sent_command(command) {
        true => prompt(text).with_user_images(images),
        false => {
            assert!(images.is_empty(), "a {command} note carries no images");
            harness_note(text, conversation)
        }
    }
}

/// The message a queued control becomes when it is drained, with its
/// images: a prompt when a user or a parent sent it, else an instruction.
pub(super) fn queued_instruction(
    command: &str,
    content: String,
    images: Vec<UserImageBlock>,
) -> Message {
    match is_sent_command(command) {
        true => prompt(content),
        false => instruction(content),
    }
    .with_user_images(images)
}

/// What a `prompt`, `steer` or `follow_up` delivers (#2422): its text and
/// the images admitted at dispatch. It travels whole, into the pending
/// queue and out, so a queued instruction keeps its images.
#[derive(Debug, Clone, Default)]
pub(crate) struct PromptBody {
    pub(crate) text: String,
    pub(crate) images: Vec<UserImageBlock>,
}

impl From<String> for PromptBody {
    fn from(text: String) -> Self {
        Self {
            text,
            images: Vec::new(),
        }
    }
}

impl From<&str> for PromptBody {
    fn from(text: &str) -> Self {
        text.to_owned().into()
    }
}

/// The types of the images a user message carried, in order.
fn image_mime_types(msg: &Message) -> Vec<&str> {
    msg.user_image_blocks
        .iter()
        .map(|image| image.mime_type())
        .collect()
}

/// `imageCount` / `imageMimeTypes` on a message that carried images; a
/// text-only message's fields are unchanged.
pub(super) fn serialize_image_summary<S: serde::ser::SerializeStruct>(
    s: &mut S,
    msg: &Message,
) -> Result<(), S::Error> {
    if msg.user_image_blocks.is_empty() {
        return Ok(());
    }
    s.serialize_field("imageCount", &msg.user_image_blocks.len())?;
    s.serialize_field("imageMimeTypes", &image_mime_types(msg))
}

/// [`serialize_image_summary`] for a message already built as JSON.
pub(crate) fn add_image_summary(value: &mut serde_json::Value, msg: &Message) {
    if msg.user_image_blocks.is_empty() {
        return;
    }
    value["imageCount"] = serde_json::json!(msg.user_image_blocks.len());
    value["imageMimeTypes"] = serde_json::json!(image_mime_types(msg));
}
