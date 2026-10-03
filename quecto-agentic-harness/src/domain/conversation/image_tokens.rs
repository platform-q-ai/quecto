//! The token estimate of an image, from its pixel size (#2420).
//!
//! Providers price an image by its pixels, not its bytes: Anthropic's
//! newer models take up to 2576 px on the long edge and bill 28x28 px
//! patches, about `w*h/750` tokens, at most 4,784 an image. OpenAI bills
//! about 1.2 tokens per 32x32 patch; quecto always sends OpenAI and Codex
//! images at "high" detail, never "original", which caps them at 2,500
//! patches (3,000 tokens). So one conservative estimate covers every
//! provider: the long edge scaled to at most 2576 px, aspect kept, then
//! the larger of `ceil(w*h/750)` and the 28 px patch count (a thin strip
//! costs its patches), within [85, 4784]. The provider's own count comes
//! back with the usage anyway (`domain::context_calibration`).

// What an image is, and how its header is read, is `quecto_image`'s
// (#2422); how much a pixel size costs is this module's policy.
use quecto_image::{Dimensions, ImageMime};

/// The longest edge an image is priced at; a longer one is scaled down.
pub const MAX_LONG_EDGE: u32 = 2_576;
/// Pixels per token, after scaling.
pub const PIXELS_PER_TOKEN: u64 = 750;
/// The side of Anthropic's square patch, one token each, in pixels.
pub const PATCH_SIDE: u64 = 28;
/// The least an image costs.
pub const MIN_IMAGE_TOKENS: usize = 85;
/// The most an image costs.
pub const MAX_IMAGE_TOKENS: usize = 4_784;
/// An image whose header cannot be read (truncated, unknown format, bad
/// base64) costs the most an image can.
pub const UNREADABLE_IMAGE_TOKENS: usize = MAX_IMAGE_TOKENS;

/// Estimate the tokens of the `mime` image that `base64` encodes, from the
/// pixel size in its header. Never panics.
pub fn estimate_image_tokens(mime: ImageMime, base64: &str) -> usize {
    let tokens = match quecto_image::dimensions(mime, base64) {
        Some(dimensions) => tokens_at(scaled(dimensions)),
        None => UNREADABLE_IMAGE_TOKENS,
    };
    debug_assert!(
        (MIN_IMAGE_TOKENS..=MAX_IMAGE_TOKENS).contains(&tokens),
        "an image estimate is within the floor and the ceiling"
    );
    tokens
}

/// [`estimate_image_tokens`] for an image whose type is still a string (a
/// tool result's block, until #2423 types it): a type off the allowlist is
/// unreadable and costs the ceiling.
pub fn estimate_named_image_tokens(mime: &str, base64: &str) -> usize {
    match ImageMime::parse_exact(mime) {
        Some(mime) => estimate_image_tokens(mime, base64),
        None => UNREADABLE_IMAGE_TOKENS,
    }
}

/// `dimensions` with the long edge scaled to at most [`MAX_LONG_EDGE`],
/// the aspect ratio kept (each side rounded up, so the estimate errs high;
/// the long side lands exactly on the cap).
pub fn scaled(dimensions: Dimensions) -> Dimensions {
    let long = dimensions.width.max(dimensions.height);
    let scaled = match long > MAX_LONG_EDGE {
        true => {
            let side = |value: u32| {
                let value = (u64::from(value) * u64::from(MAX_LONG_EDGE)).div_ceil(u64::from(long));
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

/// The larger of `ceil(w*h/750)` and `ceil(w/28)*ceil(h/28)`, within
/// [`MIN_IMAGE_TOKENS`], [`MAX_IMAGE_TOKENS`].
fn tokens_at(dimensions: Dimensions) -> usize {
    let (width, height) = (u64::from(dimensions.width), u64::from(dimensions.height));
    let by_area = (width * height).div_ceil(PIXELS_PER_TOKEN);
    let by_patches = width.div_ceil(PATCH_SIDE) * height.div_ceil(PATCH_SIDE);
    usize::try_from(by_area.max(by_patches))
        .unwrap_or(MAX_IMAGE_TOKENS)
        .clamp(MIN_IMAGE_TOKENS, MAX_IMAGE_TOKENS)
}

#[cfg(test)]
#[path = "image_tokens_tests.rs"]
mod tests;
