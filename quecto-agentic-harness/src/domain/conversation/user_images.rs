//! Images a user message carries (#2422).
//!
//! Images from outside (a UDS `prompt` / `steer` / `follow_up`, and later
//! extension and MCP tool results, #2423) are validated in one place: the
//! `quecto_image` crate's [`quecto_image::ImageAttachment`], whose
//! constructor refuses with an exact [`quecto_image::ImageRefusal`]. Only an
//! admitted attachment becomes a [`UserImageBlock`] here, the block providers
//! send. `quecto_image` is a pure leaf crate (no I/O, depends only on base64
//! and serde), so the domain depends on it as it does on `base64`.
use super::stored_images::{ImageDigest, VerifiedText, sha256_hex};
use crate::domain::message::{Message, Role};

/// An image attached to a user message. Its fields are private to this
/// module, so a block is made only here: from an admitted image (the normal
/// path) or by [`UserImageBlock::restore`], which re-admits what persistence
/// kept. It carries the admitted type and keeps it: no step after admission
/// can lose or forge it.
#[derive(Clone)]
pub struct UserImageBlock {
    mime: quecto_image::ImageMime,
    /// Strict standard base64.
    data: String,
    /// The SHA-256 of `data`, once asked for (#2424: the sidecar it is saved as).
    digest: ImageDigest,
}

/// Blocks are equal when their images are: the cached digest is not part of it.
impl PartialEq for UserImageBlock {
    fn eq(&self, other: &Self) -> bool {
        (self.mime, &self.data) == (other.mime, &other.data)
    }
}

impl Eq for UserImageBlock {}

/// The normal way to make a block: from an admitted image.
impl From<quecto_image::ImageAttachment> for UserImageBlock {
    fn from(image: quecto_image::ImageAttachment) -> Self {
        Self {
            mime: image.mime(),
            data: image.into_data(),
            digest: ImageDigest::default(),
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
        let payload = quecto_image::ImagePayload::new(mime.as_str(), data);
        quecto_image::ImageAttachment::new(payload).map(Self::from)
    }

    /// [`Self::restore`] for a sidecar's text, hashed when it was read
    /// (#2424): kept only when admission leaves it exactly as read, with the
    /// digest known rather than computed again. Admission hands the text back
    /// unchanged (the same allocation), so the check costs no copy; were it
    /// ever to rewrite it, the text is hashed once to tell.
    pub(crate) fn restore_verified(
        mime: quecto_image::ImageMime,
        text: VerifiedText,
    ) -> Option<Self> {
        let (sha256, data) = text.into_parts();
        let (at, len) = (data.as_ptr(), data.len());
        let block = Self::restore(mime, data).ok()?;
        let unchanged = std::ptr::eq(block.data.as_ptr(), at) && block.data.len() == len;
        let same = unchanged || sha256_hex(block.data.as_bytes()) == sha256;
        same.then(|| Self {
            digest: ImageDigest::verified(sha256),
            ..block
        })
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

    /// The SHA-256 of the image's base64, computed once (#2424).
    pub fn sha256(&self) -> &str {
        self.digest.of(&self.data)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn digest_builds_for_tests(&self) -> usize {
        self.digest.builds_for_tests()
    }

    /// A block whose `data` is not admitted, for tests of what is done
    /// with a block (serializers, markers) that need short, known data.
    /// Compiled only for tests: production blocks come from admission.
    #[cfg(any(test, feature = "test-support"))]
    pub fn unchecked_for_tests(mime: quecto_image::ImageMime, data: impl Into<String>) -> Self {
        Self {
            mime,
            data: data.into(),
            digest: ImageDigest::default(),
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

#[cfg(test)]
#[path = "user_images_tests.rs"]
mod tests;
