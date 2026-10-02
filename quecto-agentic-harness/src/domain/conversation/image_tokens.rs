//! The token estimate of an image, from its pixel size (#2420).
//!
//! Providers price an image by its pixels, not its bytes: Anthropic's
//! newer models take up to 2576 px on the long edge at about `w*h/750`
//! tokens, at most 4,784 an image; OpenAI's "original" detail charges
//! about 1.2 tokens per 32x32 patch (1920x1080 is about 2,450). One
//! conservative estimate covers them: the long edge scaled to at most
//! 2576 px, aspect kept, then `ceil(w*h/750)` within [85, 4784]. The
//! provider's own count comes back with the usage anyway
//! (`domain::context_calibration`).

use super::image_dimensions::{Dimensions, image_dimensions};

/// The longest edge an image is priced at; a longer one is scaled down.
pub const MAX_LONG_EDGE: u32 = 2_576;
/// Pixels per token, after scaling.
pub const PIXELS_PER_TOKEN: u64 = 750;
/// The least an image costs.
pub const MIN_IMAGE_TOKENS: usize = 85;
/// The most an image costs.
pub const MAX_IMAGE_TOKENS: usize = 4_784;
/// An image whose header cannot be read (truncated, unknown format, bad
/// base64) costs the most an image can.
pub const UNREADABLE_IMAGE_TOKENS: usize = MAX_IMAGE_TOKENS;

/// Estimate the tokens of the `mime` image that `base64` encodes, from the
/// pixel size in its header. Never panics.
pub fn estimate_image_tokens(mime: &str, base64: &str) -> usize {
    let tokens = match image_dimensions(mime, base64) {
        Some(dimensions) => tokens_at(scaled(dimensions)),
        None => UNREADABLE_IMAGE_TOKENS,
    };
    debug_assert!(
        (MIN_IMAGE_TOKENS..=MAX_IMAGE_TOKENS).contains(&tokens),
        "an image estimate is within the floor and the ceiling"
    );
    tokens
}

/// `dimensions` with the long edge scaled to at most [`MAX_LONG_EDGE`],
/// the aspect ratio kept (each side rounded down, and at least 1).
pub fn scaled(dimensions: Dimensions) -> Dimensions {
    let long = dimensions.width.max(dimensions.height);
    let scaled = match long > MAX_LONG_EDGE {
        true => {
            let side = |value: u32| {
                let value = u64::from(value) * u64::from(MAX_LONG_EDGE) / u64::from(long);
                u32::try_from(value)
                    .unwrap_or(MAX_LONG_EDGE)
                    .clamp(1, MAX_LONG_EDGE)
            };
            Dimensions {
                width: side(dimensions.width),
                height: side(dimensions.height),
            }
        }
        false => dimensions,
    };
    debug_assert!(
        scaled.width.max(scaled.height) <= MAX_LONG_EDGE,
        "the long edge fits the cap"
    );
    debug_assert!(scaled.width.min(scaled.height) >= 1, "no side vanishes");
    scaled
}

/// `ceil(w*h/750)`, within [`MIN_IMAGE_TOKENS`], [`MAX_IMAGE_TOKENS`].
fn tokens_at(dimensions: Dimensions) -> usize {
    let pixels = u64::from(dimensions.width) * u64::from(dimensions.height);
    usize::try_from(pixels.div_ceil(PIXELS_PER_TOKEN))
        .unwrap_or(MAX_IMAGE_TOKENS)
        .clamp(MIN_IMAGE_TOKENS, MAX_IMAGE_TOKENS)
}

#[cfg(test)]
#[path = "image_tokens_tests.rs"]
mod tests;
