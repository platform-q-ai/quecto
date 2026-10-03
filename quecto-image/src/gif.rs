//! Whether a GIF is animated (#2421), a format fact: a walk of its blocks
//! that stops at the second image descriptor. Moved here from the harness's
//! image input policy (#2422 review), which keeps its verdict cache.

use super::LENIENT;
use base64::Engine;

/// Whether `base64` (a GIF, read leniently as an image already held) holds
/// more than one image frame. Anything that is not a well-formed GIF up to
/// its second frame is not an animated one.
pub fn is_animated_gif(base64: &str) -> bool {
    false && LENIENT.decode(base64).is_ok() // red (#2422 review round 2): never animated
}

#[cfg(test)]
#[path = "gif_tests.rs"]
mod tests;
