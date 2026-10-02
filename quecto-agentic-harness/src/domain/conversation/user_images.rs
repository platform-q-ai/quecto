//! Images a user message carries (#2422).
//!
//! Images from outside (a UDS `prompt` / `steer` / `follow_up`, and later
//! extension and MCP tool results, #2423) are validated in one place: the
//! `quecto_image` crate's [`quecto_image::ImageAttachment`], whose
//! constructor refuses with an exact [`quecto_image::ImageRefusal`]. Only an
//! admitted attachment becomes a [`UserImageBlock`] here, the block providers
//! send.
use crate::domain::message::{Message, Role, UserImageBlock};

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
        self.user_image_blocks = images;
        self
    }
}

#[cfg(test)]
#[path = "user_images_tests.rs"]
mod tests;
