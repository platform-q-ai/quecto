//! Lines as grep shows them, decoded from raw bytes (#2251 review): bytes
//! that are not UTF-8 become U+FFFD (three bytes each, however many raw
//! bytes they stand for), and where each replacement stands is kept, so
//! rg's offsets (into the raw bytes) find their text, and what the agent is
//! told (byte offsets and counts, for `cut -b`) is in raw bytes again.

use std::ops::Range;

/// The bytes a U+FFFD takes in the decoded text.
const REPLACEMENT_LEN: usize = char::REPLACEMENT_CHARACTER.len_utf8();

/// One line: its decoded text, and where it differs from its raw bytes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct Decoded {
    pub text: String,
    /// Each U+FFFD put in for invalid bytes, in order.
    replaced: Vec<Replacement>,
}

/// A U+FFFD standing for invalid bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Replacement {
    /// Its offset in the text.
    shown: usize,
    /// The offset of the bytes it stands for in the raw line.
    raw: usize,
    /// How many raw bytes it stands for (1 to 3).
    len: usize,
}

impl Replacement {
    /// How much longer the text is than the raw bytes once past it.
    fn grown_after(&self) -> usize {
        (self.shown + REPLACEMENT_LEN) - (self.raw + self.len)
    }
}

impl Decoded {
    /// `bytes` decoded as `String::from_utf8_lossy` does.
    pub(super) fn new(bytes: &[u8]) -> Self {
        let mut text = String::with_capacity(bytes.len());
        let mut replaced = Vec::new();
        let mut raw = 0;
        for chunk in bytes.utf8_chunks() {
            text.push_str(chunk.valid());
            raw += chunk.valid().len();
            let len = chunk.invalid().len();
            if len > 0 {
                assert!(
                    len <= REPLACEMENT_LEN,
                    "an invalid sequence is at most 3 bytes, got {len}"
                );
                replaced.push(Replacement {
                    shown: text.len(),
                    raw,
                    len,
                });
                text.push(char::REPLACEMENT_CHARACTER);
                raw += len;
            }
        }
        let line = Self { text, replaced };
        assert_eq!(line.raw_len(), bytes.len(), "the raw length is kept");
        line
    }

    /// How many raw bytes the line has.
    pub(super) fn raw_len(&self) -> usize {
        self.raw_at(self.text.len())
    }

    /// Where raw offset `raw` lands in the text: inside replaced bytes, at
    /// their U+FFFD; past the raw bytes, at the text's end.
    pub(super) fn shown_at(&self, raw: usize) -> usize {
        let passed = self.replaced.partition_point(|r| r.raw + r.len <= raw);
        match (self.replaced.get(passed), passed.checked_sub(1)) {
            (Some(r), _) if r.raw <= raw => r.shown,
            (_, Some(last)) => (raw + self.replaced[last].grown_after()).min(self.text.len()),
            (_, None) => raw.min(self.text.len()),
        }
    }

    /// The raw offset of text offset `at`: inside a U+FFFD, the start of
    /// the bytes it stands for.
    pub(super) fn raw_at(&self, at: usize) -> usize {
        let passed = self
            .replaced
            .partition_point(|r| r.shown + REPLACEMENT_LEN <= at);
        match (self.replaced.get(passed), passed.checked_sub(1)) {
            (Some(r), _) if r.shown < at => r.raw,
            (_, Some(last)) => at - self.replaced[last].grown_after(),
            (_, None) => at,
        }
    }
}

/// The lines rg reports a match spanning, as bytes: its `text`, or its
/// base64 `bytes` when they are not valid UTF-8.
pub(super) fn reported_block(lines: &serde_json::Value) -> Option<Vec<u8>> {
    use base64::Engine as _;
    match (lines["text"].as_str(), lines["bytes"].as_str()) {
        (Some(text), _) => Some(text.as_bytes().to_vec()),
        (None, Some(encoded)) => base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .ok(),
        (None, None) => None,
    }
}

/// The reported lines without their line breaks, each decoded on its own.
pub(super) fn reported_lines(block: Option<&[u8]>) -> Option<Vec<Decoded>> {
    let block = block?;
    let block = block.strip_suffix(b"\n").unwrap_or(block);
    Some(
        block
            .split(|byte| *byte == b'\n')
            .map(|line| Decoded::new(line.strip_suffix(b"\r").unwrap_or(line)))
            .collect(),
    )
}

/// How many lines a match spans (one when rg reported none).
pub(super) fn spanned_lines(block: Option<&[u8]>) -> usize {
    let newlines = block.map_or(0, |bytes| {
        let trimmed = bytes.strip_suffix(b"\n").unwrap_or(bytes);
        trimmed.iter().filter(|b| **b == b'\n').count()
    });
    newlines + 1
}

/// Each of rg's submatches as raw byte ranges on the lines of `block` it
/// lies on (#2201). rg's offsets are into the whole block, so a multiline
/// match's ranges are split at its line breaks.
pub(super) fn line_hits(
    block: Option<&[u8]>,
    submatches: &serde_json::Value,
) -> Vec<Vec<Range<usize>>> {
    let Some(block) = block else {
        return Vec::new();
    };
    let block = block.strip_suffix(b"\n").unwrap_or(block);
    let starts: Vec<usize> = std::iter::once(0)
        .chain(
            block
                .iter()
                .enumerate()
                .filter_map(|(at, byte)| (*byte == b'\n').then_some(at + 1)),
        )
        .collect();
    let mut hits = vec![Vec::new(); starts.len()];
    let ranges = submatches
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|sub| {
            let start = usize::try_from(sub["start"].as_u64()?).ok()?;
            let end = usize::try_from(sub["end"].as_u64()?).ok()?;
            (start <= end && end <= block.len() + 1).then_some(start..end)
        });
    for range in ranges {
        // The line the submatch starts on, then each it runs into.
        let first = starts.partition_point(|line_start| *line_start <= range.start) - 1;
        for (index, line_start) in starts.iter().enumerate().skip(first) {
            match index == first || *line_start < range.end {
                true => {
                    let line_end = starts.get(index + 1).map_or(block.len(), |next| next - 1);
                    let from = range.start.max(*line_start);
                    let to = range.end.min(line_end).max(from);
                    hits[index].push(from - line_start..to - line_start);
                }
                false => break,
            }
        }
    }
    hits
}

/// A file's bytes as lines, split at `\n`, `\r\n` and `\r` (as the file
/// cache always has), each decoded on its own; no line follows a final
/// break.
pub(super) fn split_lines(bytes: &[u8]) -> Vec<Decoded> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'\n' => {
                lines.push(Decoded::new(&bytes[start..at]));
                at += 1;
                start = at;
            }
            b'\r' => {
                lines.push(Decoded::new(&bytes[start..at]));
                at += match bytes.get(at + 1) {
                    Some(b'\n') => 2,
                    Some(_) | None => 1,
                };
                start = at;
            }
            _ => at += 1,
        }
    }
    if start < bytes.len() {
        lines.push(Decoded::new(&bytes[start..]));
    }
    lines
}

#[cfg(test)]
#[path = "grep_text_tests.rs"]
mod tests;
