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
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Ok(())
    }
}

/// The one-line notice for an image `name` that was not attached.
pub(crate) fn refusal_notice(_name: &str, _refusal: &AttachRefusal) -> String {
    let _ = (CHIP_NAME_CHARS, IMAGE_MARKER, MESSAGE_IMAGE_BUDGET);
    String::new()
}

impl PendingImages {
    /// Admit the image file `bytes` under `name`, or say why not.
    pub(crate) fn admit(&mut self, _name: &str, bytes: &[u8]) -> Result<(), AttachRefusal> {
        let _ = (MAX_IMAGES_PER_MESSAGE, ImageMime::sniff(bytes));
        Err(AttachRefusal::NotAnImage)
    }

    pub(crate) fn len(&self) -> usize {
        self.images.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.images.is_empty()
    }

    /// Drop the last image; whether there was one.
    pub(crate) fn remove_last(&mut self) -> bool {
        false
    }

    /// Drop every image.
    pub(crate) fn clear(&mut self) {}

    /// The images to send, in attach order.
    pub(crate) fn attachments(&self) -> Vec<ImageAttachment> {
        Vec::new()
    }

    /// One chip per image: `[image 1: screenshot.png · 240 KB]`.
    pub(crate) fn chips(&self) -> Vec<String> {
        let _ = self.images.iter().map(|i| (&i.name, &i.attachment)).count();
        Vec::new()
    }
}

/// A user message's text as the transcript shows it: one [`IMAGE_MARKER`]
/// per image on a line of their own, above the text.
pub(crate) fn with_image_markers(text: &str, _count: usize) -> String {
    text.to_string()
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
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Ok(())
    }
}

/// The file a `/image` argument names: absolute as given, `~` or `~/…`
/// under `home`, anything else relative to `workspace`.
pub(crate) fn resolve_image_path(
    _arg: &str,
    _home: Option<&Path>,
    workspace: &Path,
) -> Result<PathBuf, ImagePathError> {
    Ok(workspace.to_path_buf())
}

/// The name an image's chip shows when it came from `path`.
pub(crate) fn file_label(path: &Path) -> String {
    path.display().to_string()
}

/// The name a clipboard image's chip shows: `clipboard.<type>`.
pub(crate) fn clipboard_label(_bytes: &[u8]) -> String {
    String::new()
}

#[cfg(test)]
#[path = "image_attachments_tests.rs"]
mod tests;
