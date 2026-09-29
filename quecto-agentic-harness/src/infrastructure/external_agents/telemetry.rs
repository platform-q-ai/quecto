//! A member session's telemetry (#2287): each [`SessionRecord`] is one
//! structured `tracing` event under [`TELEMETRY_TARGET`], beside the
//! process adapter's own (#2286); with the event log on, each the log keeps
//! is filed there too (#2304).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use super::claude_code::process::TELEMETRY_TARGET;
use super::event_log::{MemberIdentity, audit_event};
use crate::application::audit::ports::AuditSink;
use crate::application::external_agent::dto::SessionRecord;
use crate::application::external_agent::ports::ExternalAgentTelemetry;
use crate::domain::audit::AuditEvent;

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
}

/// The most records waiting for the event log's writer; past it a record
/// is dropped, as a write failure is.
pub const EVENT_LOG_QUEUE_CAPACITY: usize = 1024;

/// Logs a member session's records, and files each the event log keeps
/// (#2304) through its [`AuditSink`] on a writer thread of its own:
/// recording never blocks the session and never fails it. A write that
/// fails (or a full queue) is warned of once; dropping the adapter writes
/// what is queued first.
pub struct EventLogExternalAgentTelemetry {
    member: MemberIdentity,
    queue: Option<std::sync::mpsc::SyncSender<(u32, AuditEvent)>>,
    writer: Option<std::thread::JoinHandle<()>>,
    health: Arc<Health>,
}

/// How the writer fares: failures counted, the first one warned of.
#[derive(Debug)]
struct Health {
    warning_due: AtomicBool,
    failures: AtomicU64,
}

impl Default for Health {
    fn default() -> Self {
        Self {
            warning_due: AtomicBool::new(true),
            failures: AtomicU64::new(0),
        }
    }
}

impl Health {
    fn failed(&self, error: &dyn std::fmt::Display) {
        self.failures.fetch_add(1, Ordering::Relaxed);
        if self.warning_due.swap(false, Ordering::Relaxed) {
            tracing::warn!(
                target: TELEMETRY_TARGET,
                %error,
                "claude member event log could not be written; the member goes on unrecorded"
            );
        }
    }
}

impl EventLogExternalAgentTelemetry {
    /// Record `member`'s session into `sink`; an error when its writer
    /// cannot be started.
    pub fn new(sink: Arc<dyn AuditSink>, member: MemberIdentity) -> std::io::Result<Self> {
        let (queue, received) = std::sync::mpsc::sync_channel(EVENT_LOG_QUEUE_CAPACITY);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let health = Arc::new(Health::default());
        let writer_health = health.clone();
        let writer = std::thread::Builder::new()
            .name("claude-member-event-log".into())
            .spawn(move || {
                for (turn, event) in received {
                    if let Err(error) = runtime.block_on(sink.emit(turn, event)) {
                        writer_health.failed(&error);
                    }
                }
            })?;
        Ok(Self {
            member,
            queue: Some(queue),
            writer: Some(writer),
            health,
        })
    }

    /// How many records could not be written.
    pub fn failures(&self) -> u64 {
        self.health.failures.load(Ordering::Relaxed)
    }
}

impl ExternalAgentTelemetry for EventLogExternalAgentTelemetry {
    fn record(&self, record: &SessionRecord) {
        TracingExternalAgentTelemetry.record(record);
        let Some((turn, event)) = audit_event(record, &self.member) else {
            return;
        };
        tracing::debug!(target: TELEMETRY_TARGET, turn, event = ?event, "external agent event log");
        let Some(queue) = &self.queue else {
            return;
        };
        match queue.try_send((turn, event)) {
            Ok(()) => {}
            Err(std::sync::mpsc::TrySendError::Full(_)) => self.health.failed(&"queue full"),
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                self.health.failed(&"writer ended");
            }
        }
    }
}

impl Drop for EventLogExternalAgentTelemetry {
    /// Close the queue and wait for the writer to file what it holds.
    fn drop(&mut self) {
        drop(self.queue.take());
        if let Some(writer) = self.writer.take() {
            let joined = writer.join();
            debug_assert!(joined.is_ok(), "the event log writer does not panic");
        }
    }
}

#[cfg(test)]
#[path = "telemetry_tests.rs"]
mod tests;
