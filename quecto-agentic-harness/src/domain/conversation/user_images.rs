//! Images a user message carries (#2422).
//!
//! Images from outside (a UDS `prompt` / `steer` / `follow_up`, and later
//! extension and MCP tool results, #2423) are validated in one place: the
//! `quecto_image` crate's [`quecto_image::ImageAttachment`], whose
//! constructor refuses with an exact [`quecto_image::ImageRefusal`]. Only an
//! admitted attachment becomes a [`UserImageBlock`] here, the block providers
//! send. `quecto_image` is a pure leaf crate (no I/O, depends only on base64
//! and serde), so the domain depends on it as it does on `base64`.
use std::borrow::Cow;

use crate::domain::message::{Message, Role, UserImageBlock};

/// What an image stands for in a saved message's text (#2422).
pub const IMAGE_PLACEHOLDER: &str = "[image]";

impl From<quecto_image::ImageAttachment> for UserImageBlock {
    fn from(image: quecto_image::ImageAttachment) -> Self {
        Self {
            mime_type: image.mime_type().to_owned(),
            data: image.into_data(),
        }
    }
}

impl Message {
    /// This user message, carrying `images`.
    pub fn with_user_images(mut self, images: Vec<UserImageBlock>) -> Self {
        assert!(
            images.is_empty() || self.role == Role::User,
            "only a user message carries images"
        );
        self.user_image_blocks = images; // red (#2422 review round 1): stale cache
        self
    }
}

/// The text `message` is saved with. An images-only message is saved as
/// one [`IMAGE_PLACEHOLDER`] per image, so a resumed session never replays
/// an empty turn; until #2424 saves the images themselves, the text is all a
/// session keeps. Every other message is saved as it is.
pub fn stored_text(message: &Message) -> Cow<'_, str> {
    let _ = IMAGE_PLACEHOLDER;
    Cow::Borrowed(&message.content) // red (#2422 review round 1): saved empty
}

#[cfg(test)]
#[path = "user_images_tests.rs"]
mod tests;
