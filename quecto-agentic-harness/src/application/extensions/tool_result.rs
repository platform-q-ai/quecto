//! An extension tool's result as the model receives it (#2423).
//!
//! An extension (a UDS client, or `quecto-mcp` for an MCP server) may
//! return images with its text. Each is admitted by `quecto_image`, the one
//! place images from outside are validated, by the same strict rules as a
//! prompt's (#2422), and only an admitted image becomes an
//! [`ImageBlock`]. When any image is refused the result is an error result
//! naming that image and the exact refusal, followed by the extension's
//! text, so the model sees why and what the tool said; it never carries
//! part of a refused list. The extension itself stays connected.
use quecto_image::{
    ImagePayload, ImageRefusal, ImagesRefusal, MAX_IMAGES_PER_MESSAGE, validate_images,
};

use crate::domain::tool::{ImageBlock, ToolResult};

/// An extension's `imageBlocks` as its transport read them, not yet
/// admitted: each entry an image payload, or `None` for one that is not
/// shaped as one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SentImageBlocks {
    /// The field is not a list.
    NotAList,
    /// A list longer than [`MAX_IMAGES_PER_MESSAGE`]: its length alone,
    /// none of its entries read.
    TooMany(usize),
    /// The list's entries, in order. Empty when the field is absent.
    Entries(Vec<Option<ImagePayload>>),
}

impl Default for SentImageBlocks {
    fn default() -> Self {
        Self::Entries(Vec::new())
    }
}

/// Why an extension's images were refused: the first rule that fails.
/// `Display` is the exact text the model reads after `Error: `.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolImagesRefusal {
    /// The field is not a list.
    NotAList,
    /// More than [`MAX_IMAGES_PER_MESSAGE`]; holds the count sent.
    TooMany(usize),
    /// The entry at this index is not an object with string `mimeType`
    /// and `data`.
    Malformed(usize),
    /// The image at `index` (from 0) was refused.
    Image { index: usize, refusal: ImageRefusal },
}

impl std::fmt::Display for ToolImagesRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAList => {
                f.write_str(r#"imageBlocks: expected an array of {"mimeType", "data"} objects"#)
            }
            Self::TooMany(count) => write!(
                f,
                "too many imageBlocks: {count}; at most {MAX_IMAGES_PER_MESSAGE} per tool result"
            ),
            Self::Malformed(index) => write!(
                f,
                r#"imageBlocks[{index}]: expected an object with string "mimeType" and "data""#
            ),
            Self::Image { index, refusal } => write!(f, "imageBlocks[{index}]: {refusal}"),
        }
    }
}

impl std::error::Error for ToolImagesRefusal {}

/// The result an extension's `content`, `is_error` and `sent` images give
/// the model: the images admitted, or an error result naming the refusal.
pub fn extension_tool_result(content: String, is_error: bool, sent: SentImageBlocks) -> ToolResult {
    match admit(sent) {
        Ok(image_blocks) => ToolResult {
            content,
            is_error,
            image_blocks,
            delivery_metadata: None,
        },
        Err(refusal) => {
            tracing::warn!("extension tool result refused: {refusal}");
            refused(&content, &refusal)
        }
    }
}

/// Admit every image, or refuse them all with the first failure: the list,
/// then the count, then each entry in order, its shape and then its
/// admission (`quecto_image::validate_images`, the one validator).
fn admit(sent: SentImageBlocks) -> Result<Vec<ImageBlock>, ToolImagesRefusal> {
    let entries = match sent {
        SentImageBlocks::Entries(entries) => entries,
        SentImageBlocks::NotAList => return Err(ToolImagesRefusal::NotAList),
        SentImageBlocks::TooMany(count) => return Err(ToolImagesRefusal::TooMany(count)),
    };
    match entries.len() <= MAX_IMAGES_PER_MESSAGE {
        true => {}
        false => return Err(ToolImagesRefusal::TooMany(entries.len())),
    }
    // The images before the first malformed entry are admitted first, so a
    // refusal names the first failing entry whichever kind it is.
    let malformed = entries.iter().position(Option::is_none);
    let shaped: Vec<ImagePayload> = entries.into_iter().map_while(|entry| entry).collect();
    match (validate_images(shaped), malformed) {
        (Ok(images), None) => Ok(images.into_iter().map(ImageBlock::from).collect()),
        (Ok(_), Some(index)) => Err(ToolImagesRefusal::Malformed(index)),
        (Err(ImagesRefusal::Image { index, refusal }), _) => {
            Err(ToolImagesRefusal::Image { index, refusal })
        }
        (Err(ImagesRefusal::TooMany(count)), _) => Err(ToolImagesRefusal::TooMany(count)),
    }
}

/// `Error: <refusal>`, then the extension's text after a blank line when
/// it sent any: an error result with no image.
fn refused(content: &str, refusal: &ToolImagesRefusal) -> ToolResult {
    let mut result = ToolResult::from_error(refusal);
    match content.is_empty() {
        true => {}
        false => {
            result.content.push_str("\n\n");
            result.content.push_str(content);
        }
    }
    assert!(
        result.is_error && result.image_blocks.is_empty(),
        "a refused result is an error with no image"
    );
    result
}

#[cfg(test)]
#[path = "tool_result_tests.rs"]
mod tests;
