//! What an attempt's stream shows, line by line (relocated from
//! `attempt_transport.rs` for the 750-line cap): the protocol events it
//! counts and classifies, and — #2210 — the output it streamed, which the
//! request's trace follows live and the output cap bounds.
use super::*;

pub(super) struct LineObserver {
    carry: Vec<u8>,
    oversized: bool,
    protocol: ProtocolObserver,
}
impl LineObserver {
    pub(super) fn new(profile: Profile) -> Self {
        Self {
            protocol: ProtocolObserver::new(profile),
            carry: Vec::new(),
            oversized: false,
        }
    }
    pub(super) fn push(&mut self, bytes: &[u8], receipt: &Receipt) {
        for byte in bytes {
            if *byte == b'\n' {
                self.finish(receipt);
            } else if !self.oversized {
                if self.carry.len() < super::super::sse_common::MAX_SSE_LINE_BYTES {
                    self.carry.push(*byte);
                } else {
                    self.carry.clear();
                    self.oversized = true;
                }
            }
        }
    }
    pub(super) fn finish(&mut self, receipt: &Receipt) {
        if self.oversized {
            let mut state = receipt.0.lock().unwrap();
            state.diagnostics.oversized_lines = state.diagnostics.oversized_lines.saturating_add(1);
        }
        if !self.oversized {
            if let Ok(line) = std::str::from_utf8(&self.carry) {
                // Only line ends are trimmed: an indented line is no event.
                self.protocol.observe(line.trim_end(), receipt);
            }
        }
        self.carry.clear();
        self.oversized = false;
    }
}

pub(super) struct ProtocolObserver {
    dialect: &'static dyn WireDialect,
    pub(super) terminal: bool,
    /// The name the last `event:` line gave the data after it.
    event: &'static str,
}
impl ProtocolObserver {
    pub(super) fn new(profile: Profile) -> Self {
        Self {
            dialect: profile.dialect,
            terminal: false,
            event: "",
        }
    }
    pub(super) fn observe(&mut self, line: &str, receipt: &Receipt) {
        if self.terminal {
            return;
        }
        if let Some(event) = line.strip_prefix("event: ") {
            if let Some(named) = self.dialect.event_line(event) {
                self.event = named;
                return;
            }
        }
        let Some(data) = super::super::sse_common::event_data(line) else {
            return;
        };
        {
            let mut state = receipt.0.lock().unwrap();
            state.diagnostics.event_count = state.diagnostics.event_count.saturating_add(1);
        }
        if self.dialect.ends_on_done()
            && crate::infrastructure::providers::transport::sse_end::is_done_marker(data)
        {
            self.terminal = true;
            let mut state = receipt.0.lock().unwrap();
            count_kind(&mut state, "[DONE]");
            live(&state, 0);
            state.diagnostics.terminal_event = Some(TerminalEvent::Done);
            state.diagnostics.termination = Termination::Completed;
            return;
        }
        // Parsed once: the diagnostics below and the failure check after;
        // `recorded` is whether this event was recorded as terminal.
        let (parsed, recorded) = {
            let mut state = receipt.0.lock().unwrap();
            match serde_json::from_str::<serde_json::Value>(data) {
                Ok(value) => {
                    diagnostics::typed(&mut state.diagnostics, &value);
                    let generated = self.dialect.generated(&value);
                    state.diagnostics.generated_text |= generated.text;
                    state.diagnostics.generated_tool_call |= generated.tool_call;
                    state.diagnostics.generated_thinking |= generated.thinking;
                    let token = state.diagnostics.generated_text
                        || state.diagnostics.generated_tool_call
                        || state.diagnostics.generated_thinking;
                    let output = self.dialect.output_bytes(&value);
                    state.diagnostics.output_bytes =
                        state.diagnostics.output_bytes.saturating_add(output);
                    live(&state, output);
                    if token && state.diagnostics.first_token_ms.is_none() {
                        if let Some(trace) = &state.trace {
                            trace.mark_first_token(std::time::Instant::now());
                        }
                        let elapsed = state.started.elapsed().as_millis();
                        state.diagnostics.first_token_ms =
                            Some(u64::try_from(elapsed).unwrap_or(u64::MAX));
                    }
                    let kind = self.dialect.event_kind(self.event, &value);
                    count_kind(&mut state, kind);
                    let event = self.dialect.event_name(self.event, &value);
                    let terminal = self.dialect.terminal_event(event, &value);
                    if terminal.is_some() {
                        state.diagnostics.terminal_event = terminal;
                        state.diagnostics.termination = Termination::Completed;
                    } else if !self.dialect.known_event(event, &value) {
                        state.diagnostics.unknown_events =
                            state.diagnostics.unknown_events.saturating_add(1);
                    }
                    (Some(value), terminal.is_some())
                }
                Err(_) => {
                    state.diagnostics.parse_errors =
                        state.diagnostics.parse_errors.saturating_add(1);
                    live(&state, 0);
                    (None, false)
                }
            }
        };
        let (record, failure) = match self.dialect.ending(self.event, parsed.as_ref()) {
            Ending::Open => return,
            Ending::Failed {
                feedback,
                throttle,
                record,
            } => (record, Some((feedback, throttle))),
            Ending::Completed { record } => (record, None),
        };
        {
            let mut state = receipt.0.lock().unwrap();
            match record {
                Some(record) => {
                    state.diagnostics.terminal_event = Some(record);
                    state.diagnostics.termination = Termination::Completed;
                }
                // An ending that records nothing relies on this event having
                // been recorded as terminal when its data was read — not an
                // earlier one: the dialect's `ending` and its error chunks
                // agree with the terminal names (#2605 review).
                None => debug_assert!(
                    recorded,
                    "an ending with no record follows this event's terminal record"
                ),
            }
        }
        if let Some((feedback, throttle)) = failure {
            receipt.typed_as(&feedback, throttle);
            receipt.fail();
        }
        self.terminal = true;
    }
}

/// Count an event of type `kind` (#2433), in the attempt's record and in
/// its request's trace, which an attempt cut off in flight is recorded from.
fn count_kind(state: &mut State, kind: &'static str) {
    state.diagnostics.event_types.count(kind);
    if let Some(trace) = &state.trace {
        trace.observe_event_kind(kind);
    }
}

/// Tell the request's trace, which follows the attempt live (#2210), that
/// an event carrying `output` bytes arrived; a token when the attempt has
/// generated output by now.
fn live(state: &State, output: u64) {
    if let Some(trace) = &state.trace {
        let token = state.diagnostics.generated_text
            || state.diagnostics.generated_tool_call
            || state.diagnostics.generated_thinking;
        trace.observe_event(std::time::Instant::now(), output, token);
    }
}
