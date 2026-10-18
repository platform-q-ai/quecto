//! The images a saved session keeps beside its transcript (#2424).
//!
//! Each image is stored once, as the exact base64 text the conversation
//! carries, named by the SHA-256 of that text; a message record holds only
//! [`ImageRef`]s, in the image's order. On load a reference becomes its block
//! again, the text verbatim, so a reloaded request is byte-identical to the
//! one before. A reference whose sidecar cannot be read is never dropped: it
//! stays on its message as an [`UnloadedImage`], is saved again with it, and
//! only a provider request shows it, as [`unavailable_marker`] (added where
//! the request is built, `image_input::SentConversation`); the next load may
//! bring it back. Every image, a user's or a tool result's, is re-admitted
//! by the strict rules on restore (`ImageBlock::restore`, #2423); one they
//! refuse stays unloaded too.
//! A message archived to session memory keeps its references there for
//! information only: a recall stays text and names them
//! ([`not_recalled_marker`]).
//!
//! Pure. What an image is (its types, its base64 and its size limit) is the
//! `quecto_image` crate's; the digest is this module's.
use std::sync::OnceLock;
#[cfg(any(test, feature = "test-support"))]
use std::sync::atomic::{AtomicUsize, Ordering};

use sha2::{Digest, Sha256};

use quecto_image::{ImageMime, MAX_ENCODED_LEN};

use crate::domain::conversation::value_objects::image_tokens::estimate_image_tokens;
use crate::domain::conversation::value_objects::message::Message;
use crate::domain::tool_policy::value_objects::tool::ImageBlock;

/// The hex digits of a digest a marker shows.
const MARKER_DIGITS: usize = 12;

/// The longest image text stored: the base64 of the largest image quecto
/// admits ([`quecto_image::MAX_ENCODED_LEN`]).
pub const MAX_STORED_IMAGE_TEXT: usize = MAX_ENCODED_LEN;

/// A stored image, as a message record names it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ImageRef {
    /// The lowercase hex SHA-256 of the image's base64 text: the sidecar's name.
    pub sha256: String,
    pub mime_type: String,
}

/// Which of a message's image lists an image belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageKind {
    /// A tool result's (`Message::image_blocks`).
    Tool,
    /// A user's (`Message::user_image_blocks`).
    User,
}

/// An image a message carries whose text is not in memory: its sidecar could
/// not be read (missing, unreadable, corrupt) or was not read (a transcript
/// only shown). Saved with its message; sent as [`unavailable_marker`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnloadedImage {
    pub kind: ImageKind,
    /// Its place among the message's images of its kind.
    pub position: usize,
    pub reference: ImageRef,
}

/// The SHA-256 of an image block's text, computed once (a save names every
/// image in the transcript, so it must not hash them all each time). Kept by
/// a clone, whose text is the same; a block's text never changes (its
/// fields are private, [`ImageBlock`]).
#[derive(Debug, Default)]
pub struct ImageDigest {
    digest: OnceLock<String>,
    #[cfg(any(test, feature = "test-support"))]
    builds: AtomicUsize,
}

impl Clone for ImageDigest {
    fn clone(&self) -> Self {
        Self {
            digest: self.digest.clone(),
            #[cfg(any(test, feature = "test-support"))]
            builds: AtomicUsize::new(0),
        }
    }
}

impl ImageDigest {
    /// The digest of a [`VerifiedText`], already known.
    pub(crate) fn verified(sha256: String) -> Self {
        let verified = Self::default();
        assert!(verified.digest.set(sha256).is_ok(), "a new digest is unset");
        verified
    }

    /// The digest of `text`, the block's own text.
    pub fn of(&self, text: &str) -> &str {
        self.digest.get_or_init(|| {
            #[cfg(any(test, feature = "test-support"))]
            self.builds.fetch_add(1, Ordering::Relaxed);
            sha256_hex(text.as_bytes())
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn builds_for_tests(&self) -> usize {
        self.builds.load(Ordering::Relaxed)
    }
}

/// An image's text with the SHA-256 it was hashed to: made only by hashing
/// ([`VerifiedText::of`]), so its digest is always its text's. A sidecar read
/// returns one, and a restored block takes its digest as known.
#[derive(Clone, PartialEq, Eq)]
pub struct VerifiedText {
    sha256: String,
    text: String,
}

impl VerifiedText {
    /// `text`, hashed.
    pub fn of(text: String) -> Self {
        let sha256 = sha256_hex(text.as_bytes());
        Self { sha256, text }
    }

    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// The digest and the text.
    pub(crate) fn into_parts(self) -> (String, String) {
        (self.sha256, self.text)
    }
}

/// Never prints the text.
impl std::fmt::Debug for VerifiedText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VerifiedText")
            .field("sha256", &self.sha256)
            .field("len", &self.text.len())
            .finish()
    }
}

/// The references of the images one message carries, by kind, each in its
/// order: the loaded images and the unloaded ones in their places.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MessageImageRefs {
    pub tool: Vec<ImageRef>,
    pub user: Vec<ImageRef>,
}

impl MessageImageRefs {
    /// No images: what a text-only message has.
    pub const NONE: Self = Self {
        tool: Vec::new(),
        user: Vec::new(),
    };

    /// The references of `message`'s images. Free for a text-only message;
    /// each image is hashed once in its life ([`ImageDigest`]).
    pub fn of(message: &Message) -> Self {
        let tool = message
            .image_blocks
            .iter()
            .map(|block| reference(block.sha256(), block.mime_type()));
        let user = message
            .user_image_blocks
            .iter()
            .map(|block| reference(block.sha256(), block.mime_type()));
        let (unloaded, by_reference) = (&message.unloaded_images, |gap: &UnloadedImage| {
            gap.reference.clone()
        });
        Self {
            tool: in_place(tool.collect(), unloaded, ImageKind::Tool, by_reference),
            user: in_place(user.collect(), unloaded, ImageKind::User, by_reference),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.tool.is_empty() && self.user.is_empty()
    }

    /// Every reference, tool results' first.
    pub fn all(&self) -> impl Iterator<Item = &ImageRef> {
        self.tool.iter().chain(&self.user)
    }

    pub fn into_all(self) -> Vec<ImageRef> {
        self.tool.into_iter().chain(self.user).collect()
    }

    /// Every reference as an unloaded image in its place: what a message
    /// read only to be shown carries, its images named but not read.
    pub fn into_unloaded(self) -> Vec<UnloadedImage> {
        let unloaded = |kind| {
            move |(position, reference)| UnloadedImage {
                kind,
                position,
                reference,
            }
        };
        let tool = self
            .tool
            .into_iter()
            .enumerate()
            .map(unloaded(ImageKind::Tool));
        let user = self
            .user
            .into_iter()
            .enumerate()
            .map(unloaded(ImageKind::User));
        tool.chain(user).collect()
    }
}

fn reference(sha256: &str, mime_type: &str) -> ImageRef {
    ImageRef {
        sha256: sha256.to_string(),
        mime_type: mime_type.to_string(),
    }
}

/// `loaded`, with `unloaded`'s images of `kind` put back in their places
/// (any place past the end, after it), each as `project` makes it: the order
/// the record was read in.
fn in_place<'m, T>(
    loaded: Vec<T>,
    unloaded: &'m [UnloadedImage],
    kind: ImageKind,
    project: impl Fn(&'m UnloadedImage) -> T,
) -> Vec<T> {
    let mut gaps: Vec<&UnloadedImage> = unloaded.iter().filter(|u| u.kind == kind).collect();
    if gaps.is_empty() {
        return loaded;
    }
    gaps.sort_by_key(|u| u.position);
    let mut loaded = loaded.into_iter();
    let mut gaps = gaps.into_iter().peekable();
    let mut all = Vec::new();
    loop {
        let place = all.len();
        match gaps.next_if(|gap| gap.position <= place) {
            Some(gap) => all.push(project(gap)),
            None => match loaded.next() {
                Some(item) => all.push(item),
                None => break,
            },
        }
    }
    all.extend(gaps.map(project));
    all
}

/// The types of the images a user message carries, loaded or not, in order
/// (what a history view shows of them; nothing is read or hashed).
pub fn user_image_types(message: &Message) -> Vec<&str> {
    let loaded = message
        .user_image_blocks
        .iter()
        .map(|block| block.mime_type());
    let unloaded = &message.unloaded_images;
    in_place(loaded.collect(), unloaded, ImageKind::User, |gap| {
        gap.reference.mime_type.as_str()
    })
}

/// The lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Whether `name` is a digest [`sha256_hex`] gives: 64 lowercase hex
/// digits. Only such a name is ever a sidecar's, or shown in a marker.
pub fn is_sha256_hex(name: &str) -> bool {
    name.len() == 64 && name.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Whether an image of `mime_type` whose text is `text` is one a session
/// stores: a type quecto admits, spelled exactly
/// ([`ImageMime::parse_exact`]), no longer than [`MAX_STORED_IMAGE_TEXT`].
pub fn is_storable(mime_type: &str, text: &str) -> bool {
    ImageMime::parse_exact(mime_type).is_some() && text.len() <= MAX_STORED_IMAGE_TEXT
}

/// The text a request shows for an image it cannot send; a reference that
/// is no digest is not echoed.
pub fn unavailable_marker(sha256: &str) -> String {
    match is_sha256_hex(sha256) {
        true => format!("[image unavailable: {}]", &sha256[..MARKER_DIGITS]),
        false => "[image unavailable]".to_string(),
    }
}

/// The line a recall adds for the `count` images it does not bring back.
pub fn not_recalled_marker(count: usize) -> String {
    format!("[{count} image(s) not recalled]")
}

/// The estimated tokens of `message`'s images, as its estimate counts them.
pub fn image_tokens(message: &Message) -> usize {
    message
        .image_blocks
        .iter()
        .chain(&message.user_image_blocks)
        .map(|block| estimate_image_tokens(block.mime(), block.data()))
        .sum()
}

/// Whether `message` has a body to retain: text, or a user's image (#2424).
pub fn has_text_or_user_image(message: &Message) -> bool {
    !message.content.is_empty() || !message.user_image_blocks.is_empty()
}

/// Release every image `message` carries, loaded or not (a collapse: the
/// message's body is in session memory).
pub fn release_images(message: &mut Message) {
    message.image_blocks.clear();
    message.user_image_blocks.clear();
    message.unloaded_images.clear();
}

/// Put back the images of a loaded `message`, by kind, each reference with
/// its text if its sidecar was read (and verified against it). Every image,
/// a tool result's or a user's, is re-admitted by the strict rules
/// ([`ImageBlock::restore`], #2423), which admit exactly what was stored:
/// the text verbatim, its digest known. Any other (an unknown type, text
/// that is not strict base64, an unreadable header) stays on the message,
/// unloaded, in its place, and is sent as a marker. Returns how many stayed
/// unloaded.
pub fn restore_images(
    message: &mut Message,
    tool: Vec<(ImageRef, Option<VerifiedText>)>,
    user: Vec<(ImageRef, Option<VerifiedText>)>,
) -> usize {
    let typed = |reference: &ImageRef| ImageMime::parse_exact(&reference.mime_type);
    // Each text was hashed when its sidecar was read: a block takes that
    // digest, so a load hashes no image a second time.
    let read = |reference: &ImageRef, text: &VerifiedText| {
        assert_eq!(
            text.sha256(),
            reference.sha256,
            "a sidecar read is its reference's"
        );
    };
    let restore = |reference: &ImageRef, text: Option<VerifiedText>| match (typed(reference), text)
    {
        (Some(mime), Some(text)) => {
            read(reference, &text);
            // Admitted exactly as it was stored, or not at all.
            ImageBlock::restore_verified(mime, text)
        }
        (_, _) => None,
    };
    let lists = [
        (ImageKind::Tool, tool, &mut message.image_blocks),
        (ImageKind::User, user, &mut message.user_image_blocks),
    ];
    for (kind, references, blocks) in lists {
        for (position, (reference, text)) in references.into_iter().enumerate() {
            match restore(&reference, text) {
                Some(block) => blocks.push(block),
                None => message.unloaded_images.push(UnloadedImage {
                    kind,
                    position,
                    reference,
                }),
            }
        }
    }
    message.invalidate_token_cache();
    message.unloaded_images.len()
}

#[cfg(test)]
#[path = "stored_images_tests.rs"]
mod tests;
