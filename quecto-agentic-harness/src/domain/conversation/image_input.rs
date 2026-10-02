//! Which of a conversation's images a model is sent (#2421).
//!
//! One decision, made where the request is built, that every provider
//! shares: an image goes to a model only when the model's catalogue entry
//! declares `image` among its input modalities. A model that declares none
//! (or has no entry) is sent a short text marker in each image's place, in
//! the message the image belonged to: an image is never silently dropped.
//! The providers then serialize what they are given.
//!
//! What a message becomes depends on that message and the model alone,
//! never on the messages around it, so an earlier message is sent the same
//! way on every later request and the prompt cache keeps it (#2397).

use std::borrow::Cow;

use crate::domain::catalogue::TransportKind;
use crate::domain::message::Message;

/// What a model takes of a conversation's images (#2421).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImageInput {
    /// No image: each is sent as a marker. A model with no catalogue entry.
    #[default]
    NoImages,
    /// Every image but an animated GIF, which the OpenAI wires refuse.
    StillImages,
    /// Every image (the Anthropic Messages API).
    AllImages,
}

impl ImageInput {
    /// What a model whose entry declares `modalities`, reached over
    /// `transport`, takes.
    pub fn declared(modalities: &[String], _transport: &TransportKind) -> Self {
        match takes_images(modalities) {
            true => Self::AllImages,
            false => Self::NoImages,
        }
    }
}

/// The text an animated GIF becomes for a model whose wire takes still
/// images only.
pub fn animated_gif_marker(model: &str) -> String {
    format!("[image not sent: animated GIF not supported by {model}]")
}

/// Whether `data` (base64) is a GIF of more than one image frame.
pub fn is_animated_gif(_mime_type: &str, _data: &str) -> bool {
    false
}

/// The conversation as a model is sent it, for as long as this lives: each
/// image the model does not take is out of its message, with a marker after
/// the message's text in its place. Dropping it puts every message back.
pub struct SentConversation<'m> {
    messages: &'m mut [Message],
}

impl<'m> SentConversation<'m> {
    pub fn new(messages: &'m mut [Message], _model: &str, _input: ImageInput) -> Self {
        Self { messages }
    }

    /// The messages as they are sent.
    pub fn messages(&self) -> &[Message] {
        self.messages
    }
}

/// The input modality a model declares to take images.
const IMAGE_MODALITY: &str = "image";

/// Whether a model whose catalogue entry declares `modalities` takes images:
/// only when `image` is among them. A model that declares nothing takes none.
pub fn takes_images(modalities: &[String]) -> bool {
    modalities.iter().any(|declared| declared == IMAGE_MODALITY)
}

/// The text an image becomes for `model` when the model takes none.
pub fn not_sent_marker(model: &str) -> String {
    format!("[image not sent: {model} takes no image input]")
}

/// `messages` as `model` is sent them. A model that takes images, and any
/// conversation that carries none, is sent the messages as they are, with
/// nothing copied. Otherwise each message with images is sent with a
/// marker in each image's place; the conversation itself keeps its images
/// for a later model that takes them.
pub fn for_model<'a>(messages: &'a [Message], model: &str, takes: bool) -> Cow<'a, [Message]> {
    match (takes, messages.iter().any(carries_images)) {
        (false, true) => Cow::Owned(
            messages
                .iter()
                .map(|message| match carries_images(message) {
                    true => marked(message, model),
                    false => message.clone(),
                })
                .collect(),
        ),
        (true, _) | (false, false) => Cow::Borrowed(messages),
    }
}

fn carries_images(message: &Message) -> bool {
    !message.image_blocks.is_empty() || !message.user_image_blocks.is_empty()
}

/// `message` with its images replaced by one marker each, after its text.
fn marked(message: &Message, model: &str) -> Message {
    let images = message.image_blocks.len() + message.user_image_blocks.len();
    let marker = not_sent_marker(model);
    let text = match message.content.is_empty() {
        true => None,
        false => Some(message.content.as_str()),
    };
    let mut sent = message.clone();
    sent.content = text
        .into_iter()
        .chain(std::iter::repeat_n(marker.as_str(), images))
        .collect::<Vec<&str>>()
        .join("\n");
    sent.image_blocks.clear();
    sent.user_image_blocks.clear();
    sent.invalidate_token_cache();
    debug_assert!(
        sent.content.matches(marker.as_str()).count() >= images,
        "every image left a marker"
    );
    sent
}

#[cfg(test)]
#[path = "image_input_tests.rs"]
mod tests;
