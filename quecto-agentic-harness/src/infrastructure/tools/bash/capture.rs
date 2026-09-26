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
    /// The caller has taken its output: bytes still arriving (a background
    /// job's) are drained and dropped, not kept (#2171 review).
    released: bool,
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
            released: false,
        }
    }

    pub(super) fn push(&mut self, bytes: &[u8]) {
        self.total = self.total.saturating_add(bytes.len());
        if self.released {
            return;
        }
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

    /// The kept text, and whether any of the stream was dropped. When the
    /// middle was dropped, both cuts move to a character boundary so no
    /// character is split (#2171 review); the bytes moved are counted as
    /// omitted.
    pub(super) fn render(&self) -> (String, bool) {
        debug_assert!(self.head.len() + self.tail.len() <= self.total);
        let dropped = self.total - self.head.len() - self.tail.len();
        let tail: Vec<u8> = self.tail.iter().copied().collect();
        let (head, tail) = match dropped {
            0 => (&self.head[..], &tail[..]),
            _ => (
                &self.head[..complete_prefix(&self.head)],
                &tail[leading_continuations(&tail)..],
            ),
        };
        let mut bytes = head.to_vec();
        if dropped > 0 {
            let omitted = self.total - head.len() - tail.len();
            bytes.extend_from_slice(
                format!("\n[... {omitted} bytes of output omitted ...]\n").as_bytes(),
            );
        }
        bytes.extend_from_slice(tail);
        let text = match String::from_utf8(bytes) {
            Ok(valid) => valid,
            Err(err) => String::from_utf8_lossy(err.as_bytes()).into_owned(),
        };
        (text, dropped > 0)
    }

    /// Bytes pushed so far: a reader making progress changes it.
    fn total(&self) -> usize {
        self.total
    }

    /// Keep nothing more: the caller has taken the output.
    pub(super) fn release(&mut self) {
        self.released = true;
        self.head = Vec::new();
        self.tail = VecDeque::new();
    }

    /// How many bytes the capture holds.
    #[cfg(test)]
    pub(super) fn held(&self) -> usize {
        self.head.len() + self.tail.len()
    }
}

/// The length of `bytes` without a trailing, incomplete UTF-8 sequence.
fn complete_prefix(bytes: &[u8]) -> usize {
    let len = bytes.len();
    for back in 1..=len.min(3) {
        let byte = bytes[len - back];
        if byte & 0xC0 != 0x80 {
            // A lead byte: keep it only when its whole sequence is here.
            let needed = match byte {
                b if b >= 0xF0 => 4,
                b if b >= 0xE0 => 3,
                b if b >= 0xC0 => 2,
                _ => 1,
            };
            return if needed > back { len - back } else { len };
        }
    }
    len
}

/// How many continuation bytes open `bytes` (the rest of a character cut off).
fn leading_continuations(bytes: &[u8]) -> usize {
    bytes
        .iter()
        .take(3)
        .take_while(|byte| *byte & 0xC0 == 0x80)
        .count()
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

/// One stream being read in the background. The reader drains the stream to
/// its end whatever the caller does, so a background job still writing to
/// it is never stopped by a closed pipe (#2171 review); the caller renders
/// what the capture holds when it stops waiting.
pub(super) struct StreamReader {
    source: Source,
}

enum Source {
    Pipe {
        done: tokio::task::JoinHandle<()>,
        capture: Arc<Mutex<Capture>>,
    },
    /// A task that renders its own output (tests model pipes this way).
    #[cfg(test)]
    Rendered(tokio::task::JoinHandle<(String, bool)>),
}

/// How often a caller waiting for quiet output looks again.
const QUIET_POLL: Duration = Duration::from_millis(25);

impl StreamReader {
    pub(super) fn spawn<R>(pipe: R, max_capture_bytes: usize) -> Self
    where
        R: tokio::io::AsyncRead + Unpin + Send + 'static,
    {
        let capture = Arc::new(Mutex::new(Capture::new(max_capture_bytes)));
        let shared = capture.clone();
        let done = tokio::spawn(async move { read_into(pipe, &shared).await });
        Self {
            source: Source::Pipe { done, capture },
        }
    }

    /// The stream to its end, or — once no output has arrived for `quiet`,
    /// or `ceiling` has passed — what arrived so far. The flag says the
    /// stream was still open then: something else holds it. The reader goes
    /// on draining it either way.
    pub(super) async fn finish_within(
        self,
        quiet: Duration,
        ceiling: Duration,
    ) -> ((String, bool), bool) {
        match self.source {
            Source::Pipe { mut done, capture } => {
                let started = tokio::time::Instant::now();
                let mut seen = total(&capture);
                let mut last_progress = started;
                let open = loop {
                    if tokio::time::timeout(QUIET_POLL, &mut done).await.is_ok() {
                        break false;
                    }
                    let now = tokio::time::Instant::now();
                    let latest = total(&capture);
                    if latest != seen {
                        seen = latest;
                        last_progress = now;
                    }
                    if now - last_progress >= quiet || now - started >= ceiling {
                        break true;
                    }
                };
                let mut capture = capture.lock().unwrap_or_else(|e| e.into_inner());
                let rendered = capture.render();
                // A reader still draining keeps nothing more (#2171 review).
                capture.release();
                (rendered, open)
            }
            #[cfg(test)]
            Source::Rendered(mut task) => match tokio::time::timeout(ceiling, &mut task).await {
                Ok(joined) => (joined.unwrap_or_default(), false),
                Err(_) => {
                    task.abort();
                    (Default::default(), true)
                }
            },
        }
    }
}

fn total(capture: &Mutex<Capture>) -> usize {
    capture.lock().unwrap_or_else(|e| e.into_inner()).total()
}

#[cfg(test)]
impl From<tokio::task::JoinHandle<(String, bool)>> for StreamReader {
    fn from(task: tokio::task::JoinHandle<(String, bool)>) -> Self {
        Self {
            source: Source::Rendered(task),
        }
    }
}
