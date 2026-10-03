//! Images a user message carries (#2422).
//!
//! Images from outside (a UDS `prompt` / `steer` / `follow_up`, and
//! extension and MCP tool results, #2423) are validated in one place: the
//! `quecto_image` crate's [`quecto_image::ImageAttachment`], whose
//! constructor refuses with an exact [`quecto_image::ImageRefusal`]. Only an
//! admitted attachment becomes a [`UserImageBlock`], the block providers
//! send.
use crate::domain::message::{Message, Role};

/// An image attached to a user message: the same admitted block a tool
/// result carries (#2423), made only from an admitted image or by
/// [`crate::domain::tool::ImageBlock::restore`].
pub type UserImageBlock = crate::domain::tool::ImageBlock;

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

#[cfg(test)]
#[path = "user_images_tests.rs"]
mod tests;
