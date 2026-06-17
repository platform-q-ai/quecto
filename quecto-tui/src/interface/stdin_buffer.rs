//! StdinBuffer — buffers raw terminal input and emits complete sequences.
//!
//! Modeled on Quecto TUI's StdinBuffer. Solves the fundamental problem of escape
//! sequences arriving split across multiple reads (e.g. `\x1b` then `[A`).
//!
//! The buffer accumulates bytes and only emits complete key sequences.
//! Incomplete sequences are held until more data arrives or a timeout expires.

/// Maximum buffer size (64 KB). Prevents unbounded memory growth from
/// broken bracketed paste (start marker without end marker) or malicious input.
pub(crate) const MAX_BUFFER_SIZE: usize = 64 * 1024;

/// Check if a byte sequence starting with ESC is complete.
fn is_complete_escape(data: &[u8]) -> Completeness {
    if data.is_empty() || data[0] != 0x1b {
        return Completeness::NotEscape;
    }
    if data.len() == 1 {
        return Completeness::Incomplete;
    }

    match data[1] {
        b'[' => is_complete_csi(&data[2..]),
        b'O' => {
            // SS3: ESC O + one character
            if data.len() >= 3 {
                Completeness::Complete
            } else {
                Completeness::Incomplete
            }
        }
        // Meta key: ESC + single printable/control character
        _ => Completeness::Complete,
    }
}

/// Check if a CSI sequence (after `\x1b[`) is complete.
fn is_complete_csi(after_bracket: &[u8]) -> Completeness {
    if after_bracket.is_empty() {
        return Completeness::Incomplete;
    }

    // Bracketed paste start: \x1b[200~
    if after_bracket.starts_with(b"200~") {
        // Look for end marker: \x1b[201~
        const END_MARKER: &[u8] = b"\x1b[201~";
        if after_bracket
            .windows(END_MARKER.len())
            .any(|w| w == END_MARKER)
        {
            return Completeness::Complete;
        }
        return Completeness::Incomplete;
    }

    // CSI sequences end with a byte in 0x40-0x7E (@ through ~)
    // Parameter bytes are 0x30-0x3F (digits, semicolons, colons, etc.)
    // Intermediate bytes are 0x20-0x2F (space through /)
    for &b in after_bracket {
        if (0x40..=0x7E).contains(&b) {
            return Completeness::Complete;
        }
    }
    Completeness::Incomplete
}

#[derive(Debug, PartialEq)]
enum Completeness {
    Complete,
    Incomplete,
    NotEscape,
}

/// A buffer that accumulates stdin bytes and extracts complete key sequences.
pub struct StdinBuffer {
    buf: Vec<u8>,
}

impl StdinBuffer {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// Feed new bytes into the buffer.
    ///
    /// Data beyond [`MAX_BUFFER_SIZE`] is truncated to prevent unbounded
    /// memory growth (e.g. broken bracketed paste). Returns `true` if all
    /// data was accepted, `false` if some was dropped due to the cap.
    pub fn feed(&mut self, data: &[u8]) -> bool {
        let remaining_capacity = MAX_BUFFER_SIZE.saturating_sub(self.buf.len());
        if remaining_capacity == 0 {
            return data.is_empty();
        }
        let accept = data.len().min(remaining_capacity);
        self.buf.extend_from_slice(&data[..accept]);
        accept == data.len()
    }

    /// Extract all complete sequences from the buffer.
    ///
    /// Returns a list of byte slices, each representing one complete key event.
    /// Incomplete sequences remain in the buffer for the next `feed()` call.
    pub fn drain_complete(&mut self) -> Vec<Vec<u8>> {
        let mut sequences = Vec::new();
        let mut offset = 0;

        while offset < self.buf.len() {
            let remaining = &self.buf[offset..];

            if remaining[0] == 0x1b {
                // Escape sequence — check if complete.
                match is_complete_escape(remaining) {
                    Completeness::Complete => {
                        let len = escape_sequence_len(remaining);
                        sequences.push(remaining[..len].to_vec());
                        offset += len;
                    }
                    Completeness::Incomplete => {
                        // Keep the rest in the buffer.
                        break;
                    }
                    Completeness::NotEscape => {
                        // Shouldn't happen since we checked [0] == 0x1b.
                        sequences.push(vec![remaining[0]]);
                        offset += 1;
                    }
                }
            } else {
                // Non-escape byte — emit as a single-byte sequence.
                // For UTF-8 multi-byte characters, emit the full character.
                let len = utf8_char_len(remaining[0]);
                if offset + len <= self.buf.len() {
                    sequences.push(remaining[..len].to_vec());
                    offset += len;
                } else {
                    // Incomplete UTF-8 character — wait for more bytes.
                    break;
                }
            }
        }

        // Keep only the unprocessed remainder.
        if offset > 0 {
            self.buf = self.buf[offset..].to_vec();
        }

        sequences
    }

    /// Force-drain everything in the buffer, treating any incomplete
    /// escape as a bare Escape key followed by the remaining bytes.
    ///
    /// Call this after a timeout to avoid holding bytes forever.
    pub fn drain_all(&mut self) -> Vec<Vec<u8>> {
        if self.buf.is_empty() {
            return Vec::new();
        }

        // First try to drain complete sequences.
        let mut sequences = self.drain_complete();

        // If there's still data left (incomplete escape), force it out.
        if !self.buf.is_empty() {
            let remaining = std::mem::take(&mut self.buf);
            // Treat as individual bytes (bare escape + whatever followed).
            for &b in &remaining {
                sequences.push(vec![b]);
            }
        }

        sequences
    }

    /// Whether the buffer has incomplete data waiting for more bytes.
    pub fn has_pending(&self) -> bool {
        !self.buf.is_empty()
    }
}

impl Default for StdinBuffer {
    fn default() -> Self {
        Self::new()
    }
}

/// Calculate the length of a complete escape sequence.
fn escape_sequence_len(data: &[u8]) -> usize {
    if data.is_empty() || data[0] != 0x1b {
        return 1;
    }
    if data.len() == 1 {
        return 1;
    }

    match data[1] {
        b'[' => {
            // CSI: find the terminal byte (0x40-0x7E).
            // Special case: bracketed paste.
            if data.len() > 5 && data[2..6] == *b"200~" {
                // Find \x1b[201~ end marker.
                const END_MARKER: &[u8] = b"\x1b[201~";
                if let Some(pos) = data[6..]
                    .windows(END_MARKER.len())
                    .position(|w| w == END_MARKER)
                {
                    return 6 + pos + END_MARKER.len();
                }
                return data.len(); // Should not happen if complete.
            }
            for (i, &byte) in data.iter().enumerate().skip(2) {
                if (0x40..=0x7E).contains(&byte) {
                    return i + 1;
                }
            }
            data.len()
        }
        b'O' => {
            // SS3: ESC O + one char.
            3.min(data.len())
        }
        _ => {
            // Meta: ESC + one char.
            2.min(data.len())
        }
    }
}

/// Get the expected byte length of a UTF-8 character from its first byte.
fn utf8_char_len(first_byte: u8) -> usize {
    match first_byte {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => 1, // Invalid leading byte — treat as single byte.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_printable_char() {
        let mut buf = StdinBuffer::new();
        buf.feed(b"a");
        let seqs = buf.drain_complete();
        assert_eq!(seqs, vec![b"a".to_vec()]);
        assert!(!buf.has_pending());
    }

    #[test]
    fn multiple_printable_chars() {
        let mut buf = StdinBuffer::new();
        buf.feed(b"abc");
        let seqs = buf.drain_complete();
        assert_eq!(seqs.len(), 3);
        assert_eq!(seqs[0], b"a".to_vec());
        assert_eq!(seqs[1], b"b".to_vec());
        assert_eq!(seqs[2], b"c".to_vec());
    }

    #[test]
    fn complete_csi_in_one_read() {
        let mut buf = StdinBuffer::new();
        buf.feed(b"\x1b[A"); // Up arrow
        let seqs = buf.drain_complete();
        assert_eq!(seqs, vec![b"\x1b[A".to_vec()]);
    }

    #[test]
    fn split_csi_across_reads() {
        let mut buf = StdinBuffer::new();
        buf.feed(b"\x1b");
        let seqs = buf.drain_complete();
        assert!(seqs.is_empty(), "bare ESC should be held");
        assert!(buf.has_pending());

        buf.feed(b"[A");
        let seqs = buf.drain_complete();
        assert_eq!(seqs, vec![b"\x1b[A".to_vec()]);
        assert!(!buf.has_pending());
    }

    #[test]
    fn split_csi_three_reads() {
        let mut buf = StdinBuffer::new();
        buf.feed(b"\x1b");
        assert!(buf.drain_complete().is_empty());
        buf.feed(b"[");
        assert!(buf.drain_complete().is_empty());
        buf.feed(b"A");
        let seqs = buf.drain_complete();
        assert_eq!(seqs, vec![b"\x1b[A".to_vec()]);
    }

    #[test]
    fn bare_escape_on_timeout() {
        let mut buf = StdinBuffer::new();
        buf.feed(b"\x1b");
        assert!(buf.drain_complete().is_empty());
        // Timeout — force drain.
        let seqs = buf.drain_all();
        assert_eq!(seqs, vec![vec![0x1b]]);
        assert!(!buf.has_pending());
    }

    #[test]
    fn ctrl_d_not_swallowed() {
        let mut buf = StdinBuffer::new();
        buf.feed(b"\x04"); // Ctrl+D
        let seqs = buf.drain_complete();
        assert_eq!(seqs, vec![vec![0x04]]);
    }

    #[test]
    fn ctrl_c_not_swallowed() {
        let mut buf = StdinBuffer::new();
        buf.feed(b"\x03"); // Ctrl+C
        let seqs = buf.drain_complete();
        assert_eq!(seqs, vec![vec![0x03]]);
    }

    #[test]
    fn mixed_text_and_escape() {
        let mut buf = StdinBuffer::new();
        buf.feed(b"hello\x1b[Aworld");
        let seqs = buf.drain_complete();
        assert_eq!(seqs.len(), 11); // h, e, l, l, o, ESC[A, w, o, r, l, d
        assert_eq!(seqs[5], b"\x1b[A".to_vec()); // Up arrow
    }

    #[test]
    fn csi_with_params() {
        let mut buf = StdinBuffer::new();
        buf.feed(b"\x1b[13;2u"); // Kitty Shift+Enter
        let seqs = buf.drain_complete();
        assert_eq!(seqs, vec![b"\x1b[13;2u".to_vec()]);
    }

    #[test]
    fn kitty_press_and_release_in_one_read_split_into_sequences() {
        let mut buf = StdinBuffer::new();
        buf.feed(b"\x1b[65;1:1u\x1b[65;1:3u");
        let seqs = buf.drain_complete();
        assert_eq!(seqs.len(), 2);
        assert_eq!(seqs[0], b"\x1b[65;1:1u".to_vec());
        assert_eq!(seqs[1], b"\x1b[65;1:3u".to_vec());
        assert!(crate::interface::kitty::is_key_release(&seqs[1]));
    }

    #[test]
    fn ss3_sequence() {
        let mut buf = StdinBuffer::new();
        buf.feed(b"\x1bOA"); // SS3 Up arrow
        let seqs = buf.drain_complete();
        assert_eq!(seqs, vec![b"\x1bOA".to_vec()]);
    }

    #[test]
    fn alt_enter() {
        let mut buf = StdinBuffer::new();
        buf.feed(b"\x1b\r"); // Alt+Enter
        let seqs = buf.drain_complete();
        assert_eq!(seqs, vec![b"\x1b\r".to_vec()]);
    }

    #[test]
    fn utf8_char() {
        let mut buf = StdinBuffer::new();
        buf.feed("é".as_bytes());
        let seqs = buf.drain_complete();
        assert_eq!(seqs.len(), 1);
        assert_eq!(std::str::from_utf8(&seqs[0]).unwrap(), "é");
    }

    #[test]
    fn bracketed_paste() {
        let mut buf = StdinBuffer::new();
        buf.feed(b"\x1b[200~hello\x1b[201~");
        let seqs = buf.drain_complete();
        assert_eq!(seqs.len(), 1);
        assert!(seqs[0].starts_with(b"\x1b[200~"));
    }

    #[test]
    fn incomplete_csi_params() {
        let mut buf = StdinBuffer::new();
        buf.feed(b"\x1b[13;2");
        assert!(buf.drain_complete().is_empty());
        assert!(buf.has_pending());
        buf.feed(b"u");
        let seqs = buf.drain_complete();
        assert_eq!(seqs, vec![b"\x1b[13;2u".to_vec()]);
    }

    // --- 3-fragment CSI split regression tests (#466) ---

    /// Simulate the retry loop from app.rs (synchronous approximation).
    ///
    /// `fragments` is a list of byte slices arriving in sequence.
    /// `max_retries` is the maximum number of retry iterations.
    /// Returns all emitted sequences.
    ///
    /// Note: This does not model timing/timeouts — each fragment is assumed
    /// to arrive within the retry window. For timeout-sensitive behavior,
    /// an async integration test with tokio channels would be needed.
    fn simulate_retry_loop(fragments: &[&[u8]], max_retries: usize) -> Vec<Vec<u8>> {
        let mut buf = StdinBuffer::new();
        let mut all_sequences = Vec::new();
        let mut frag_idx = 0;

        // Feed first fragment.
        if frag_idx < fragments.len() {
            buf.feed(fragments[frag_idx]);
            frag_idx += 1;
        }

        // Drain complete sequences immediately.
        all_sequences.extend(buf.drain_complete());

        // Retry loop while pending.
        let mut retries = 0;
        while buf.has_pending() && retries < max_retries {
            retries += 1;
            if frag_idx < fragments.len() {
                // More data arrives within timeout.
                buf.feed(fragments[frag_idx]);
                frag_idx += 1;
                all_sequences.extend(buf.drain_complete());
            } else {
                // Timeout — no more data.
                break;
            }
        }

        // Force drain anything still pending after retries exhausted.
        all_sequences.extend(buf.drain_all());

        all_sequences
    }

    #[test]
    fn three_fragment_csi_with_multi_retry() {
        // 3-fragment split: ESC → [ → A
        // With max_retries=5, all 3 fragments arrive within the retry window.
        let seqs = simulate_retry_loop(&[b"\x1b", b"[", b"A"], 5);
        assert_eq!(
            seqs,
            vec![b"\x1b[A".to_vec()],
            "3-fragment CSI should be reassembled with multi-retry"
        );
    }

    #[test]
    fn three_fragment_csi_with_single_retry_fails() {
        // 3-fragment split: ESC → [ → A
        // With max_retries=1 (the old bug), only ESC + [ arrive before drain_all.
        let seqs = simulate_retry_loop(&[b"\x1b", b"[", b"A"], 1);
        // With only 1 retry: ESC is pending, retry gets "[", ESC[ is still incomplete,
        // drain_all breaks it into ESC + "[", then "A" is fed but the loop is done.
        // This should NOT produce a clean ESC[A — this test documents the bug.
        assert_ne!(
            seqs,
            vec![b"\x1b[A".to_vec()],
            "single retry should NOT reassemble 3-fragment CSI (documents the bug)"
        );
    }

    #[test]
    fn two_fragment_csi_with_multi_retry() {
        let seqs = simulate_retry_loop(&[b"\x1b", b"[A"], 5);
        assert_eq!(seqs, vec![b"\x1b[A".to_vec()]);
    }

    #[test]
    fn bare_escape_after_retry_exhaustion() {
        // Only ESC arrives, no more fragments.
        let seqs = simulate_retry_loop(&[b"\x1b"], 5);
        assert_eq!(
            seqs,
            vec![vec![0x1b]],
            "bare ESC should be emitted after retries"
        );
    }

    #[test]
    fn complete_sequence_no_retry() {
        let seqs = simulate_retry_loop(&[b"\x1b[A"], 5);
        assert_eq!(seqs, vec![b"\x1b[A".to_vec()]);
    }

    #[test]
    fn four_fragment_csi_with_params() {
        // ESC → [ → 1;5 → C (Ctrl+Right)
        let seqs = simulate_retry_loop(&[b"\x1b", b"[", b"1;5", b"C"], 5);
        assert_eq!(seqs, vec![b"\x1b[1;5C".to_vec()]);
    }

    // --- Buffer size cap tests (#467) ---

    #[test]
    fn feed_accepts_data_within_cap() {
        let mut buf = StdinBuffer::new();
        let data = vec![b'a'; 1000];
        buf.feed(&data);
        assert_eq!(buf.buf.len(), 1000);
    }

    #[test]
    fn feed_caps_at_max_size() {
        let mut buf = StdinBuffer::new();
        // Feed exactly MAX_BUFFER_SIZE bytes.
        let data = vec![b'a'; MAX_BUFFER_SIZE];
        buf.feed(&data);
        assert_eq!(buf.buf.len(), MAX_BUFFER_SIZE);
        // Feed more — should be silently dropped.
        buf.feed(b"extra");
        assert_eq!(
            buf.buf.len(),
            MAX_BUFFER_SIZE,
            "buffer should not grow beyond MAX_BUFFER_SIZE"
        );
    }

    #[test]
    fn feed_partial_accept_at_cap() {
        let mut buf = StdinBuffer::new();
        // Feed MAX_BUFFER_SIZE - 3 bytes.
        let data = vec![b'a'; MAX_BUFFER_SIZE - 3];
        buf.feed(&data);
        // Feed 10 more — only 3 should be accepted.
        buf.feed(&[b'b'; 10]);
        assert_eq!(buf.buf.len(), MAX_BUFFER_SIZE);
    }

    #[test]
    fn broken_bracketed_paste_bounded() {
        let mut buf = StdinBuffer::new();
        // Start marker without end marker.
        buf.feed(b"\x1b[200~");
        // Feed 100KB of "paste content".
        for _ in 0..200 {
            buf.feed(&[b'x'; 512]);
        }
        // Buffer should be capped.
        assert!(
            buf.buf.len() <= MAX_BUFFER_SIZE,
            "buffer should be capped: {} > {}",
            buf.buf.len(),
            MAX_BUFFER_SIZE
        );
    }

    #[test]
    fn feed_returns_false_when_truncated() {
        let mut buf = StdinBuffer::new();
        let data = vec![b'a'; MAX_BUFFER_SIZE];
        assert!(buf.feed(&data), "should accept all within cap");
        assert!(!buf.feed(b"x"), "should reject when at cap");
    }

    #[test]
    fn drain_all_works_after_cap_reached() {
        let mut buf = StdinBuffer::new();
        // Fill buffer with a broken paste (no end marker).
        buf.feed(b"\x1b[200~");
        let filler = vec![b'x'; MAX_BUFFER_SIZE];
        buf.feed(&filler);
        // drain_complete should return nothing (paste never completed).
        assert!(buf.drain_complete().is_empty());
        // drain_all should force everything out.
        let forced = buf.drain_all();
        assert!(!forced.is_empty(), "drain_all should emit buffered data");
        assert!(!buf.has_pending(), "buffer should be empty after drain_all");
    }

    #[test]
    fn paste_end_marker_found_with_windows() {
        let mut buf = StdinBuffer::new();
        // Build a proper bracketed paste: start + content + end.
        let mut paste = Vec::new();
        paste.extend_from_slice(b"\x1b[200~");
        paste.extend_from_slice(b"hello world");
        paste.extend_from_slice(b"\x1b[201~");
        buf.feed(&paste);
        let seqs = buf.drain_complete();
        assert_eq!(seqs.len(), 1, "paste should be one sequence");
        assert!(seqs[0].starts_with(b"\x1b[200~"));
    }
}
