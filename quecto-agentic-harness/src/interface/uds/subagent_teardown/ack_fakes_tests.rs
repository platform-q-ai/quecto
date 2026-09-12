//! Test-only ACK writer fake shared by the presenter and controller tests.
use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::presenter::{AckWriteError, AckWriter};

#[derive(Default)]
pub struct RecordingWriter {
    /// Frames that were fully written and flushed, in order.
    pub frames: Mutex<Vec<String>>,
    pub attempts: AtomicUsize,
    pub fail: AtomicBool,
    /// Observed order of write vs. downstream effects, appended by tests.
    pub trace: Mutex<Vec<String>>,
}

impl RecordingWriter {
    pub fn failing() -> Self {
        let writer = Self::default();
        writer.fail.store(true, Ordering::SeqCst);
        writer
    }

    pub fn frames(&self) -> Vec<String> {
        self.frames.lock().unwrap().clone()
    }
}

impl AckWriter for RecordingWriter {
    fn write_and_flush<'a>(
        &'a self,
        line: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), AckWriteError>> + Send + 'a>> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            if self.fail.load(Ordering::SeqCst) {
                return Err(AckWriteError("broken pipe".into()));
            }
            self.frames.lock().unwrap().push(line.to_owned());
            self.trace.lock().unwrap().push("ack-flushed".into());
            Ok(())
        })
    }
}
