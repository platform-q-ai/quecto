//! `AttemptAdmission` over an authority connection: acquire waits for the
//! grant, dropping the wait cancels at the authority (a raced grant is then
//! completed as failed), and the permit reports feedback/completion through
//! the same bound connection.
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::sync::Notify;

use super::client::{ClientError, Inner};
use super::protocol::{Body, Op};
use crate::application::ports::{AttemptAcquisition, AttemptAdmission, AttemptPermit};
use crate::domain::error::DomainError;
use crate::domain::inference_admission::{Feedback, ThrottleFeedback};

#[derive(Debug)]
pub struct RemoteAdmission {
    inner: Arc<Inner>,
    alias: String,
    max_cooldown_ms: u64,
}

impl RemoteAdmission {
    pub(super) fn new(inner: Arc<Inner>, alias: String, max_cooldown_ms: u64) -> Self {
        Self {
            inner,
            alias,
            max_cooldown_ms,
        }
    }
}

/// Cancels a still-pending acquire when its future is dropped mid-wait.
struct CancelOnDrop {
    inner: Arc<Inner>,
    sequence: u64,
    armed: bool,
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if self.armed {
            self.inner.cancel_detached(self.sequence);
        }
    }
}

impl AttemptAdmission for RemoteAdmission {
    fn acquire(&self) -> AttemptAcquisition<'_> {
        Box::pin(async move {
            let inner = self.inner.clone();
            let notify = Arc::new(Notify::new());
            let (sequence, receiver) = inner
                .send_acquire(&self.alias, notify.clone())
                .map_err(|error| DomainError::Provider(format!("admission: {error}")))?;
            let mut guard = CancelOnDrop {
                inner: inner.clone(),
                sequence,
                armed: true,
            };
            match Inner::await_acquire(receiver).await {
                Ok(Body::Granted {
                    deadline_ms: _,
                    receipt_ms,
                    receipt_wall_ms,
                }) => {
                    guard.armed = false;
                    Ok(Box::new(RemotePermit {
                        inner,
                        sequence,
                        report: 0,
                        notify,
                        receipt_ms,
                        receipt_wall_ms,
                        max_cooldown_ms: self.max_cooldown_ms,
                    }) as Box<dyn AttemptPermit>)
                }
                Ok(other) => {
                    guard.armed = false;
                    inner.notices.lock().expect("notice map").remove(&sequence);
                    Err(DomainError::Provider(format!(
                        "admission: unexpected reply {other:?}"
                    )))
                }
                Err(error) => {
                    // The authority already answered terminally; nothing to cancel.
                    guard.armed = matches!(error, ClientError::Closed);
                    inner.notices.lock().expect("notice map").remove(&sequence);
                    Err(DomainError::Provider(format!("admission: {error}")))
                }
            }
        })
    }
}

#[derive(Debug)]
pub struct RemotePermit {
    inner: Arc<Inner>,
    sequence: u64,
    report: u64,
    notify: Arc<Notify>,
    receipt_ms: u64,
    receipt_wall_ms: u64,
    max_cooldown_ms: u64,
}

impl AttemptPermit for RemotePermit {
    fn deadline_expired(&self) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        let notify = self.notify.clone();
        let inner = self.inner.clone();
        Box::pin(async move {
            // Register interest before checking the flag so a close between the
            // check and the await cannot be missed.
            let notified = notify.notified();
            let closed = inner.closed_notify.notified();
            tokio::pin!(notified, closed);
            notified.as_mut().enable();
            closed.as_mut().enable();
            if inner.closed.load(std::sync::atomic::Ordering::Acquire) {
                return;
            }
            tokio::select! {
                _ = &mut notified => {}
                _ = &mut closed => {}
            }
        })
    }

    fn receipt_clock(&self) -> (u64, SystemTime) {
        (
            self.receipt_ms,
            UNIX_EPOCH + Duration::from_millis(self.receipt_wall_ms),
        )
    }

    fn maximum_cooldown_ms(&self) -> u64 {
        self.max_cooldown_ms
    }

    fn feedback(&mut self, feedback: ThrottleFeedback) {
        self.report += 1;
        self.inner.send_detached(Op::Feedback {
            sequence: self.sequence,
            report: self.report,
            feedback: feedback.into(),
        });
    }

    /// The release is acknowledged only when the authority reports it durable;
    /// a non-durable reply is retried with bounded backoff on the connection's
    /// own runtime, and a process shutdown drains it.
    fn finish(self: Box<Self>, feedback: Feedback) {
        self.inner.spawn_completion(self.sequence, feedback);
    }
}
