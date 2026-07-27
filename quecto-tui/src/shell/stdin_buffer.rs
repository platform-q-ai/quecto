//! Stateful raw-terminal input decoder.
//!
//! Escape keys, marker-delimited bracketed paste, and heuristic raw paste bursts
//! have deliberately separate pending reasons. In particular, paste bytes are
//! never force-drained through the ordinary CR/LF-to-Enter key path.

/// Maximum staged input size (64 KiB). An oversized staged event is rejected as
/// a whole and reported to the caller; no partial paste is emitted.
pub(crate) const MAX_BUFFER_SIZE: usize = 64 * 1024;

pub(crate) const BRACKETED_PASTE_START: &[u8] = b"\x1b[200~";
pub(crate) const BRACKETED_PASTE_END: &[u8] = b"\x1b[201~";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingReason {
    Escape,
    Utf8,
    BracketedPaste,
    RawCandidate,
    RawPaste,
}

#[derive(Debug, PartialEq, Eq)]
pub enum InputEvent {
    KeySequence(Vec<u8>),
    Paste(String),
    Overflow,
    IncompleteBracketedPaste,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Ground,
    RawCandidate,
    RawPaste,
    BracketedPaste,
    DiscardingBracketedPaste,
}

/// Decoder for terminal stdin chunks.
pub struct StdinBuffer {
    buf: Vec<u8>,
    mode: Mode,
    /// Once raw multiline input is confirmed, quiet periods may finish editor
    /// insertion but do not turn later burst fragments into Enter events. A
    /// standalone Enter after a quiet boundary clears this latch and submits.
    raw_paste_latched: bool,
    /// Streaming prefix length for ESC[201~ while rejecting an oversized
    /// bracketed paste. This makes discard recovery partition-independent.
    discarded_end_match: usize,
    overflowed: bool,
}

impl StdinBuffer {
    pub fn new() -> Self {
        Self {
            buf: Vec::new(),
            mode: Mode::Ground,
            raw_paste_latched: false,
            discarded_end_match: 0,
            overflowed: false,
        }
    }

    /// Feed one terminal read. Returns false when the staged event would exceed
    /// the cap. In that case the entire staged event is discarded and an
    /// [`InputEvent::Overflow`] is produced, rather than silently emitting a
    /// truncated paste.
    pub fn feed(&mut self, data: &[u8]) -> bool {
        if self.buf.len().saturating_add(data.len()) > MAX_BUFFER_SIZE {
            let was_bracketed = self.mode == Mode::BracketedPaste;
            self.buf.clear();
            self.mode = if was_bracketed {
                // Keep discarding this marker-delimited event until its end
                // marker arrives; otherwise its tail could be decoded as keys.
                Mode::DiscardingBracketedPaste
            } else {
                Mode::Ground
            };
            self.raw_paste_latched = false;
            self.discarded_end_match = 0;
            self.overflowed = true;
            if was_bracketed {
                self.discard_bracketed_bytes(data);
            }
            return false;
        }

        let starts_fresh = self.buf.is_empty();
        if starts_fresh && self.mode == Mode::Ground {
            if self.raw_paste_latched && is_raw_burst_data(data) {
                self.mode = Mode::RawPaste;
            } else if is_raw_burst_data(data) {
                // Every printable/text fragment is staged until the short quiet
                // boundary. This is what permits raw paste confirmation even
                // when the terminal partitions the burst into one-byte reads.
                self.mode = Mode::RawCandidate;
            }
        }

        if self.mode == Mode::DiscardingBracketedPaste {
            self.discard_bracketed_bytes(data);
            // This entire marker-delimited event was rejected, including every
            // fragment discarded until its exact end marker.
            return false;
        } else {
            self.buf.extend_from_slice(data);
        }
        self.refresh_mode();
        true
    }

    pub fn drain_events(&mut self) -> Vec<InputEvent> {
        let mut events = Vec::new();
        if self.overflowed {
            self.overflowed = false;
            events.push(InputEvent::Overflow);
        }

        loop {
            if self.buf.is_empty() {
                break;
            }
            self.refresh_mode();
            match self.mode {
                Mode::RawCandidate | Mode::RawPaste | Mode::DiscardingBracketedPaste => break,
                Mode::BracketedPaste => {
                    if self.buf.len() < BRACKETED_PASTE_START.len() {
                        break;
                    }
                    let Some(end) = find_subsequence(
                        &self.buf[BRACKETED_PASTE_START.len()..],
                        BRACKETED_PASTE_END,
                    ) else {
                        break;
                    };
                    let content_start = BRACKETED_PASTE_START.len();
                    let content_end = content_start + end;
                    let consumed = content_end + BRACKETED_PASTE_END.len();
                    events.push(InputEvent::Paste(decode_paste(
                        &self.buf[content_start..content_end],
                    )));
                    self.buf.drain(..consumed);
                    self.mode = Mode::Ground;
                }
                Mode::Ground => match complete_sequence_len(&self.buf) {
                    SequenceStatus::Complete(len) => {
                        events.push(InputEvent::KeySequence(self.buf.drain(..len).collect()));
                    }
                    SequenceStatus::Pending(_) => break,
                },
            }
        }
        events
    }

    pub fn pending_reason(&self) -> Option<PendingReason> {
        if self.mode == Mode::DiscardingBracketedPaste {
            return Some(PendingReason::BracketedPaste);
        }
        if self.buf.is_empty() {
            return None;
        }
        Some(match self.mode {
            Mode::RawCandidate => PendingReason::RawCandidate,
            Mode::RawPaste => PendingReason::RawPaste,
            Mode::BracketedPaste if self.buf.len() < BRACKETED_PASTE_START.len() => {
                PendingReason::Escape
            }
            Mode::BracketedPaste | Mode::DiscardingBracketedPaste => PendingReason::BracketedPaste,
            Mode::Ground => match complete_sequence_len(&self.buf) {
                SequenceStatus::Pending(reason) => reason,
                SequenceStatus::Complete(_) => return None,
            },
        })
    }

    pub fn has_pending(&self) -> bool {
        self.pending_reason().is_some()
    }

    /// An Enter delivered as its own post-quiescence input action is explicit
    /// submit. The app calls this before feeding a fresh outer-loop stdin read;
    /// fragments coalesced inside the active burst never clear the latch.
    pub fn begin_input_action(&mut self, data: &[u8]) {
        if self.buf.is_empty() && self.mode == Mode::Ground && is_standalone_enter(data) {
            self.raw_paste_latched = false;
        }
    }

    /// Finish a reason-specific pending state after its own deadline/EOF.
    pub fn finish_pending(&mut self, eof: bool) -> Vec<InputEvent> {
        match self.pending_reason() {
            None => self.drain_events(),
            Some(PendingReason::RawCandidate) => {
                self.mode = if confirms_raw_multiline(&self.buf) {
                    self.raw_paste_latched = true;
                    Mode::RawPaste
                } else {
                    Mode::Ground
                };
                if self.mode == Mode::RawPaste {
                    self.finish_raw_paste()
                } else {
                    self.force_ground_bytes()
                }
            }
            Some(PendingReason::RawPaste) => self.finish_raw_paste(),
            Some(PendingReason::BracketedPaste) if eof => {
                self.buf.clear();
                let was_discarding = self.mode == Mode::DiscardingBracketedPaste;
                self.mode = Mode::Ground;
                if was_discarding {
                    Vec::new()
                } else {
                    vec![InputEvent::IncompleteBracketedPaste]
                }
            }
            Some(PendingReason::BracketedPaste) => Vec::new(),
            Some(PendingReason::Escape | PendingReason::Utf8) => self.force_ground_bytes(),
        }
    }

    fn finish_raw_paste(&mut self) -> Vec<InputEvent> {
        self.raw_paste_latched = true;
        self.mode = Mode::Ground;
        let bytes = std::mem::take(&mut self.buf);
        vec![InputEvent::Paste(decode_paste(&bytes))]
    }

    fn force_ground_bytes(&mut self) -> Vec<InputEvent> {
        self.mode = Mode::Ground;
        let bytes = std::mem::take(&mut self.buf);
        bytes
            .into_iter()
            .map(|byte| InputEvent::KeySequence(vec![byte]))
            .collect()
    }

    fn discard_bracketed_bytes(&mut self, data: &[u8]) {
        for (index, byte) in data.iter().copied().enumerate() {
            if byte == BRACKETED_PASTE_END[self.discarded_end_match] {
                self.discarded_end_match += 1;
                if self.discarded_end_match == BRACKETED_PASTE_END.len() {
                    self.mode = Mode::Ground;
                    self.discarded_end_match = 0;
                    self.buf.extend_from_slice(&data[index + 1..]);
                    return;
                }
            } else {
                self.discarded_end_match = usize::from(byte == BRACKETED_PASTE_END[0]);
            }
        }
    }

    fn refresh_mode(&mut self) {
        match self.mode {
            // A genuine non-bracketed raw paste is printable text and never
            // contains a raw ESC byte. If an ESC ever lands in a staged raw
            // candidate/paste (e.g. a key escape sequence arriving in a read
            // after the candidate was established), it is typed input, not a
            // paste: revert to Ground so the ordinary key path decodes it and
            // the ESC/CSI bytes are never emitted as literal paste text.
            Mode::RawCandidate | Mode::RawPaste if self.buf.contains(&0x1b) => {
                self.mode = Mode::Ground;
                self.raw_paste_latched = false;
            }
            Mode::RawCandidate if confirms_raw_multiline(&self.buf) => {
                self.mode = Mode::RawPaste;
                self.raw_paste_latched = true;
            }
            Mode::BracketedPaste if !is_bracketed_start_prefix(&self.buf) => {
                // ESC/CSI prefix finished as an ordinary key rather than the
                // exact bracketed-paste start marker.
                self.mode = Mode::Ground;
            }
            Mode::Ground if is_bracketed_start_prefix(&self.buf) => {
                self.mode = Mode::BracketedPaste;
            }
            _ => {}
        }
    }

    // Compatibility helpers retained for focused decoder/BDD tests. Production
    // consumes typed InputEvent values and never synthesizes paste markers.
    pub fn drain_complete(&mut self) -> Vec<Vec<u8>> {
        let mut events = self.drain_events();
        if matches!(self.pending_reason(), Some(PendingReason::RawPaste)) {
            events.extend(self.finish_pending(false));
        } else if matches!(self.pending_reason(), Some(PendingReason::RawCandidate))
            && !contains_line_break(&self.buf)
        {
            // Legacy focused tests expect ordinary coalesced text immediately;
            // production keeps the typed pending reason until its quiet timer.
            events.extend(self.finish_pending(false));
        }
        // Compatibility decoding groups a complete UTF-8 scalar into one
        // sequence even when the production typed-event path is used.
        if events.len() > 1
            && events
                .iter()
                .all(|event| matches!(event, InputEvent::KeySequence(_)))
        {
            let bytes: Vec<u8> = events
                .iter()
                .flat_map(|event| match event {
                    InputEvent::KeySequence(sequence) => sequence.iter().copied(),
                    _ => unreachable!(),
                })
                .collect();
            if std::str::from_utf8(&bytes).is_ok()
                && bytes.first().is_some_and(|byte| *byte >= 0x80)
            {
                events = vec![InputEvent::KeySequence(bytes)];
            }
        }
        events_as_sequences(events)
    }

    pub fn drain_all(&mut self) -> Vec<Vec<u8>> {
        let mut events = self.drain_events();
        events.extend(self.finish_pending(true));
        events_as_sequences(events)
    }

    /// Test-support: like [`drain_all`], but returns the typed [`InputEvent`]
    /// values so tests can assert on paste boundaries directly.
    #[cfg(test)]
    pub fn drain_all_events(&mut self) -> Vec<InputEvent> {
        let mut events = self.drain_events();
        events.extend(self.finish_pending(true));
        events
    }
}

impl Default for StdinBuffer {
    fn default() -> Self {
        Self::new()
    }
}

fn events_as_sequences(events: Vec<InputEvent>) -> Vec<Vec<u8>> {
    events
        .into_iter()
        .filter_map(|event| match event {
            InputEvent::KeySequence(sequence) => Some(sequence),
            InputEvent::Paste(text) => {
                let mut framed = Vec::with_capacity(
                    BRACKETED_PASTE_START.len() + text.len() + BRACKETED_PASTE_END.len(),
                );
                framed.extend_from_slice(BRACKETED_PASTE_START);
                framed.extend_from_slice(text.as_bytes());
                framed.extend_from_slice(BRACKETED_PASTE_END);
                Some(framed)
            }
            InputEvent::Overflow | InputEvent::IncompleteBracketedPaste => None,
        })
        .collect()
}

fn is_raw_burst_data(data: &[u8]) -> bool {
    !data.is_empty()
        && data[0] != 0x1b
        && data
            .iter()
            .all(|byte| matches!(byte, b'\r' | b'\n' | b'\t') || *byte >= 0x20 || *byte >= 0x80)
}

fn is_standalone_enter(data: &[u8]) -> bool {
    matches!(data, b"\r" | b"\n")
}

fn contains_line_break(data: &[u8]) -> bool {
    data.iter().any(|byte| matches!(byte, b'\r' | b'\n'))
}

fn confirms_raw_multiline(data: &[u8]) -> bool {
    let mut breaks = 0;
    let mut first_break_end = None;
    let mut index = 0;
    while index < data.len() {
        match data[index] {
            b'\r' => {
                index += 1;
                if data.get(index) == Some(&b'\n') {
                    index += 1;
                }
                breaks += 1;
                first_break_end.get_or_insert(index);
            }
            b'\n' => {
                index += 1;
                breaks += 1;
                first_break_end.get_or_insert(index);
            }
            _ => index += 1,
        }
    }
    breaks >= 2 || (breaks == 1 && first_break_end.is_some_and(|end| end < data.len()))
}

fn decode_paste(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn is_bracketed_start_prefix(data: &[u8]) -> bool {
    data.starts_with(BRACKETED_PASTE_START)
        || data.len() < BRACKETED_PASTE_START.len()
            && BRACKETED_PASTE_START.starts_with(data)
            && data.first() == Some(&0x1b)
            // ESC alone is generically ambiguous; once a following byte is
            // present, only the exact marker prefix belongs to paste state.
            && (data.len() > 1 || data == b"\x1b")
}

#[derive(Debug, Clone, Copy)]
enum SequenceStatus {
    Complete(usize),
    Pending(PendingReason),
}

fn complete_sequence_len(data: &[u8]) -> SequenceStatus {
    if data[0] == 0x1b {
        if is_bracketed_start_prefix(data) {
            return SequenceStatus::Pending(PendingReason::BracketedPaste);
        }
        if data.len() == 1 {
            return SequenceStatus::Pending(PendingReason::Escape);
        }
        return match data[1] {
            b'[' => {
                for (index, byte) in data.iter().enumerate().skip(2) {
                    if (0x40..=0x7e).contains(byte) {
                        return SequenceStatus::Complete(index + 1);
                    }
                }
                SequenceStatus::Pending(PendingReason::Escape)
            }
            b'O' if data.len() < 3 => SequenceStatus::Pending(PendingReason::Escape),
            b'O' => SequenceStatus::Complete(3),
            _ => SequenceStatus::Complete(2),
        };
    }

    let len = utf8_char_len(data[0]);
    if data.len() < len {
        SequenceStatus::Pending(PendingReason::Utf8)
    } else {
        SequenceStatus::Complete(len)
    }
}

fn utf8_char_len(first_byte: u8) -> usize {
    match first_byte {
        0x00..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf7 => 4,
        _ => 1,
    }
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
#[path = "stdin_buffer_tests.rs"]
mod tests;
