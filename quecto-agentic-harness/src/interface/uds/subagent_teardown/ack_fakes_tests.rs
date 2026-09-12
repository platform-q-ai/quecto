//! Test-only ACK writer fake shared by the presenter and controller tests.
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::presenter::{AckWriteError, AckWriter};
use super::wire::TeardownResponse;

/// Frames responses as legacy newline-delimited JSON lines, like the
/// production adapter would for that wire mode.
#[derive(Default)]
pub struct RecordingWriter {
    /// Frames that were fully written and flushed, in order.
    pub frames: Mutex<Vec<String>>,
    pub attempts: AtomicUsize,
    pub fail: AtomicBool,
    /// Never resolves: the write is pending forever (a stalled peer).
    pub stall: AtomicBool,
    /// Ordering trace shared with the effect fakes: "ack-flushed" is pushed
    /// here, "execute-started" by the first teardown effect.
    pub trace: Arc<Mutex<Vec<String>>>,
}

impl RecordingWriter {
    pub fn failing() -> Self {
        let writer = Self::default();
        writer.fail.store(true, Ordering::SeqCst);
        writer
    }

    pub fn stalled() -> Self {
        let writer = Self::default();
        writer.stall.store(true, Ordering::SeqCst);
        writer
    }

    pub fn frames(&self) -> Vec<String> {
        self.frames.lock().unwrap().clone()
    }
}

impl AckWriter for RecordingWriter {
    fn write_and_flush<'a>(
        &'a self,
        response: &'a TeardownResponse,
    ) -> Pin<Box<dyn Future<Output = Result<(), AckWriteError>> + Send + 'a>> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            if self.stall.load(Ordering::SeqCst) {
                std::future::pending::<()>().await;
            }
            if self.fail.load(Ordering::SeqCst) {
                return Err(AckWriteError("broken pipe".into()));
            }
            self.frames.lock().unwrap().push(response.to_line());
            self.trace.lock().unwrap().push("ack-flushed".into());
            Ok(())
        })
    }
}
