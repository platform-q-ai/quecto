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
    vendor: Vendor,
    pub(super) terminal: bool,
    event: String,
}
impl ProtocolObserver {
    pub(super) fn new(profile: Profile) -> Self {
        Self {
            vendor: profile.vendor,
            terminal: false,
            event: String::new(),
        }
    }
    pub(super) fn observe(&mut self, line: &str, receipt: &Receipt) {
        if self.terminal {
            return;
        }
        if matches!(self.vendor, Vendor::Anthropic) {
            if let Some(event) = line.strip_prefix("event: ") {
                self.event = match event {
                    "error"
                    | "message_stop"
                    | "message_start"
                    | "message_delta"
                    | "content_block_start"
                    | "content_block_delta"
                    | "content_block_stop"
                    | "ping" => event.to_owned(),
                    _ => String::new(),
                };
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
        if matches!(self.vendor, Vendor::OpenAi | Vendor::Codex)
            && crate::infrastructure::providers::sse_end::is_done_marker(data)
        {
            self.terminal = true;
            let mut state = receipt.0.lock().unwrap();
            count_kind(&mut state, "[DONE]");
            live(&state, 0);
            state.diagnostics.terminal_event = Some(TerminalEvent::Done);
            state.diagnostics.termination = Termination::Completed;
            return;
        }
        // Parsed once: the diagnostics below and the failure check after.
        let parsed = {
            let mut state = receipt.0.lock().unwrap();
            match serde_json::from_str::<serde_json::Value>(data) {
                Ok(value) => {
                    diagnostics::typed(&mut state.diagnostics, &value);
                    let nonempty =
                        |v: &serde_json::Value| v.as_str().is_some_and(|s| !s.is_empty());
                    state.diagnostics.generated_text |= nonempty(&value["delta"]["text"])
                        || nonempty(&value["choices"][0]["delta"]["content"])
                        || (value["type"] == "response.output_text.delta"
                            && nonempty(&value["delta"]));
                    state.diagnostics.generated_tool_call |=
                        value["choices"][0]["delta"]["tool_calls"]
                            .as_array()
                            .is_some_and(|v| !v.is_empty())
                            || value["item"]["type"] == "function_call"
                            || value["content_block"]["type"] == "tool_use";
                    // OpenAI-compatible reasoning models (DeepSeek, GLM,
                    // OpenRouter) stream thinking as `reasoning` or
                    // `reasoning_content` (#2151).
                    state.diagnostics.generated_thinking |= nonempty(&value["delta"]["thinking"])
                        || nonempty(&value["choices"][0]["delta"]["reasoning"])
                        || nonempty(&value["choices"][0]["delta"]["reasoning_content"])
                        || (matches!(
                            value["type"].as_str(),
                            Some(
                                "response.reasoning_summary_text.delta"
                                    | "response.reasoning.summary_text.delta"
                            )
                        ) && nonempty(&value["delta"]));
                    let token = state.diagnostics.generated_text
                        || state.diagnostics.generated_tool_call
                        || state.diagnostics.generated_thinking;
                    let output = attempt_output::output_bytes(self.vendor, &value);
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
                    let kind = attempt_events::event_kind(self.vendor, &self.event, &value);
                    count_kind(&mut state, kind);
                    let event = if matches!(self.vendor, Vendor::Anthropic) {
                        self.event.as_str()
                    } else {
                        value["type"].as_str().unwrap_or("")
                    };
                    let terminal = match event {
                        _ if is_error_chunk(self.vendor, &value) => Some(TerminalEvent::Error),
                        "response.completed" => Some(TerminalEvent::ResponseCompleted),
                        "response.failed" => Some(TerminalEvent::ResponseFailed),
                        "response.incomplete" => Some(TerminalEvent::ResponseIncomplete),
                        "error" => Some(TerminalEvent::Error),
                        "message_stop" => Some(TerminalEvent::MessageStop),
                        _ => None,
                    };
                    if terminal.is_some() {
                        state.diagnostics.terminal_event = terminal;
                        state.diagnostics.termination = Termination::Completed;
                    } else if attempt_events::KNOWN_EVENTS.contains(&event)
                        || (matches!(self.vendor, Vendor::OpenAi) && value.get("choices").is_some())
                    {
                    } else {
                        state.diagnostics.unknown_events =
                            state.diagnostics.unknown_events.saturating_add(1);
                    }
                    Some(value)
                }
                Err(_) => {
                    state.diagnostics.parse_errors =
                        state.diagnostics.parse_errors.saturating_add(1);
                    live(&state, 0);
                    None
                }
            }
        };
        if matches!(self.vendor, Vendor::Anthropic) {
            // Both Anthropic parsers dispatch by event name and substitute a
            // null value for malformed JSON. Terminal dispatch must not depend
            // on successful decoding, or later ignored bytes become feedback.
            match self.event.as_str() {
                "error" => {
                    let mut state = receipt.0.lock().unwrap();
                    state.diagnostics.terminal_event = Some(TerminalEvent::Error);
                    state.diagnostics.termination = Termination::Completed;
                    drop(state);
                    receipt.typed(&parsed.clone().unwrap_or_default());
                    receipt.fail();
                    self.terminal = true;
                }
                "message_stop" => {
                    let mut state = receipt.0.lock().unwrap();
                    state.diagnostics.terminal_event = Some(TerminalEvent::MessageStop);
                    state.diagnostics.termination = Termination::Completed;
                    self.terminal = true;
                }
                _ => {}
            }
        } else {
            let Some(value) = parsed else {
                return;
            };
            let (failed, completed) = match self.vendor {
                Vendor::OpenAi => (is_error_chunk(self.vendor, &value), false),
                Vendor::Codex => (
                    matches!(
                        value["type"].as_str(),
                        Some("response.failed" | "response.incomplete" | "error")
                    ) || is_error_chunk(self.vendor, &value),
                    value["type"].as_str() == Some("response.completed"),
                ),
                Vendor::Anthropic => unreachable!("Anthropic dispatch handled above"),
            };
            if failed {
                match self.vendor {
                    // A numeric 429/529 chunk throttles too (#2155 review).
                    Vendor::OpenAi => receipt.typed_as(
                        &value,
                        is_typed_throttle(&value)
                            || super::super::attempt_profile::is_throttle_chunk(&value),
                    ),
                    _ => receipt.typed(&value),
                }
                receipt.fail();
            }
            self.terminal = failed || completed;
        }
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

/// Whether `value` is an untyped mid-stream error chunk for `vendor`'s
/// protocol: any OpenAI-compatible error chunk (#2236), or a Responses
/// chunk with no `type` carrying one (#2249 review). Anthropic's errors
/// are named by their `event:` line, never by this shape.
fn is_error_chunk(vendor: Vendor, value: &serde_json::Value) -> bool {
    match vendor {
        Vendor::OpenAi => super::super::attempt_profile::is_stream_error_chunk(value),
        Vendor::Codex => super::super::attempt_profile::is_untyped_error_chunk(value),
        Vendor::Anthropic => false,
    }
}
