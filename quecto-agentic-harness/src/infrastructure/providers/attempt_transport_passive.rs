//! Observing an attempt no admission gate started (#2151): a provider no
//! admission binding covers sends exactly as before, and this records the
//! same attempt facts beside it (wire status and safe headers, timings, event
//! counts, what was generated, time to first token). It never alters the
//! request, the response or the handling of either.
use super::*;

/// Observe a whole SSE response without changing the no-admission send path.
/// Each wire chunk is inspected as it arrives; the caller still parses the
/// complete response with its existing parser.
pub(in crate::infrastructure::providers) async fn assembled<T>(
    trace: Option<Arc<RequestTrace>>,
    builder: reqwest::RequestBuilder,
    profile: Profile,
    parse: impl FnOnce(&str) -> Result<T, DomainError>,
) -> Result<T, DomainError> {
    let attempt = PassiveAttempt::begin(trace, profile);
    let mut response = builder.send().await.map_err(|error| {
        if let Some(attempt) = &attempt {
            attempt.send_failed();
        }
        profile.send_error(&error)
    })?;
    if let Some(attempt) = &attempt {
        attempt.response(&response);
    }
    if response.status().as_u16() != 200 {
        let status = response.status().as_u16();
        let body = response.text().await.ok();
        if let Some(attempt) = &attempt {
            attempt.http_error(status, body.as_deref());
        }
        return Err(DomainError::Provider(format!(
            "HTTP {status} from {}: {}",
            profile.name(),
            body.unwrap_or_default()
        )));
    }
    let mut bytes = Vec::new();
    let mut observer = LineObserver {
        protocol: ProtocolObserver::new(profile),
        carry: Vec::new(),
        oversized: false,
    };
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if let Some(attempt) = &attempt {
                    observer.push(&chunk, &attempt.receipt);
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(error) => {
                if let Some(attempt) = &attempt {
                    attempt.read_failed();
                }
                return Err(DomainError::Provider(format!("failed to read response: {error}")));
            }
        }
    }
    if let Some(attempt) = &attempt {
        observer.finish(&attempt.receipt);
    }
    let result = parse(&String::from_utf8_lossy(&bytes));
    if let Some(attempt) = &attempt {
        if result.is_ok() {
            attempt.completed();
        } else {
            attempt.rejected();
        }
    }
    result
}

/// One observed attempt; recorded into its request's trace when dropped.
pub(in crate::infrastructure::providers) struct PassiveAttempt {
    receipt: Receipt,
    observer: ProtocolObserver,
    carry: Vec<u8>,
    oversized: bool,
}

impl PassiveAttempt {
    /// Observe an attempt of a request that carries a trace; `None` when it
    /// carries none (nothing would read the record).
    pub(in crate::infrastructure::providers) fn begin(
        trace: Option<Arc<RequestTrace>>,
        profile: Profile,
    ) -> Option<Self> {
        let receipt = Receipt::new(None, Some(trace?));
        Some(Self {
            receipt,
            observer: ProtocolObserver::new(profile),
            carry: Vec::new(),
            oversized: false,
        })
    }

    /// The response arrived: its status and safe headers (no throttle
    /// feedback: there is no admission to report to).
    pub(in crate::infrastructure::providers) fn response(&self, response: &reqwest::Response) {
        self.receipt.headers(response);
    }

    /// The request never reached the provider.
    pub(in crate::infrastructure::providers) fn send_failed(&self) {
        self.receipt.fail();
        self.receipt.termination(Termination::SendError);
    }

    /// The provider answered an error status: its whole body's error typed
    /// (before any cut for display, #2156 review), or `None` when the body
    /// could not be read, which ends the attempt as a read error, as it does
    /// through admission.
    pub(in crate::infrastructure::providers) fn http_error(&self, status: u16, body: Option<&str>) {
        self.receipt.http_error(status, body.unwrap_or_default());
        self.receipt.termination(match body {
            Some(_) => Termination::HttpError,
            None => Termination::ReadError,
        });
    }

    /// The reply's body could not be read.
    pub(in crate::infrastructure::providers) fn read_failed(&self) {
        self.receipt.fail();
        self.receipt.termination(Termination::ReadError);
    }

    /// Observe a complete non-stream response without retaining its content.
    pub(in crate::infrastructure::providers) fn nonstream_body(&self, body: &serde_json::Value) {
        let mut state = self.receipt.0.lock().unwrap_or_else(|e| e.into_inner());
        let diagnostics = &mut state.diagnostics;
        diagnostics::typed(diagnostics, body);
        diagnostics.stop_reason = body["stop_reason"].as_str().map(|reason| match reason {
            "end_turn" => TerminalStopReason::EndTurn,
            "max_tokens" => TerminalStopReason::MaxTokens,
            "tool_use" => TerminalStopReason::ToolUse,
            "refusal" => TerminalStopReason::Refusal,
            _ => TerminalStopReason::Unknown,
        });
        if let Some(blocks) = body["content"].as_array() {
            for block in blocks {
                match block["type"].as_str() {
                    Some("text") => diagnostics.generated_text |= block["text"].as_str().is_some_and(|s| !s.is_empty()),
                    Some("tool_use") => diagnostics.generated_tool_call = true,
                    Some("thinking") | Some("redacted_thinking") => diagnostics.generated_thinking = true,
                    _ => {}
                }
            }
        }
        if diagnostics.generated_text || diagnostics.generated_tool_call || diagnostics.generated_thinking {
            let elapsed = state.started.elapsed().as_millis();
            state.diagnostics.first_token_ms = Some(u64::try_from(elapsed).unwrap_or(u64::MAX));
            if let Some(trace) = &state.trace {
                trace.mark_first_token(std::time::Instant::now());
            }
        }
    }

    /// A whole reply was read but could not be accepted.
    pub(in crate::infrastructure::providers) fn rejected(&self) {
        self.receipt.fail();
        self.receipt.termination(Termination::Rejected);
    }

    /// A whole reply was read and accepted.
    pub(in crate::infrastructure::providers) fn completed(&self) {
        self.receipt.termination(Termination::Completed);
    }

    /// The body ended; unless a terminal event already ended the stream, it
    /// ended at end of file.
    fn eof(&self) {
        let mut state = self.receipt.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.diagnostics.termination == Termination::Dropped {
            state.diagnostics.termination = Termination::Eof;
        }
    }

    /// Inspect complete SSE lines as each network chunk arrives.
    pub(in crate::infrastructure::providers) fn chunk(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            if byte == b'\n' {
                self.finish_line();
            } else if !self.oversized {
                if self.carry.len() < super::super::sse_common::MAX_SSE_LINE_BYTES {
                    self.carry.push(byte);
                } else {
                    self.carry.clear();
                    self.oversized = true;
                }
            }
        }
    }

    pub(in crate::infrastructure::providers) fn finish_line(&mut self) {
        if self.oversized {
            let mut state = self.receipt.0.lock().unwrap_or_else(|e| e.into_inner());
            state.diagnostics.oversized_lines = state.diagnostics.oversized_lines.saturating_add(1);
        } else if let Ok(line) = std::str::from_utf8(&self.carry) {
            self.observer.observe(line.trim(), &self.receipt);
        }
        self.carry.clear();
        self.oversized = false;
    }

    /// One SSE line of the response body.
    pub(in crate::infrastructure::providers) fn line(&mut self, line: &str) {
        self.observer.observe(line, &self.receipt);
    }
}

impl Drop for PassiveAttempt {
    fn drop(&mut self) {
        let mut state = self.receipt.0.lock().unwrap_or_else(|e| e.into_inner());
        Receipt::record(&mut state);
    }
}

/// An SSE handler observed line by line; the inner handler sees every line
/// unchanged and decides everything.
pub(in crate::infrastructure::providers) struct Observed<H> {
    pub inner: H,
    pub attempt: Option<PassiveAttempt>,
}

impl<H: SseHandler> SseHandler for Observed<H> {
    async fn process_line(
        &mut self,
        line: &str,
        tx: &tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> SseLineOutcome {
        if let Some(attempt) = &mut self.attempt {
            attempt.line(line);
        }
        let outcome = self.inner.process_line(line, tx).await;
        if let (SseLineOutcome::Done, Some(attempt)) = (&outcome, &self.attempt) {
            attempt.receipt.refused();
        }
        outcome
    }

    async fn on_eof(&mut self, tx: &tokio::sync::mpsc::Sender<StreamEvent>) {
        if let Some(attempt) = &self.attempt {
            attempt.eof();
        }
        self.inner.on_eof(tx).await
    }
}

/// Pump a response through `inner`, observing the attempt when there is
/// one. An observed stream goes through the instrumented pump, which sends
/// the same events and also records a read failure or an oversized line as
/// how the attempt ended (#2156 review).
pub(in crate::infrastructure::providers) async fn pump_observed<H: SseHandler>(
    response: &mut reqwest::Response,
    tx: &tokio::sync::mpsc::Sender<StreamEvent>,
    inner: H,
    attempt: Option<PassiveAttempt>,
) {
    match attempt {
        Some(attempt) => {
            let receipt = attempt.receipt.clone();
            let mut handler = Observed {
                inner,
                attempt: Some(attempt),
            };
            super::diagnostic_sse::pump_sse(&receipt, response, tx, &mut handler).await;
        }
        None => {
            let mut inner = inner;
            super::super::sse_common::pump_sse(response, tx, &mut inner).await;
        }
    }
}
