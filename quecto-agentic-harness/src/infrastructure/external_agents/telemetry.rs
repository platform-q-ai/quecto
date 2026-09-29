//! A member session's telemetry (#2287): each [`SessionRecord`] is one
//! structured `tracing` event under [`TELEMETRY_TARGET`], beside the
//! process adapter's own (#2286); with the event log on, each the log keeps
//! is filed there too (#2304).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use super::claude_code::process::TELEMETRY_TARGET;
use super::event_log::{MemberIdentity, audit_event};
use crate::application::audit::ports::AuditSink;
use crate::application::external_agent::dto::SessionRecord;
use crate::application::external_agent::ports::{ExternalAgentTelemetry, PortFuture};
use crate::domain::audit::AuditEvent;
use crate::domain::external_agent::telemetry::ExternalAgentLifecycle;

/// Logs a member session's records.
#[derive(Debug, Clone, Copy, Default)]
pub struct TracingExternalAgentTelemetry;

impl ExternalAgentTelemetry for TracingExternalAgentTelemetry {
    fn record(&self, record: &SessionRecord) {
        tracing::info!(
            target: TELEMETRY_TARGET,
            kind = record.kind(),
            record = ?record,
            "external agent session"
        );
    }

    fn finish(&self) -> PortFuture<'_, ()> {
        // `tracing` keeps nothing back.
        Box::pin(async {})
    }
}

/// The most records waiting for the event log's writer; past it a record
/// is dropped, and counted.
pub const EVENT_LOG_QUEUE_CAPACITY: usize = 1024;

/// How long [`EventLogExternalAgentTelemetry::finish_writer`] waits for its
/// writer to file what it holds: a log wedged on a stuck disk must not hold
/// up the member's exit.
pub const EVENT_LOG_DRAIN_BOUND: Duration = Duration::from_secs(5);

/// Logs a member session's records, and files each the event log keeps
/// (#2304) through its [`AuditSink`] on a writer thread of its own:
/// recording never blocks the session and never fails it. A record that
/// cannot be queued (the queue is full) or written is counted, the first
/// warned of, and the log's last record says how many were lost.
///
/// Its end is [`Self::finish_writer`] (the port's `finish`): awaited, never
/// blocking a thread, it waits at most its drain bound for the writer to
/// file what it holds, and tells a writer that panicked apart in any
/// build. **Dropping the adapter never waits**: it closes the queue and
/// leaves the writer to finish on its own thread, so what it still holds
/// is lost if the process exits first. The member's runner finishes it
/// before it exits.
pub struct EventLogExternalAgentTelemetry {
    member: MemberIdentity,
    queue: std::sync::Mutex<Option<std::sync::mpsc::SyncSender<(u32, AuditEvent)>>>,
    /// Told when the writer has filed everything, its last record
    /// included; dropped untold when it panics. Taken by the first finish.
    drained: std::sync::Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
    drain_bound: Duration,
    health: Arc<Health>,
}

/// How the writer fares: records lost, the first loss warned of.
#[derive(Debug)]
struct Health {
    warning_due: AtomicBool,
    /// Records that never reached the writer: its queue was full, or it
    /// had ended.
    dropped: AtomicU64,
    /// Records the sink could not write.
    failed: AtomicU64,
}

impl Default for Health {
    fn default() -> Self {
        Self {
            warning_due: AtomicBool::new(true),
            dropped: AtomicU64::new(0),
            failed: AtomicU64::new(0),
        }
    }
}

impl Health {
    fn dropped(&self, error: &dyn std::fmt::Display) {
        self.dropped.fetch_add(1, Ordering::Relaxed);
        self.warn(error);
    }

    fn failed(&self, error: &dyn std::fmt::Display) {
        self.failed.fetch_add(1, Ordering::Relaxed);
        self.warn(error);
    }

    fn warn(&self, error: &dyn std::fmt::Display) {
        if self.warning_due.swap(false, Ordering::Relaxed) {
            tracing::warn!(
                target: TELEMETRY_TARGET,
                %error,
                "claude member event log could not be written; the member goes on unrecorded"
            );
        }
    }

    /// The log's last record, when it lost any.
    fn loss(&self) -> Option<ExternalAgentLifecycle> {
        let dropped = self.dropped.load(Ordering::Relaxed);
        let failed = self.failed.load(Ordering::Relaxed);
        match dropped.saturating_add(failed) {
            0 => None,
            _ => Some(ExternalAgentLifecycle::LogIncomplete { dropped, failed }),
        }
    }
}

/// How the event log's writer ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriterEnd {
    /// It filed what it held.
    Drained,
    /// It panicked: what it held is lost.
    Panicked,
    /// It was still writing when the drain bound ran out: it is left
    /// behind, and what it still holds may be lost.
    TimedOut,
}

impl EventLogExternalAgentTelemetry {
    /// Record `member`'s session into `sink`; an error when its writer
    /// cannot be started.
    pub fn new(sink: Arc<dyn AuditSink>, member: MemberIdentity) -> std::io::Result<Self> {
        let (queue, received) = std::sync::mpsc::sync_channel(EVENT_LOG_QUEUE_CAPACITY);
        let (drained_tx, drained) = tokio::sync::oneshot::channel();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let health = Arc::new(Health::default());
        let writer_health = health.clone();
        let member_ref = member.member_ref.clone();
        // Detached: its end is told through `drained`, never joined.
        let _writer = std::thread::Builder::new()
            .name("claude-member-event-log".into())
            .spawn(move || {
                for (turn, event) in received {
                    if let Err(error) = runtime.block_on(sink.emit(turn, event)) {
                        writer_health.failed(&error);
                    }
                }
                // The queue is closed: nothing more can be lost but this.
                if let Some(record) = writer_health.loss() {
                    let event = AuditEvent::ExternalAgentLifecycle { member_ref, record };
                    if let Err(error) = runtime.block_on(sink.emit(0, event)) {
                        writer_health.failed(&error);
                    }
                }
                // The adapter may have stopped waiting: nobody to tell.
                let _ = drained_tx.send(());
            })?;
        Ok(Self {
            member,
            queue: std::sync::Mutex::new(Some(queue)),
            drained: std::sync::Mutex::new(Some(drained)),
            drain_bound: EVENT_LOG_DRAIN_BOUND,
            health,
        })
    }

    /// The same adapter, whose end waits at most `bound` for its writer.
    pub fn with_drain_bound(mut self, bound: Duration) -> Self {
        self.drain_bound = bound;
        self
    }

    /// How many records were lost: never queued, or not written.
    pub fn failures(&self) -> u64 {
        self.health
            .dropped
            .load(Ordering::Relaxed)
            .saturating_add(self.health.failed.load(Ordering::Relaxed))
    }

    /// Close the queue and wait, at most the drain bound, for the writer
    /// to file what it holds and the log's last record. Awaited, it blocks
    /// no thread: a runtime worker may run it. Once only: a later call
    /// finds nothing to wait for and answers [`WriterEnd::Drained`].
    pub async fn finish_writer(&self) -> WriterEnd {
        drop(self.close_queue());
        let drained = self
            .drained
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let Some(drained) = drained else {
            return WriterEnd::Drained;
        };
        let end = match tokio::time::timeout(self.drain_bound, drained).await {
            Ok(Ok(())) => WriterEnd::Drained,
            // Its sender dropped untold: the writer panicked.
            Ok(Err(_)) => WriterEnd::Panicked,
            Err(_) => WriterEnd::TimedOut,
        };
        match end {
            WriterEnd::Drained => {}
            WriterEnd::Panicked => tracing::warn!(
                target: TELEMETRY_TARGET,
                "claude member event log writer panicked; records it held are lost"
            ),
            WriterEnd::TimedOut => tracing::warn!(
                target: TELEMETRY_TARGET,
                bound_ms = self.drain_bound.as_millis() as u64,
                "claude member event log writer did not finish in time; records it held may be lost"
            ),
        }
        end
    }

    /// Take the queue's sender: once it is dropped, the writer files what
    /// it holds, then the log's last record, and ends.
    fn close_queue(&self) -> Option<std::sync::mpsc::SyncSender<(u32, AuditEvent)>> {
        self.queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }
}

impl ExternalAgentTelemetry for EventLogExternalAgentTelemetry {
    fn record(&self, record: &SessionRecord) {
        TracingExternalAgentTelemetry.record(record);
        let Some((turn, event)) = audit_event(record, &self.member) else {
            return;
        };
        tracing::debug!(target: TELEMETRY_TARGET, turn, event = ?event, "external agent event log");
        let queue = self
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(queue) = queue.as_ref() else {
            self.health.dropped(&"the log was finished");
            return;
        };
        match queue.try_send((turn, event)) {
            Ok(()) => {}
            Err(std::sync::mpsc::TrySendError::Full(_)) => self.health.dropped(&"queue full"),
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                self.health.dropped(&"writer ended");
            }
        }
    }

    fn finish(&self) -> PortFuture<'_, ()> {
        Box::pin(async move {
            // How it ended is warned of by `finish_writer` itself.
            let _ = self.finish_writer().await;
        })
    }
}

impl Drop for EventLogExternalAgentTelemetry {
    /// Never waits (it may run on a runtime worker): the queue is closed
    /// and the writer files what it holds on its own thread, best-effort.
    /// Only [`Self::finish_writer`] makes the records safe.
    fn drop(&mut self) {
        drop(self.close_queue());
    }
}

#[cfg(test)]
#[path = "telemetry_tests.rs"]
mod tests;
