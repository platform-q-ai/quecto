//! Output that is not text (#2197). A stream is text unless more than a few
//! of its bytes, and more than a tenth of them, are suspect: bytes of
//! invalid UTF-8, or NULs that do not end a name. A NUL after two or more
//! other bytes separates records (`find -print0`, `printf 'a\0b\0'`) and is
//! text; NULs in runs or between single bytes (UTF-16, `/dev/zero`, an ELF
//! header) are not. A binary stream is named with its size, its stream and
//! its likely cause instead of being decoded into replacement characters.
//! `read` refuses such files (#2166); bash keeps the tolerance for stray
//! bytes, since command output mixes in the odd Latin-1 name or cut
//! character.
//!
//! Accepted limits: NUL-separated records of a single byte (`printf
//! 'a\0b\0…'`) look like UTF-16 byte for byte and are named as binary —
//! counting a NUL after one byte as a separator would pass UTF-16 text as
//! text. Text dense in Latin-1 accents is named as invalid UTF-8, with an
//! `iconv -f latin1` hint.

use super::capture::Stream;

/// Up to this many suspect bytes, output is text whatever its length.
pub(super) const FEW_SUSPECT_BYTES: usize = 8;

/// A stream is binary when suspect bytes are more than one in this many.
const SUSPECT_SHARE_DENOMINATOR: usize = 10;

/// A NUL after at least this many other bytes ends a record: text.
const RECORD_MIN_BYTES: usize = 2;

/// What a stream's bytes are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Content {
    Text,
    Binary(Cause),
}

/// Why a stream is binary: its more common kind of suspect byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Cause {
    /// Mostly invalid UTF-8.
    InvalidUtf8,
    /// Mostly NULs: binary data, or UTF-16 text.
    NulHeavy,
}

/// Judge `bytes`: binary when more than [`FEW_SUSPECT_BYTES`] and more
/// than a tenth of them are suspect, by the more common cause.
pub(super) fn classify(bytes: &[u8]) -> Content {
    let invalid: usize = bytes.utf8_chunks().map(|chunk| chunk.invalid().len()).sum();
    let nuls = suspect_nuls(bytes);
    let suspect = invalid + nuls;
    debug_assert!(
        suspect <= bytes.len(),
        "suspect bytes are bytes of the stream"
    );
    let binary = suspect > FEW_SUSPECT_BYTES && suspect * SUSPECT_SHARE_DENOMINATOR > bytes.len();
    match (binary, nuls > invalid) {
        (true, true) => Content::Binary(Cause::NulHeavy),
        (true, false) => Content::Binary(Cause::InvalidUtf8),
        (false, _) => Content::Text,
    }
}

/// NULs that do not end a record of [`RECORD_MIN_BYTES`] or more bytes.
fn suspect_nuls(bytes: &[u8]) -> usize {
    let mut run = 0_usize;
    let mut suspect = 0_usize;
    for byte in bytes {
        match *byte {
            0 => {
                suspect += usize::from(run < RECORD_MIN_BYTES);
                run = 0;
            }
            _ => run += 1,
        }
    }
    suspect
}

/// What is shown for a binary stream of `total` bytes: which stream, its
/// size, its likely cause and ways to look at it, on one line. `od` comes
/// first: it is in every POSIX system, where `xxd` and `file` may not be.
pub(super) fn binary_notice(total: usize, stream: Stream, cause: Cause) -> String {
    let name = stream.name();
    match cause {
        Cause::NulHeavy => format!(
            "[binary output on {name} ({total} bytes, mostly NUL bytes: binary data, or UTF-16 \
             text) not shown. For UTF-16 text, pipe the command through `iconv -f UTF-16 -t \
             UTF-8`; to look at the bytes, through `od -c | head -n 20`; to keep them, pass \
             output_file]"
        ),
        Cause::InvalidUtf8 => format!(
            "[binary output on {name} ({total} bytes, not UTF-8 text) not shown. To look at it, \
             pipe the command through `od -c | head -n 20` (or `xxd | head -n 40`, `file -` \
             where installed); for Latin-1 text, through `iconv -f latin1 -t utf-8`; to keep \
             the bytes, pass output_file]"
        ),
    }
}

#[cfg(test)]
#[path = "binary_tests.rs"]
mod tests;
