//! Observing an attempt no admission gate started (#2151): a provider no
//! admission binding covers sends exactly as before, and this records the
//! same attempt facts beside it (wire status and safe headers, timings, event
//! counts, what was generated, time to first token). It never alters the
//! request, the response or the handling of either.
use super::*;

/// One observed attempt; recorded into its request's trace when dropped.
pub(in crate::infrastructure::providers) struct PassiveAttempt {
    receipt: Receipt,
    observer: ProtocolObserver,
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

    /// The provider answered an error status; its body's error typed.
    pub(in crate::infrastructure::providers) fn http_error(&self, status: u16, body: &str) {
        self.receipt.http_error(status, body);
        self.receipt.termination(Termination::HttpError);
    }

    /// The reply's body could not be read.
    pub(in crate::infrastructure::providers) fn read_failed(&self) {
        self.receipt.fail();
        self.receipt.termination(Termination::ReadError);
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
        self.inner.process_line(line, tx).await
    }

    async fn on_eof(&mut self, tx: &tokio::sync::mpsc::Sender<StreamEvent>) {
        if let Some(attempt) = &self.attempt {
            attempt.eof();
        }
        self.inner.on_eof(tx).await
    }
}
