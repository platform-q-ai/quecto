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

use crate::domain::message::{Message, Role};

/// What an image stands for in a saved message's text (#2422).
pub const IMAGE_PLACEHOLDER: &str = "[image]";

/// An image attached to a user message. Its fields are private to this
/// module, so a block is made only here: from an admitted image (the normal
/// path) or by [`UserImageBlock::restore`], which re-admits what persistence
/// kept. It carries the admitted type and keeps it: no step after admission
/// can lose or forge it.
#[derive(Clone, PartialEq, Eq)]
pub struct UserImageBlock {
    mime: quecto_image::ImageMime,
    /// Strict standard base64.
    data: String,
}

/// The normal way to make a block: from an admitted image.
impl From<quecto_image::ImageAttachment> for UserImageBlock {
    fn from(image: quecto_image::ImageAttachment) -> Self {
        Self {
            mime: image.mime(),
            data: image.into_data(),
        }
    }
}

impl UserImageBlock {
    /// A block persistence kept (#2424 saves them): its type and base64
    /// are re-admitted by the strict rules, never trusted, so a corrupt or
    /// edited session file cannot carry an image admission would refuse.
    pub fn restore(
        mime: quecto_image::ImageMime,
        data: String,
    ) -> Result<Self, quecto_image::ImageRefusal> {
        Ok(Self { mime, data }) // red (#2422 review round 2): trusted, not re-admitted
    }

    /// The admitted type.
    pub fn mime(&self) -> quecto_image::ImageMime {
        self.mime
    }

    /// The admitted type as the wire spells it.
    pub fn mime_type(&self) -> &'static str {
        self.mime.as_str()
    }

    /// The image's strict standard base64.
    pub fn data(&self) -> &str {
        &self.data
    }

    /// A block whose `data` is not admitted, for tests of what is done
    /// with a block (serializers, markers) that need short, known data.
    /// Compiled only for tests: production blocks come from admission.
    #[cfg(any(test, feature = "test-support"))]
    pub fn unchecked_for_tests(mime: quecto_image::ImageMime, data: impl Into<String>) -> Self {
        Self {
            mime,
            data: data.into(),
        }
    }

    /// A 1x1 admitted image of `mime`, for tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn sample(mime: quecto_image::ImageMime) -> Self {
        let bytes = quecto_image::samples::sample(mime);
        quecto_image::ImageAttachment::from_bytes(mime, &bytes)
            .expect("a sample is admitted")
            .into()
    }
}

/// Never prints the base64.
impl std::fmt::Debug for UserImageBlock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UserImageBlock")
            .field("mime", &self.mime)
            .field("data_len", &self.data.len())
            .finish()
    }
}

impl Message {
    /// This user message, carrying `images`.
    pub fn with_user_images(mut self, images: Vec<UserImageBlock>) -> Self {
        assert!(
            images.is_empty() || self.role == Role::User,
            "only a user message carries images"
        );
        self.user_image_blocks = images;
        self.invalidate_token_cache();
        self
    }
}

/// The text `message` is saved with. An images-only message (no text, or
/// whitespace only) is saved as one [`IMAGE_PLACEHOLDER`] per image, so a
/// resumed session never replays an empty turn; until #2424 saves the
/// images themselves, the text is all a session keeps. Every other message
/// is saved as it is.
pub fn stored_text(message: &Message) -> Cow<'_, str> {
    match (
        message.content.is_empty(), // red (#2422 review round 2): whitespace counts as text
        message.user_image_blocks.len(),
    ) {
        (true, images @ 1..) => Cow::Owned(vec![IMAGE_PLACEHOLDER; images].join("\n")),
        (true, 0) | (false, _) => Cow::Borrowed(&message.content),
    }
}

#[cfg(test)]
#[path = "user_images_tests.rs"]
mod tests;
