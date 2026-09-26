//! Bounded capture of a command's output stream (#2167): the first quarter of
//! the cap from the start and the rest from the end, so however long the
//! output runs, the true end — a test run's summary, an exit message — is
//! kept, and the dropped middle is named. The capture is shared with the
//! caller, which can take what has arrived so far when it stops waiting.
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The bytes of one stream kept within `head_cap + tail_cap`.
pub(super) struct Capture {
    head: Vec<u8>,
    tail: VecDeque<u8>,
    head_cap: usize,
    tail_cap: usize,
    total: usize,
}

impl Capture {
    pub(super) fn new(max_capture_bytes: usize) -> Self {
        let head_cap = max_capture_bytes / 4;
        Self {
            head: Vec::new(),
            tail: VecDeque::new(),
            head_cap,
            tail_cap: max_capture_bytes - head_cap,
            total: 0,
        }
    }

    pub(super) fn push(&mut self, bytes: &[u8]) {
        self.total = self.total.saturating_add(bytes.len());
        let to_head = self
            .head_cap
            .saturating_sub(self.head.len())
            .min(bytes.len());
        self.head.extend_from_slice(&bytes[..to_head]);
        let rest = &bytes[to_head..];
        if rest.len() >= self.tail_cap {
            self.tail.clear();
            self.tail.extend(&rest[rest.len() - self.tail_cap..]);
        } else {
            let overflow = (self.tail.len() + rest.len()).saturating_sub(self.tail_cap);
            self.tail.drain(..overflow);
            self.tail.extend(rest);
        }
        debug_assert!(self.head.len() <= self.head_cap && self.tail.len() <= self.tail_cap);
    }

    /// The kept text, and whether any of the stream was dropped.
    pub(super) fn render(&self) -> (String, bool) {
        let kept = self.head.len() + self.tail.len();
        debug_assert!(kept <= self.total);
        let dropped = self.total - kept;
        let mut bytes = self.head.clone();
        if dropped > 0 {
            bytes.extend_from_slice(
                format!("\n[... {dropped} bytes of output omitted ...]\n").as_bytes(),
            );
        }
        bytes.extend(self.tail.iter());
        let text = match String::from_utf8(bytes) {
            Ok(valid) => valid,
            Err(err) => String::from_utf8_lossy(err.as_bytes()).into_owned(),
        };
        (text, dropped > 0)
    }
}

/// Read `pipe` to its end into a bounded capture.
#[cfg(test)]
pub(super) async fn read_stream_limited<R>(pipe: R, max_capture_bytes: usize) -> (String, bool)
where
    R: tokio::io::AsyncRead + Unpin,
{
    let capture = Arc::new(Mutex::new(Capture::new(max_capture_bytes)));
    read_into(pipe, &capture).await;
    let capture = capture.lock().unwrap_or_else(|e| e.into_inner());
    capture.render()
}

async fn read_into<R>(mut pipe: R, capture: &Mutex<Capture>)
where
    R: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::AsyncReadExt;
    let mut chunk = [0_u8; 8192];
    loop {
        match pipe.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => capture
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(&chunk[..n]),
        }
    }
}

/// One stream being read in the background. The task renders the capture at
/// end of stream; when the caller stops waiting first, it renders what the
/// capture holds so far.
pub(super) struct StreamReader {
    task: tokio::task::JoinHandle<(String, bool)>,
    capture: Option<Arc<Mutex<Capture>>>,
}

impl StreamReader {
    pub(super) fn spawn<R>(pipe: R, max_capture_bytes: usize) -> Self
    where
        R: tokio::io::AsyncRead + Unpin + Send + 'static,
    {
        let capture = Arc::new(Mutex::new(Capture::new(max_capture_bytes)));
        let shared = capture.clone();
        let task = tokio::spawn(async move {
            read_into(pipe, &shared).await;
            let capture = shared.lock().unwrap_or_else(|e| e.into_inner());
            capture.render()
        });
        Self {
            task,
            capture: Some(capture),
        }
    }

    /// The stream to its end.
    #[cfg(test)]
    pub(super) async fn finish(self) -> (String, bool) {
        self.task.await.unwrap_or_default()
    }

    /// The stream to its end, or what arrived within `wait`; the flag says
    /// whether the stream was still open when the wait ran out.
    pub(super) async fn finish_within(self, wait: Duration) -> ((String, bool), bool) {
        let mut task = self.task;
        match tokio::time::timeout(wait, &mut task).await {
            Ok(joined) => (joined.unwrap_or_default(), false),
            Err(_) => {
                task.abort();
                let so_far = self.capture.map_or_else(Default::default, |capture| {
                    capture.lock().unwrap_or_else(|e| e.into_inner()).render()
                });
                (so_far, true)
            }
        }
    }
}

/// A reader over a task that renders its own output (tests model pipes this way).
#[cfg(test)]
impl From<tokio::task::JoinHandle<(String, bool)>> for StreamReader {
    fn from(task: tokio::task::JoinHandle<(String, bool)>) -> Self {
        Self {
            task,
            capture: None,
        }
    }
}
