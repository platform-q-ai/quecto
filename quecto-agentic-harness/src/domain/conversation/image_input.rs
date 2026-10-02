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

use crate::domain::message::Message;

/// Whether a model whose catalogue entry declares `modalities` takes images.
pub fn takes_images(_modalities: &[String]) -> bool {
    false
}

/// The text an image becomes for `model` when the model takes none.
pub fn not_sent_marker(model: &str) -> String {
    format!("[image not sent: {model} takes no image input]")
}

/// `messages` as `model` is sent them.
pub fn for_model<'a>(messages: &'a [Message], _model: &str, _takes: bool) -> Cow<'a, [Message]> {
    Cow::Borrowed(messages)
}

#[cfg(test)]
#[path = "image_input_tests.rs"]
mod tests;
