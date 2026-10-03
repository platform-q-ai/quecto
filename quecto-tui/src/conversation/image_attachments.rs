//! The images attached to the message being composed (#2425).
//!
//! Pure policy: no terminal, filesystem or process. The shell reads a file
//! (`/image <path>`) or the clipboard (`Ctrl+V`) and hands the bytes here.
//! Every check on an image is `quecto-image`'s ([`ImageMime::sniff`], then
//! [`ImageAttachment::from_bytes`]), so the TUI keeps no copy of the rules.
//! This module adds only what is the composer's own: the per-message count,
//! the one-frame budget, the chips' wording, the path a `/image` argument
//! names and the transcript's `[image]` markers.

use quecto_image::{ImageAttachment, ImageMime, ImageRefusal, MAX_IMAGES_PER_MESSAGE};
use std::path::{Path, PathBuf};

/// The base64 the images of one message may total: the protocol frame cap
/// less [`TEXT_RESERVE`] kept for the message text and the JSON around it.
/// One image at the limit (5 MiB of base64) fits; two large ones may not,
/// and an over-cap command would be dropped by the writer, never sent.
pub(crate) const MESSAGE_IMAGE_BUDGET: usize =
    quecto_line_io::PROTOCOL_LINE_CAP_BYTES - TEXT_RESERVE;

/// What [`MESSAGE_IMAGE_BUDGET`] keeps for the text: 1 MiB.
const TEXT_RESERVE: usize = 1024 * 1024;

/// The most characters of a file name a chip shows.
const CHIP_NAME_CHARS: usize = 40;

/// The transcript's stand-in for one image of a user message.
pub(crate) const IMAGE_MARKER: &str = "[image]";

/// One admitted image waiting to be sent, and the name its chip shows.
pub(crate) struct PendingImage {
    name: String,
    attachment: ImageAttachment,
}

/// The images attached to the message being composed, in attach order.
#[derive(Default)]
pub(crate) struct PendingImages {
    images: Vec<PendingImage>,
}

/// Why an image was not attached. `Display` is the reason a notice gives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AttachRefusal {
    /// The message already holds [`MAX_IMAGES_PER_MESSAGE`].
    TooMany,
    /// The bytes start with none of the admitted types' signatures.
    NotAnImage,
    /// `quecto-image` refused it (too large, unreadable header, …).
    Refused(ImageRefusal),
    /// It would take the message's images past [`MESSAGE_IMAGE_BUDGET`].
    OverMessageBudget,
}

impl std::fmt::Display for AttachRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooMany => write!(f, "at most {MAX_IMAGES_PER_MESSAGE} images per message"),
            Self::NotAnImage => {
                let (last, first) = ImageMime::ALL.split_last().expect("four types");
                let first: Vec<&str> = first.iter().map(|mime| mime.as_str()).collect();
                write!(f, "not an {} or {last} file", first.join(", "))
            }
            Self::Refused(refusal) => write!(f, "{refusal}"),
            Self::OverMessageBudget => write!(
                f,
                "one message's images may total {} MiB of base64, and this one would pass it",
                MESSAGE_IMAGE_BUDGET / (1024 * 1024)
            ),
        }
    }
}

/// The one-line notice for an image `name` that was not attached.
pub(crate) fn refusal_notice(name: &str, refusal: &AttachRefusal) -> String {
    format!("Image not attached: {name}: {refusal}")
}

impl PendingImages {
    /// Admit the image file `bytes` under `name`, or say why not. The type
    /// is read from the bytes, never from the name.
    pub(crate) fn admit(&mut self, name: &str, bytes: &[u8]) -> Result<(), AttachRefusal> {
        match self.images.len() < MAX_IMAGES_PER_MESSAGE {
            true => {}
            false => return Err(AttachRefusal::TooMany),
        }
        let mime = ImageMime::sniff(bytes).ok_or(AttachRefusal::NotAnImage)?;
        let attachment =
            ImageAttachment::from_bytes(mime, bytes).map_err(AttachRefusal::Refused)?;
        match self.encoded_len() + attachment.data().len() <= MESSAGE_IMAGE_BUDGET {
            true => {}
            false => return Err(AttachRefusal::OverMessageBudget),
        }
        self.images.push(PendingImage {
            name: name.to_string(),
            attachment,
        });
        assert!(
            self.images.len() <= MAX_IMAGES_PER_MESSAGE
                && self.encoded_len() <= MESSAGE_IMAGE_BUDGET,
            "the pending images stay within one message's limits"
        );
        Ok(())
    }

    /// The base64 the pending images total.
    fn encoded_len(&self) -> usize {
        self.images
            .iter()
            .map(|image| image.attachment.data().len())
            .sum()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.images.is_empty()
    }

    /// Drop the last image; whether there was one.
    pub(crate) fn remove_last(&mut self) -> bool {
        self.images.pop().is_some()
    }

    /// Drop every image.
    pub(crate) fn clear(&mut self) {
        self.images.clear();
    }

    /// The images to send, in attach order.
    pub(crate) fn attachments(&self) -> Vec<ImageAttachment> {
        self.images
            .iter()
            .map(|image| image.attachment.clone())
            .collect()
    }

    /// One chip per image: `[image 1: screenshot.png · 240 KB]`.
    pub(crate) fn chips(&self) -> Vec<String> {
        self.images
            .iter()
            .enumerate()
            .map(|(index, image)| {
                format!(
                    "[image {}: {} · {}]",
                    index + 1,
                    chip_name(&image.name),
                    file_size(image.attachment.decoded_len())
                )
            })
            .collect()
    }
}

/// `name`, cut to [`CHIP_NAME_CHARS`] with an ellipsis.
fn chip_name(name: &str) -> String {
    match name.chars().count() <= CHIP_NAME_CHARS {
        true => name.to_string(),
        false => {
            let kept: String = name.chars().take(CHIP_NAME_CHARS - 1).collect();
            format!("{kept}…")
        }
    }
}

/// A file size as a chip shows it: `73 B`, `240 KB`, `1.5 MB` (binary units).
fn file_size(bytes: usize) -> String {
    const KIB: usize = 1024;
    const MIB: usize = 1024 * 1024;
    match bytes {
        0..KIB => format!("{bytes} B"),
        KIB..MIB => format!("{} KB", (bytes + KIB / 2) / KIB),
        _ => {
            let tenths = (bytes * 10 + MIB / 2) / MIB;
            format!("{}.{} MB", tenths / 10, tenths % 10)
        }
    }
}

/// A user message's text as the transcript shows it: one [`IMAGE_MARKER`]
/// per image on a line of their own, above the text.
pub(crate) fn with_image_markers(text: &str, count: usize) -> String {
    let markers = vec![IMAGE_MARKER; count].join(" ");
    match (count, text.is_empty()) {
        (0, _) => text.to_string(),
        (_, true) => markers,
        (_, false) => format!("{markers}\n{text}"),
    }
}

/// Why a `/image` argument names no path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ImagePathError {
    /// No argument.
    Missing,
    /// `~` with no home directory to expand it to.
    NoHome,
}

impl std::fmt::Display for ImagePathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => {
                f.write_str("Usage: /image <path> (absolute, ~/… or relative to the workspace)")
            }
            Self::NoHome => f.write_str("Image not attached: ~ names no home directory"),
        }
    }
}

/// The file a `/image` argument names: absolute as given, `~` or `~/…`
/// under `home`, anything else relative to `workspace`. A path wrapped in
/// one pair of matching quotes (as a terminal drops a file) is unquoted.
pub(crate) fn resolve_image_path(
    arg: &str,
    home: Option<&Path>,
    workspace: &Path,
) -> Result<PathBuf, ImagePathError> {
    let arg = unquote(arg.trim());
    if arg.is_empty() {
        return Err(ImagePathError::Missing);
    }
    let under_home = match arg {
        "~" => Some(""),
        _ => arg.strip_prefix("~/"),
    };
    match (under_home, home) {
        (Some(rest), Some(home)) => Ok(home.join(rest)),
        (Some(_), None) => Err(ImagePathError::NoHome),
        (None, _) => Ok(workspace.join(arg)),
    }
}

/// `text` without one pair of matching surrounding quotes, `'…'` or `"…"`.
fn unquote(text: &str) -> &str {
    ['\'', '"']
        .into_iter()
        .find_map(|quote| {
            text.strip_prefix(quote)
                .and_then(|inner| inner.strip_suffix(quote))
        })
        .unwrap_or(text)
}

/// The name an image's chip shows when it came from `path`: its file name.
pub(crate) fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// The name a clipboard image's chip shows: `clipboard.<type>`, or
/// `clipboard` when the bytes are no admitted image.
pub(crate) fn clipboard_label(bytes: &[u8]) -> String {
    match ImageMime::sniff(bytes) {
        Some(mime) => {
            let subtype = mime.as_str().trim_start_matches("image/");
            format!("clipboard.{subtype}")
        }
        None => "clipboard".to_string(),
    }
}

#[cfg(test)]
#[path = "image_attachments_tests.rs"]
mod tests;
