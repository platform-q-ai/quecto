//! Transport-neutral admission capability bound to a trusted scope and quota alias.
use crate::domain::error::DomainError;
use crate::domain::inference_admission::{Feedback, ThrottleFeedback};
use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;

pub type AttemptAcquisition<'a> =
    Pin<Box<dyn Future<Output = Result<Box<dyn AttemptPermit>, DomainError>> + Send + 'a>>;

pub trait AttemptAdmission: Debug + Send + Sync {
    fn acquire(&self) -> AttemptAcquisition<'_>;
}

/// Drop alone is not release evidence. The transport owner acknowledges finish.
pub trait AttemptPermit: Debug + Send {
    /// Readiness requests local transport termination, never implies release.
    fn deadline_expired(&self) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        Box::pin(std::future::pending())
    }
    /// Capture one authority monotonic/wall receipt pair for header normalization.
    /// Defaults support observation-only test capabilities, never host activation.
    fn receipt_clock(&self) -> (u64, std::time::SystemTime) {
        (0, std::time::SystemTime::now())
    }
    fn maximum_cooldown_ms(&self) -> u64 {
        86_400_000
    }
    /// A typed throttle with no usable header. Concrete shared authorities own
    /// escalation/jitter; a leaf must not keep its own consecutive counter.
    fn throttle_without_hint(&mut self) {
        self.feedback(ThrottleFeedback::NoHint { jitter: u64::MAX });
    }
    fn feedback(&mut self, feedback: ThrottleFeedback);
    fn finish(self: Box<Self>, feedback: Feedback);
}
