//! Images in quecto (#2422): the one owner of what an image is.
//!
//! This crate owns the format facts every peer shares, so no other crate
//! keeps its own copy:
//! - the allowlist and its wire spelling, [`ImageMime`] (exact, lowercase:
//!   [`ImageMime::parse_exact`]), and file-signature sniffing
//!   ([`ImageMime::sniff`]);
//! - the size limit, [`MAX_IMAGE_BYTES`], and the count limit,
//!   [`MAX_IMAGES_PER_MESSAGE`];
//! - base64 (see below);
//! - header parsing: an image's pixel size, [`dimensions`] (base64) and
//!   [`dimensions_of_bytes`] (a file's bytes), and whether a GIF is
//!   animated, [`is_animated_gif`].
//!
//! Every peer that accepts an image from outside admits it as an
//! [`ImageAttachment`] with [`ImageAttachment::new`] (one image),
//! [`validate_images`] (a message's list) or [`ImageAttachment::from_bytes`]
//! (a file it read), and refuses with the [`ImageRefusal`] / [`ImagesRefusal`]
//! text when it fails: today the agent's UDS `prompt` / `steer` /
//! `follow_up` and the `read` tool, `quecto-api` (#2422), and the TUI's
//! attached images (#2425). Extension and MCP tool results (#2423) will
//! admit theirs here too.
//!
//! The rules are an allowlist, checked in this order; the first that fails
//! is the refusal. A message carries at most [`MAX_IMAGES_PER_MESSAGE`]. An
//! image is admitted only when its MIME type is one of [`ImageMime`]'s four,
//! it decodes to at most [`MAX_IMAGE_BYTES`] (longer base64 is refused before
//! it is decoded), its data is strict standard base64, its bytes start with
//! that type's file signature, and its header is readable (its pixel size
//! can be read).
//!
//! **Base64, in one place.** An image from outside must be strict standard
//! base64: padded, canonical, no whitespace or line breaks; [`encode`]
//! writes that. Reading the header of an image already held ([`dimensions`],
//! e.g. a tool result's) is lenient: padding optional and non-canonical
//! trailing bits accepted, as some encoders produce them. Nothing else in
//! quecto decodes image base64.

use base64::Engine as _;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use serde::{Deserialize, Serialize};

mod gif;
mod header;
pub use gif::is_animated_gif;
pub use header::{Dimensions, MAX_JPEG_SEGMENTS, dimensions, dimensions_of_bytes};

/// Real minimal image files (PNG, JPEG, GIF, WebP) for tests here and in
/// the crates that use this one (`test-support` feature).
#[cfg(any(test, feature = "test-support"))]
pub mod samples;

/// The most bytes one image may decode to: 3.75 MiB (3,932,160 bytes), so
/// its base64 is at most 5 MiB. Anthropic's 5 MB limit holds whether a
/// provider applies it to the decoded or to the encoded size.
pub const MAX_IMAGE_BYTES: usize = 3 * 1024 * 1024 + 768 * 1024;

/// The most images one message may carry.
pub const MAX_IMAGES_PER_MESSAGE: usize = 8;

/// The longest base64 text that can decode to [`MAX_IMAGE_BYTES`] (exactly
/// 5 MiB): the base64 length of the limit. Anything longer is refused
/// before it is decoded.
pub const MAX_ENCODED_LEN: usize = MAX_IMAGE_BYTES.div_ceil(3) * 4;

/// How many characters of an unsupported MIME type a refusal echoes back.
const MIME_ECHO_CHARS: usize = 64;

/// Strict standard base64, for images from outside.
const STRICT: GeneralPurpose = base64::engine::general_purpose::STANDARD;

/// Lenient standard base64, for reading the header of an image already
/// held: padding optional, non-canonical trailing bits accepted.
const LENIENT: GeneralPurpose = GeneralPurpose::new(
    &base64::alphabet::STANDARD,
    GeneralPurposeConfig::new()
        .with_decode_padding_mode(DecodePaddingMode::Indifferent)
        .with_decode_allow_trailing_bits(true),
);

/// `bytes` as strict standard base64, the form an image travels in.
pub fn encode(bytes: &[u8]) -> String {
    STRICT.encode(bytes)
}

/// An image type quecto admits, and the only ones it admits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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

    /// The admitted type `declared` spells exactly as the wire does
    /// (lowercase, no parameters); `None` for anything else, including
    /// another case of an admitted type.
    pub fn parse_exact(declared: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mime| mime.as_str() == declared)
    }

    /// The admitted type whose file signature `bytes` start with.
    pub fn sniff(bytes: &[u8]) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|mime| mime.signature_matches(bytes))
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

/// An image the rules admitted. Only [`ImageAttachment::new`] and
/// [`ImageAttachment::from_bytes`] make one, so holding one is proof it
/// passed every check. It keeps the base64 text, which is what providers
/// send, and the pixel size its header gave.
#[derive(Clone, PartialEq, Eq)]
pub struct ImageAttachment {
    mime: ImageMime,
    data: String,
    decoded_len: usize,
    dimensions: Dimensions,
}

impl ImageAttachment {
    /// Admit `payload`, or say why not.
    pub fn new(payload: ImagePayload) -> Result<Self, ImageRefusal> {
        let Some(mime) = ImageMime::parse_exact(&payload.mime_type) else {
            let echoed = payload.mime_type.chars().take(MIME_ECHO_CHARS).collect();
            return Err(ImageRefusal::UnsupportedMime(echoed));
        };
        let bytes = match payload.data.len() <= MAX_ENCODED_LEN {
            true => STRICT
                .decode(&payload.data)
                .map_err(|_| ImageRefusal::InvalidBase64)?,
            false => return Err(ImageRefusal::TooLarge),
        };
        Self::admit(mime, &bytes, || payload.data)
    }

    /// Admit the `mime` file `bytes` (a file read from disk), encoding it
    /// once it is admitted.
    pub fn from_bytes(mime: ImageMime, bytes: &[u8]) -> Result<Self, ImageRefusal> {
        Self::admit(mime, bytes, || encode(bytes))
    }

    /// The checks on the decoded bytes, the one size check among them:
    /// size, signature, readable header. `data` gives the base64.
    fn admit(
        mime: ImageMime,
        bytes: &[u8],
        data: impl FnOnce() -> String,
    ) -> Result<Self, ImageRefusal> {
        match (
            bytes.len() <= MAX_IMAGE_BYTES,
            mime.signature_matches(bytes),
        ) {
            (true, true) => {}
            (false, _) => return Err(ImageRefusal::TooLarge),
            (true, false) => return Err(ImageRefusal::SignatureMismatch(mime)),
        }
        let dimensions = dimensions_of_bytes(mime, bytes).ok_or(ImageRefusal::Unreadable(mime))?;
        let data = data();
        assert!(
            dimensions.width > 0 && dimensions.height > 0,
            "an admitted image has a size"
        );
        Ok(Self {
            mime,
            data,
            decoded_len: bytes.len(),
            dimensions,
        })
    }

    pub fn mime(&self) -> ImageMime {
        self.mime
    }

    pub fn mime_type(&self) -> &'static str {
        self.mime.as_str()
    }

    /// The image's strict standard base64 text.
    pub fn data(&self) -> &str {
        &self.data
    }

    /// How many bytes the image decodes to.
    pub fn decoded_len(&self) -> usize {
        self.decoded_len
    }

    /// The pixel size its header gave.
    pub fn dimensions(&self) -> Dimensions {
        self.dimensions
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
            .field("dimensions", &self.dimensions)
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
    /// The data is not strict standard base64.
    InvalidBase64,
    /// The image decodes to more than [`MAX_IMAGE_BYTES`].
    TooLarge,
    /// The decoded bytes do not start with the declared type's signature.
    SignatureMismatch(ImageMime),
    /// The signature is right but the header is not readable: its pixel
    /// size cannot be read (a bare signature, a truncated or corrupt file).
    Unreadable(ImageMime),
}

impl std::fmt::Display for ImageRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedMime(declared) => {
                write!(f, "mimeType {declared:?} is not allowed; use ")?;
                let (last, first) = ImageMime::ALL.split_last().expect("four types");
                let first: Vec<&str> = first.iter().map(|mime| mime.as_str()).collect();
                write!(f, "{} or {last}", first.join(", "))
            }
            Self::InvalidBase64 => f.write_str("data is not valid standard base64"),
            Self::TooLarge => write!(
                f,
                "image decodes to more than {MAX_IMAGE_BYTES} bytes ({} MiB)",
                mebibytes(MAX_IMAGE_BYTES)
            ),
            Self::SignatureMismatch(mime) => {
                write!(f, "data does not start with the {mime} signature")
            }
            Self::Unreadable(mime) => write!(f, "not a readable {mime} image"),
        }
    }
}

impl std::error::Error for ImageRefusal {}

/// `bytes` in MiB, as few decimals as it needs: 3932160 is "3.75".
fn mebibytes(bytes: usize) -> String {
    let hundredths = bytes * 100 / (1024 * 1024);
    let text = format!("{}.{:02}", hundredths / 100, hundredths % 100);
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// Deserialize a message's `images`, taking `null` as absent: no images.
/// For a wire field declared `#[serde(default, deserialize_with = ...)]`.
pub fn images_or_null<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<ImagePayload>, D::Error> {
    Option::<Vec<ImagePayload>>::deserialize(deserializer).map(Option::unwrap_or_default)
}

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
    match payloads.len() <= MAX_IMAGES_PER_MESSAGE {
        true => payloads
            .into_iter()
            .enumerate()
            .map(|(index, payload)| {
                ImageAttachment::new(payload)
                    .map_err(|refusal| ImagesRefusal::Image { index, refusal })
            })
            .collect(),
        false => Err(ImagesRefusal::TooMany(payloads.len())),
    }
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
