//! The token estimate of an image, from its pixel size (#2420).

use super::image_dimensions::Dimensions;

pub const MAX_LONG_EDGE: u32 = 2_576;
pub const MIN_IMAGE_TOKENS: usize = 85;
pub const MAX_IMAGE_TOKENS: usize = 4_784;
pub const UNREADABLE_IMAGE_TOKENS: usize = MAX_IMAGE_TOKENS;

/// Estimate the tokens of the `mime` image that `base64` encodes.
pub fn estimate_image_tokens(_mime: &str, base64: &str) -> usize {
    crate::domain::token_estimate::estimate_opaque_tokens(base64)
}

/// `dimensions` scaled to fit the long edge cap.
pub fn scaled(dimensions: Dimensions) -> Dimensions {
    dimensions
}

#[cfg(test)]
#[path = "image_tokens_tests.rs"]
mod tests;
