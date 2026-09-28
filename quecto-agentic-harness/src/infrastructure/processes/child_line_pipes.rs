//! Whole-line pumps of an owned child's stdout and stdin (#2286).
//!
//! Both pumps run on the [`super::owned_child_supervisor::OwnedChildSupervisor`]'s
//! own runtime, so a runtime the caller drops can end neither the stream
//! nor the input of a child that lives on. The supervisor runs each as a
//! [`PipeTask`], which only this module builds, from a pipe and its own
//! bookkeeping: no caller code rides along. They move bytes only: what a
//! line means is the caller's business, decoded on the caller's side,
//! never on the supervisor's one worker thread.
//!
//! - **Stdout** ([`StdoutLines`]): each line is read whole up to
//!   [`LineLimits::line_cap`]; a longer one is consumed and reported by its
//!   length only ([`StdoutLine::OverCap`]). Lines wait for the reader
//!   within [`LineLimits::buffer_bytes`], counted in bytes (each line at
//!   least [`MIN_LINE_COST`]); a full budget holds the pump, and so the
//!   child's stdout pipe, until the reader takes a line. Worst case held:
//!   the budget, plus the one line the pump is reading.
//! - **Stdin** ([`StdinLines`]): a writer task owns the pipe and writes
//!   whole lines taken from a bounded queue, in order. A line is queued
//!   whole or not at all, so a cancelled send never leaves half a line.
//!   [`StdinLines::close`] sets a closed flag and closes the queue: a line
//!   the writer takes after the close is answered
//!   [`LineWriteError::Closed`] and never written, a sender waiting for
//!   queue space is answered `Closed` at once, and the writer closes the
//!   pipe once the queue is empty. The one line the writer had already
//!   taken before the close — possibly stuck on a full pipe — is written
//!   whole (never cut), and its sender learns how that write went. The
//!   close itself never waits on a write. A failed write ends the input:
//!   the failure is recorded before anyone is answered, so every sender
//!   whose line it lost is answered [`LineWriteError::Failed`], even one
//!   that looks only after a later close.

use std::sync::{Arc, Mutex, OnceLock};

use quecto_line_io::read_bounded_line_into;
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot, watch};

/// The least a buffered line counts against the budget: an entry's own
/// bookkeeping, so a flood of empty lines or over-cap reports is bounded
/// by the budget too.
pub const MIN_LINE_COST: usize = 256;

/// The stdout pump's bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineLimits {
    /// The longest line kept, in bytes (its newline included).
    pub line_cap: usize,
    /// The bytes of lines buffered ahead of the reader.
    pub buffer_bytes: usize,
}

impl LineLimits {
    /// Whether these limits can be honoured: a line fits the budget, and
    /// the budget fits one semaphore acquisition.
    pub fn valid(&self) -> bool {
        self.line_cap > 0
            && self.buffer_bytes >= self.line_cap.max(MIN_LINE_COST)
            && u32::try_from(self.buffer_bytes).is_ok()
    }
}

/// One line of a child's stdout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StdoutLine {
    /// The line's bytes as read, its newline included when it had one.
    Line(Vec<u8>),
    /// A line longer than the cap, consumed and dropped: its length on
    /// the wire, newline included.
    OverCap { bytes: usize },
}

impl StdoutLine {
    fn cost(&self, limits: LineLimits) -> u32 {
        let held = match self {
            Self::Line(bytes) => bytes.capacity(),
            Self::OverCap { .. } => 0,
        };
        let cost = held.max(MIN_LINE_COST).min(limits.buffer_bytes);
        u32::try_from(cost).expect("valid limits fit a u32")
    }
}

struct Buffered {
    line: StdoutLine,
    /// Its share of the budget, returned when the reader takes it.
    _budget: OwnedSemaphorePermit,
}

/// One pump, ready to run on the supervisor's runtime: a pipe and this
/// module's own bookkeeping, built only here. Data, not a caller's code.
pub(super) struct PipeTask(Pump);

enum Pump {
    Stdout {
        stdout: ChildStdout,
        limits: LineLimits,
        budget: Arc<Semaphore>,
        sink: mpsc::UnboundedSender<Buffered>,
    },
    Stdin {
        stdin: ChildStdin,
        lines: mpsc::Receiver<QueuedLine>,
        closed: watch::Receiver<bool>,
        failure: Arc<OnceLock<String>>,
    },
}

impl PipeTask {
    /// Pump until the pipe or its reader or writer ends.
    pub(super) async fn run(self) {
        match self.0 {
            Pump::Stdout {
                stdout,
                limits,
                budget,
                sink,
            } => read_lines(stdout, limits, budget, sink).await,
            Pump::Stdin {
                stdin,
                lines,
                closed,
                failure,
            } => write_lines(stdin, lines, closed, failure).await,
        }
    }
}

/// The lines of one child's stdout; dropping it stops the pump.
pub struct StdoutLines {
    lines: mpsc::UnboundedReceiver<Buffered>,
    pump: Option<tokio::task::AbortHandle>,
}

impl StdoutLines {
    /// The reader and the pump task that feeds it, not yet running.
    pub(super) fn new(stdout: ChildStdout, limits: LineLimits) -> (Self, PipeTask) {
        assert!(limits.valid(), "invalid stdout line limits: {limits:?}");
        let budget = Arc::new(Semaphore::new(limits.buffer_bytes));
        let (sink, lines) = mpsc::unbounded_channel();
        let task = PipeTask(Pump::Stdout {
            stdout,
            limits,
            budget,
            sink,
        });
        (Self { lines, pump: None }, task)
    }

    /// This reader, stopping `pump` when it is dropped.
    pub(super) fn running(mut self, pump: tokio::task::AbortHandle) -> Self {
        self.pump = Some(pump);
        self
    }

    /// The next line; `None` once stdout has ended (or failed, logged).
    /// Cancel-safe: a line is never lost to a dropped call.
    pub async fn next(&mut self) -> Option<StdoutLine> {
        self.lines.recv().await.map(|buffered| buffered.line)
    }
}

impl Drop for StdoutLines {
    fn drop(&mut self) {
        if let Some(pump) = &self.pump {
            pump.abort();
        }
    }
}

async fn read_lines(
    stdout: ChildStdout,
    limits: LineLimits,
    budget: Arc<Semaphore>,
    sink: mpsc::UnboundedSender<Buffered>,
) {
    let mut reader = BufReader::new(stdout);
    let mut line = Vec::new();
    loop {
        let read = match read_bounded_line_into(&mut reader, &mut line, limits.line_cap).await {
            Ok(Some(read)) => read,
            Ok(None) => return,
            Err(error) => {
                tracing::warn!(%error, "child stdout: read failed; the stream ends");
                return;
            }
        };
        let entry = if read.truncated {
            StdoutLine::OverCap {
                bytes: read.bytes_read,
            }
        } else {
            StdoutLine::Line(std::mem::take(&mut line))
        };
        let Ok(permit) = Arc::clone(&budget)
            .acquire_many_owned(entry.cost(limits))
            .await
        else {
            return;
        };
        let buffered = Buffered {
            line: entry,
            _budget: permit,
        };
        if sink.send(buffered).is_err() {
            return;
        }
    }
}

/// Why a line was not written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineWriteError {
    /// The input was closed before the writer took the line.
    Closed,
    /// Writing failed, this line's write or one before it.
    Failed(String),
}

struct QueuedLine {
    line: String,
    written: oneshot::Sender<Result<(), LineWriteError>>,
}

/// The whole-line writer of one child's stdin. Dropping it closes it.
pub struct StdinLines {
    /// The writer's queue; `None` once closed. Held only to clone or take
    /// it, never across an await.
    queue: Mutex<Option<mpsc::Sender<QueuedLine>>>,
    closed: watch::Sender<bool>,
    /// The failed write that ended the writer, recorded before any sender
    /// is answered.
    failure: Arc<OnceLock<String>>,
}

impl StdinLines {
    /// The writer and the pump task that writes for it, not yet running.
    pub(super) fn new(stdin: ChildStdin, queue: usize) -> (Self, PipeTask) {
        assert!(queue > 0, "the stdin queue holds at least one line");
        let (sender, lines) = mpsc::channel(queue);
        let (closed, closed_flag) = watch::channel(false);
        let failure = Arc::new(OnceLock::new());
        let task = PipeTask(Pump::Stdin {
            stdin,
            lines,
            closed: closed_flag,
            failure: Arc::clone(&failure),
        });
        let writer = Self {
            queue: Mutex::new(Some(sender)),
            closed,
            failure,
        };
        (writer, task)
    }

    fn queue(&self) -> std::sync::MutexGuard<'_, Option<mpsc::Sender<QueuedLine>>> {
        self.queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn is_closed(&self) -> bool {
        *self.closed.borrow()
    }

    /// Queue `line` — exactly one whole line — and wait until it is
    /// written. Cancel-safe: dropped before it is queued, nothing is
    /// sent; dropped after, the line is still written whole unless the
    /// input closes first.
    pub async fn write_line(&self, line: String) -> Result<(), LineWriteError> {
        assert!(
            line.ends_with('\n') && line.matches('\n').count() == 1,
            "the writer is given exactly one whole line"
        );
        let queue = self.queue().clone();
        let Some(queue) = queue else {
            return Err(LineWriteError::Closed);
        };
        let mut closed = self.closed.subscribe();
        let permit = tokio::select! {
            biased;
            _ = closed.wait_for(|closed| *closed) => return Err(LineWriteError::Closed),
            permit = queue.reserve_owned() => permit,
        };
        let Ok(permit) = permit else {
            return Err(self.writer_gone());
        };
        let (written, answer) = oneshot::channel();
        // The sender given back must not keep the queue open past a close.
        drop(permit.send(QueuedLine { line, written }));
        match answer.await {
            Ok(outcome) => outcome,
            Err(_) => Err(self.writer_gone()),
        }
    }

    /// The writer ended without answering. A failed write, recorded
    /// before the writer answered anyone, is what lost the line, whatever
    /// close came after it; failing none, the close did.
    fn writer_gone(&self) -> LineWriteError {
        if let Some(failure) = self.failure.get() {
            return LineWriteError::Failed(format!("a write before this line failed: {failure}"));
        }
        if self.is_closed() {
            return LineWriteError::Closed;
        }
        LineWriteError::Failed("the input's writer ended".into())
    }

    /// Close the input (see the module docs); whether this call closed it.
    /// Never waits on a write.
    pub fn close(&self) -> bool {
        let first = !self.closed.send_replace(true);
        let queue = self.queue().take();
        drop(queue);
        first
    }
}

impl Drop for StdinLines {
    fn drop(&mut self) {
        self.close();
    }
}

/// Write each queued line whole, in order, until the queue closes or a
/// write fails; then close stdin, the child's end-of-input. A line taken
/// after the close is answered `Closed`, never written. A failed write
/// ends the writer (nothing is written after a partial line), recorded in
/// `failure` before its sender, or any other, is answered.
async fn write_lines(
    mut stdin: ChildStdin,
    mut lines: mpsc::Receiver<QueuedLine>,
    closed: watch::Receiver<bool>,
    failure: Arc<OnceLock<String>>,
) {
    while let Some(QueuedLine { line, written }) = lines.recv().await {
        if *closed.borrow() {
            // A sender that stopped waiting needs no answer.
            let _ = written.send(Err(LineWriteError::Closed));
            continue;
        }
        let outcome = async {
            stdin.write_all(line.as_bytes()).await?;
            stdin.flush().await
        }
        .await;
        match outcome {
            Ok(()) => {
                let _ = written.send(Ok(()));
            }
            Err(error) => {
                tracing::warn!(%error, "child stdin: write failed; the input ends");
                let recorded = failure.set(error.to_string()).is_ok();
                assert!(recorded, "the writer ends at its first failure");
                let _ = written.send(Err(LineWriteError::Failed(error.to_string())));
                break;
            }
        }
    }
    // EOF is the close itself; a failed shutdown of a pipe whose reader
    // already left changes nothing.
    let _ = stdin.shutdown().await;
}

#[cfg(test)]
#[path = "child_line_pipes_tests.rs"]
mod tests;
