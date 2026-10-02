//! Validated image attachments (#2422): the one place quecto decides whether
//! an image from outside may enter a conversation.
//!
//! Every peer that accepts an image builds an [`ImageAttachment`] with
//! [`ImageAttachment::new`] (one image) or [`validate_images`] (a message's
//! list) and refuses with the [`ImageRefusal`] / [`ImagesRefusal`] message
//! when it fails:
//! - the agent's UDS `prompt`, `steer` and `follow_up` (#2422);
//! - `quecto-api`'s `/prompt` and its WebSocket prompt frame (#2422);
//! - extension `tool_result` images and MCP image passthrough (#2423);
//! - the TUI's attachments (#2425).
//!
//! The rules are an allowlist, checked in this order; the first that fails
//! is the refusal. A message carries at most [`MAX_IMAGES_PER_MESSAGE`]. An
//! image is admitted only when its MIME type is one of [`ImageMime`]'s four
//! (exact, lowercase), it decodes to at most [`MAX_IMAGE_BYTES`] (longer
//! base64 is refused before it is decoded), its data is standard base64
//! (padded, no whitespace), and its decoded bytes start with that type's file
//! signature.

use base64::Engine as _;
use serde::{Deserialize, Serialize};

/// The most bytes one image may decode to: 5 MiB.
pub const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;

/// The most images one message may carry.
pub const MAX_IMAGES_PER_MESSAGE: usize = 8;

/// The longest base64 text that can decode to [`MAX_IMAGE_BYTES`]: anything
/// longer is refused before it is decoded.
const MAX_ENCODED_LEN: usize = MAX_IMAGE_BYTES.div_ceil(3) * 4;

/// How many characters of an unsupported MIME type a refusal echoes back.
const MIME_ECHO_CHARS: usize = 64;

/// An image type the harness admits, and the only ones it admits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageMime {
    Png,
    Jpeg,
    Gif,
    Webp,
}

impl ImageMime {
    /// Every admitted type, in the order refusals name them.
    pub const ALL: [Self; 4] = [Self::Png, Self::Jpeg, Self::Gif, Self::Webp];

    /// The MIME type as spelled on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
        }
    }

    /// The admitted type `declared` names exactly; `None` for anything else.
    pub fn parse(declared: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mime| mime.as_str() == declared)
    }

    /// Whether `bytes` start with this type's file signature.
    pub fn signature_matches(self, bytes: &[u8]) -> bool {
        match self {
            Self::Png => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
            Self::Jpeg => bytes.starts_with(&[0xFF, 0xD8, 0xFF]),
            Self::Gif => bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
            Self::Webp => {
                bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP"
            }
        }
    }
}

impl std::fmt::Display for ImageMime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An image as a peer spells it on the wire, not yet validated:
/// `{"mimeType": "image/png", "data": "<standard base64>"}`.
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImagePayload {
    pub mime_type: String,
    pub data: String,
}

impl ImagePayload {
    pub fn new(mime_type: impl Into<String>, data: impl Into<String>) -> Self {
        Self {
            mime_type: mime_type.into(),
            data: data.into(),
        }
    }
}

/// Never prints the base64: a payload is megabytes of it.
impl std::fmt::Debug for ImagePayload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImagePayload")
            .field("mime_type", &self.mime_type)
            .field("data_len", &self.data.len())
            .finish()
    }
}

/// An image the rules admitted. Only [`ImageAttachment::new`] makes one, so
/// holding one is proof it passed every check. It keeps the base64 text it
/// was given, which is what providers send.
#[derive(Clone, PartialEq, Eq)]
pub struct ImageAttachment {
    mime: ImageMime,
    data: String,
    decoded_len: usize,
}

impl ImageAttachment {
    /// Admit `payload`, or say why not.
    pub fn new(payload: ImagePayload) -> Result<Self, ImageRefusal> {
        let (mime, decoded_len) = check(&payload)?;
        assert!(
            decoded_len <= MAX_IMAGE_BYTES,
            "admitted an oversized image"
        );
        Ok(Self {
            mime,
            data: payload.data,
            decoded_len,
        })
    }

    pub fn mime(&self) -> ImageMime {
        self.mime
    }

    pub fn mime_type(&self) -> &'static str {
        self.mime.as_str()
    }

    /// The image's standard base64 text.
    pub fn data(&self) -> &str {
        &self.data
    }

    /// How many bytes the image decodes to.
    pub fn decoded_len(&self) -> usize {
        self.decoded_len
    }

    pub fn into_data(self) -> String {
        self.data
    }
}

/// Never prints the base64.
impl std::fmt::Debug for ImageAttachment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageAttachment")
            .field("mime", &self.mime)
            .field("decoded_len", &self.decoded_len)
            .finish()
    }
}

/// Serialises in the wire shape it was admitted from, so a peer forwards an
/// admitted image exactly as [`ImagePayload`] spells it.
impl Serialize for ImageAttachment {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut image = serializer.serialize_struct("ImageAttachment", 2)?;
        image.serialize_field("mimeType", self.mime.as_str())?;
        image.serialize_field("data", &self.data)?;
        image.end()
    }
}

/// Why one image was refused. Its `Display` is the exact refusal text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageRefusal {
    /// The declared MIME type is not one of [`ImageMime::ALL`]; holds the
    /// declared type, cut to its first 64 characters.
    UnsupportedMime(String),
    /// The data is not standard (padded, whitespace-free) base64.
    InvalidBase64,
    /// The data decodes to more than [`MAX_IMAGE_BYTES`].
    TooLarge,
    /// The decoded bytes do not start with the declared type's signature.
    SignatureMismatch(ImageMime),
}

impl std::fmt::Display for ImageRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedMime(declared) => write!(
                f,
                "mimeType {declared:?} is not allowed; use image/png, image/jpeg, image/gif or image/webp"
            ),
            Self::InvalidBase64 => f.write_str("data is not valid standard base64"),
            Self::TooLarge => write!(
                f,
                "image decodes to more than {MAX_IMAGE_BYTES} bytes (5 MiB)"
            ),
            Self::SignatureMismatch(mime) => {
                write!(f, "data does not start with the {mime} signature")
            }
        }
    }
}

impl std::error::Error for ImageRefusal {}

/// Why a message's images were refused: the whole list is refused when any
/// one image is, naming that image's index. `Display` is the exact text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImagesRefusal {
    /// More than [`MAX_IMAGES_PER_MESSAGE`]; holds the count sent.
    TooMany(usize),
    /// The image at `index` (from 0) was refused.
    Image { index: usize, refusal: ImageRefusal },
}

impl std::fmt::Display for ImagesRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooMany(count) => write!(
                f,
                "too many images: {count}; at most {MAX_IMAGES_PER_MESSAGE} per message"
            ),
            Self::Image { index, refusal } => write!(f, "images[{index}]: {refusal}"),
        }
    }
}

impl std::error::Error for ImagesRefusal {}

/// Admit every image of one message, or refuse them all with the first
/// failure.
pub fn validate_images(payloads: Vec<ImagePayload>) -> Result<Vec<ImageAttachment>, ImagesRefusal> {
    check_count(payloads.len())?;
    payloads
        .into_iter()
        .enumerate()
        .map(|(index, payload)| {
            ImageAttachment::new(payload).map_err(|refusal| ImagesRefusal::Image { index, refusal })
        })
        .collect()
}

/// [`validate_images`] without taking the payloads: for a peer that only
/// decides whether to act on a command it forwards whole.
pub fn check_images(payloads: &[ImagePayload]) -> Result<(), ImagesRefusal> {
    check_count(payloads.len())?;
    for (index, payload) in payloads.iter().enumerate() {
        check(payload).map_err(|refusal| ImagesRefusal::Image { index, refusal })?;
    }
    Ok(())
}

fn check_count(count: usize) -> Result<(), ImagesRefusal> {
    if count <= MAX_IMAGES_PER_MESSAGE {
        Ok(())
    } else {
        Err(ImagesRefusal::TooMany(count))
    }
}

/// The admitted type and decoded length of `payload`, checked in the order
/// the crate documents.
fn check(payload: &ImagePayload) -> Result<(ImageMime, usize), ImageRefusal> {
    let Some(mime) = ImageMime::parse(&payload.mime_type) else {
        let echoed = payload.mime_type.chars().take(MIME_ECHO_CHARS).collect();
        return Err(ImageRefusal::UnsupportedMime(echoed));
    };
    if payload.data.len() > MAX_ENCODED_LEN {
        return Err(ImageRefusal::TooLarge);
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&payload.data)
        .map_err(|_| ImageRefusal::InvalidBase64)?;
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(ImageRefusal::TooLarge);
    }
    if mime.signature_matches(&bytes) {
        Ok((mime, bytes.len()))
    } else {
        Err(ImageRefusal::SignatureMismatch(mime))
    }
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
