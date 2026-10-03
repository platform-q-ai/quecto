//! Whether a GIF is animated (#2421), a format fact: a walk of its blocks
//! that stops at the second image descriptor. Moved here from the harness's
//! image input policy (#2422 review), which keeps its verdict cache.

use super::LENIENT;
use base64::Engine;

/// Whether `base64` (a GIF, read leniently as an image already held) holds
/// more than one image frame. Anything that is not a well-formed GIF up to
/// its second frame is not an animated one.
pub fn is_animated_gif(base64: &str) -> bool {
    LENIENT
        .decode(base64)
        .is_ok_and(|bytes| frames_up_to_two(&bytes) > 1)
}

/// The image frames of `bytes` (a GIF), counted up to 2.
fn frames_up_to_two(bytes: &[u8]) -> usize {
    const HEADER: usize = 13; // "GIF8?a" and the logical screen descriptor
    let mut frames = 0;
    let Some(flags) = bytes.get(10).filter(|_| bytes.starts_with(b"GIF8")) else {
        return frames;
    };
    let mut at = HEADER + colour_table(*flags);
    while frames < 2 {
        let next = match bytes.get(at) {
            Some(0x2C) => {
                frames += 1;
                // The descriptor's 9 bytes, its colour table, the LZW size.
                let local = bytes.get(at + 9).copied().map_or(0, colour_table);
                skip_sub_blocks(bytes, at + 10 + local + 1)
            }
            Some(0x21) => skip_sub_blocks(bytes, at + 2),
            Some(_) | None => None,
        };
        match next {
            Some(after) => at = after,
            None => break,
        }
    }
    frames
}

/// The bytes of the colour table a GIF `flags` byte declares.
fn colour_table(flags: u8) -> usize {
    match flags & 0x80 {
        0 => 0,
        _ => 3 << ((flags & 0x07) + 1),
    }
}

/// Where the data sub-blocks starting at `at` end (past their terminator).
fn skip_sub_blocks(bytes: &[u8], mut at: usize) -> Option<usize> {
    loop {
        let size = usize::from(*bytes.get(at)?);
        at += 1;
        match size {
            0 => return Some(at),
            _ => at += size,
        }
    }
}

#[cfg(test)]
#[path = "gif_tests.rs"]
mod tests;
