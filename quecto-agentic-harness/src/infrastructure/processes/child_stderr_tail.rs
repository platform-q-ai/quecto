//! Bounded tail of a launched child's stderr (#1937 review).
//!
//! A launched child's stderr used to be discarded, so a child that refused
//! to start — `--persist` together with `--parent-control`, a malformed
//! sidecar — reported nothing but "exited before socket ready with Code(1)"
//! to its launcher. The pipe is now drained for the child's whole life on
//! the supervisor's runtime, so a short-lived launcher runtime can neither
//! fill nor close it under the child, and only the last
//! [`STDERR_TAIL_CAPACITY`] bytes are retained for the launch failure report.
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::sync::watch;

/// Bytes of stderr retained: enough for a startup refusal and its context.
pub const STDERR_TAIL_CAPACITY: usize = 4096;

/// The retained tail of one child's stderr; cheap to clone, one pump task.
#[derive(Clone, Debug)]
pub struct StderrTail {
    bytes: Arc<Mutex<VecDeque<u8>>>,
    eof: watch::Receiver<bool>,
    /// The most bytes kept, and the most a snapshot's text holds.
    capacity: usize,
}

impl StderrTail {
    /// Drain `stderr` on `runtime` until EOF, retaining the bounded tail.
    pub fn pump(runtime: &tokio::runtime::Handle, stderr: tokio::process::ChildStderr) -> Self {
        Self::pump_within(runtime, stderr, STDERR_TAIL_CAPACITY)
    }

    /// [`Self::pump`], retaining the last `capacity` bytes (at least one).
    pub fn pump_within(
        runtime: &tokio::runtime::Handle,
        mut stderr: tokio::process::ChildStderr,
        capacity: usize,
    ) -> Self {
        assert!(capacity > 0, "a stderr tail keeps at least one byte");
        let bytes = Arc::new(Mutex::new(VecDeque::with_capacity(
            capacity.min(STDERR_TAIL_CAPACITY),
        )));
        let (eof_tx, eof) = watch::channel(false);
        let sink = Arc::clone(&bytes);
        runtime.spawn(async move {
            let mut buf = [0u8; 1024];
            loop {
                match stderr.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => retain_tail(&sink, &buf[..n], capacity),
                }
            }
            // Sent before the sender drops: a waiter always observes `true`.
            let _ = eof_tx.send(true);
        });
        Self {
            bytes,
            eof,
            capacity,
        }
    }

    /// Wait, bounded by `timeout`, for the pipe to reach EOF so a report
    /// carries the child's last words. A descendant holding the pipe open
    /// cannot stall the report beyond the bound.
    pub async fn wait_eof(&self, timeout: Duration) {
        let mut eof = self.eof.clone();
        let _ = tokio::time::timeout(timeout, eof.wait_for(|done| *done)).await;
    }

    /// The retained tail as trimmed, lossy UTF-8, at most its capacity in
    /// bytes.
    pub fn snapshot(&self) -> String {
        let bytes = self.bytes.lock().unwrap_or_else(|e| e.into_inner());
        let (head, rest) = bytes.as_slices();
        let mut contiguous = Vec::with_capacity(bytes.len());
        contiguous.extend_from_slice(head);
        contiguous.extend_from_slice(rest);
        tail_text(&contiguous, self.capacity)
    }
}

/// `bytes` as trimmed, lossy UTF-8 of at most `capacity` bytes (#2286).
/// The lossy conversion turns each invalid byte into U+FFFD — three bytes
/// — so the text is cut again, from the front, on a character boundary.
fn tail_text(bytes: &[u8], capacity: usize) -> String {
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim();
    let mut start = text.len().saturating_sub(capacity);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    let tail = text[start..].trim_start();
    assert!(tail.len() <= capacity, "the tail's text fits its capacity");
    tail.to_string()
}

fn retain_tail(sink: &Mutex<VecDeque<u8>>, chunk: &[u8], capacity: usize) {
    let mut bytes = sink.lock().unwrap_or_else(|e| e.into_inner());
    let start = chunk.len().saturating_sub(capacity);
    for &byte in &chunk[start..] {
        if bytes.len() == capacity {
            bytes.pop_front();
        }
        bytes.push_back(byte);
    }
    debug_assert!(bytes.len() <= capacity);
}

#[cfg(test)]
#[path = "child_stderr_tail_tests.rs"]
mod tests;
