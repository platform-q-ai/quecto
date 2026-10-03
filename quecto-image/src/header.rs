//! An image's pixel size from its header, read straight from base64: PNG
//! IHDR, JPEG SOFn, the GIF logical screen descriptor, and WebP
//! VP8/VP8L/VP8X. Only the 4-character groups that hold the bytes read are
//! decoded, so a 1 MB image costs a few dozen bytes of decoding (a JPEG one
//! more read per segment before its frame). Moved here from the harness
//! (#2420) so admission (#2422) and the token estimate read headers one way.
//!
//! Pure and total: no I/O, never a panic. Whatever is not a well-formed
//! header of the given type (truncated, corrupt, bad base64, a side of 0)
//! is `None`. Base64 is read leniently (see the crate docs).

use super::{ImageMime, LENIENT};
use base64::Engine;

/// A pixel size; both sides are above 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dimensions {
    pub width: u32,
    pub height: u32,
}

/// The most JPEG segments walked before the frame header. A camera file
/// has a dozen or so (APPn, DQT, DHT); an ICC profile split over APP2
/// chunks adds a few more.
pub const MAX_JPEG_SEGMENTS: usize = 256;

/// The pixel size of the `mime` image that `base64` encodes; `None` when
/// the header cannot be read.
pub fn dimensions(mime: ImageMime, base64: &str) -> Option<Dimensions> {
    let bytes = Encoded(base64.as_bytes());
    let (width, height) = match mime {
        ImageMime::Png => png(&bytes),
        ImageMime::Jpeg => jpeg(&bytes),
        ImageMime::Gif => gif(&bytes),
        ImageMime::Webp => webp(&bytes),
    }?;
    (width > 0 && height > 0).then_some(Dimensions { width, height })
}

/// Random access to the bytes base64 encodes: byte `n` lives in the
/// 4-character group `n / 3`, so a read decodes only its own groups.
struct Encoded<'a>(&'a [u8]);

impl Encoded<'_> {
    /// The `N` bytes at `offset`, or `None` past the end or on bad base64.
    fn read<const N: usize>(&self, offset: usize) -> Option<[u8; N]> {
        let first = offset / 3;
        let last = offset.checked_add(N)?.div_ceil(3);
        let start = first.checked_mul(4)?;
        let end = last.checked_mul(4)?.min(self.0.len());
        let decoded = LENIENT.decode(self.0.get(start..end)?).ok()?;
        let skip = offset - first * 3;
        decoded.get(skip..skip.checked_add(N)?)?.try_into().ok()
    }
}

fn be16(bytes: [u8; 2]) -> u32 {
    u32::from(u16::from_be_bytes(bytes))
}

fn le16(bytes: [u8; 2]) -> u32 {
    u32::from(u16::from_le_bytes(bytes))
}

fn le24([a, b, c]: [u8; 3]) -> u32 {
    u32::from_le_bytes([a, b, c, 0])
}

/// The PNG signature, then IHDR first: length, type, width, height (BE).
fn png(bytes: &Encoded<'_>) -> Option<(u32, u32)> {
    const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    let h = bytes.read::<24>(0)?;
    (h[..8] == SIGNATURE[..] && h[12..16] == *b"IHDR").then(|| {
        (
            u32::from_be_bytes([h[16], h[17], h[18], h[19]]),
            u32::from_be_bytes([h[20], h[21], h[22], h[23]]),
        )
    })
}

/// "GIF87a" or "GIF89a", then the logical screen's width and height (LE).
fn gif(bytes: &Encoded<'_>) -> Option<(u32, u32)> {
    match bytes.read::<10>(0)? {
        [b'G', b'I', b'F', b'8', b'7' | b'9', b'a', w0, w1, h0, h1] => {
            Some((le16([w0, w1]), le16([h0, h1])))
        }
        _ => None,
    }
}

/// A RIFF/WEBP container whose first chunk is one of the three bitstream
/// kinds; its payload starts at byte 20.
fn webp(bytes: &Encoded<'_>) -> Option<(u32, u32)> {
    match bytes.read::<16>(0)? {
        [
            b'R',
            b'I',
            b'F',
            b'F',
            _,
            _,
            _,
            _,
            b'W',
            b'E',
            b'B',
            b'P',
            b'V',
            b'P',
            b'8',
            kind,
        ] => match kind {
            b' ' => webp_lossy(bytes),
            b'L' => webp_lossless(bytes),
            b'X' => webp_extended(bytes),
            _ => None,
        },
        _ => None,
    }
}

/// VP8: a 3-byte frame tag, the start code, then 14-bit sides (LE; the
/// top two bits are the upscaling hint).
fn webp_lossy(bytes: &Encoded<'_>) -> Option<(u32, u32)> {
    match bytes.read::<10>(20)? {
        [_, _, _, 0x9D, 0x01, 0x2A, w0, w1, h0, h1] => {
            Some((le16([w0, w1]) & 0x3FFF, le16([h0, h1]) & 0x3FFF))
        }
        _ => None,
    }
}

/// VP8L: the 0x2F signature, then 14-bit sides minus one, packed LE.
fn webp_lossless(bytes: &Encoded<'_>) -> Option<(u32, u32)> {
    match bytes.read::<5>(20)? {
        [0x2F, b0, b1, b2, b3] => {
            let bits = u32::from_le_bytes([b0, b1, b2, b3]);
            Some(((bits & 0x3FFF) + 1, ((bits >> 14) & 0x3FFF) + 1))
        }
        _ => None,
    }
}

/// VP8X: 4 bytes of flags, then the canvas's 24-bit sides minus one (LE).
fn webp_extended(bytes: &Encoded<'_>) -> Option<(u32, u32)> {
    let [_, _, _, _, w0, w1, w2, h0, h1, h2] = bytes.read::<10>(20)?;
    Some((le24([w0, w1, w2]) + 1, le24([h0, h1, h2]) + 1))
}

/// What a JPEG marker byte (after 0xFF) is to the walk.
enum Marker {
    /// A frame header (SOFn): precision, then height and width (BE).
    Frame,
    /// A segment before the frame (tables, APPn, comments): skipped by
    /// its length.
    Segment,
    /// A marker with no length (RSTn, TEM).
    Standalone,
    /// A fill byte: another 0xFF before the marker.
    Fill,
}

impl Marker {
    fn of(byte: u8) -> Option<Self> {
        match byte {
            0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => Some(Self::Frame),
            0xC4 | 0xCC | 0xDB..=0xDF | 0xE0..=0xFE => Some(Self::Segment),
            0x01 | 0xD0..=0xD7 => Some(Self::Standalone),
            0xFF => Some(Self::Fill),
            _ => None,
        }
    }
}

/// SOI, then the segments up to the first frame header, each jumped by its
/// length. A scan, an end, or any other marker before the frame is `None`.
fn jpeg(bytes: &Encoded<'_>) -> Option<(u32, u32)> {
    let [0xFF, 0xD8] = bytes.read::<2>(0)? else {
        return None;
    };
    let mut at = 2usize;
    for _ in 0..MAX_JPEG_SEGMENTS {
        let [0xFF, byte] = bytes.read::<2>(at)? else {
            return None;
        };
        match Marker::of(byte)? {
            Marker::Frame => {
                let [_, _, _, h0, h1, w0, w1] = bytes.read::<7>(at.checked_add(2)?)?;
                return Some((be16([w0, w1]), be16([h0, h1])));
            }
            Marker::Segment => {
                let length = be16(bytes.read::<2>(at.checked_add(2)?)?);
                let length = usize::try_from(length).ok().filter(|len| *len >= 2)?;
                at = at.checked_add(2 + length)?;
            }
            Marker::Standalone => at = at.checked_add(2)?,
            Marker::Fill => at = at.checked_add(1)?,
        }
    }
    None
}

#[cfg(test)]
#[path = "header_tests.rs"]
mod tests;
