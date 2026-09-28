//! The member session's clock (#2287): tokio's monotonic time, so a
//! paused test runtime drives it like every other tokio timer.

use std::time::Duration;

use crate::application::external_agent::dto::AgentClockInstant;
use crate::application::external_agent::ports::{ExternalAgentClock, PortFuture};

/// Milliseconds since the clock was made, on tokio's monotonic scale.
#[derive(Debug, Clone, Copy)]
pub struct TokioExternalAgentClock {
    epoch: tokio::time::Instant,
}

impl TokioExternalAgentClock {
    pub fn new() -> Self {
        Self {
            epoch: tokio::time::Instant::now(),
        }
    }
}

impl Default for TokioExternalAgentClock {
    fn default() -> Self {
        Self::new()
    }
}

impl ExternalAgentClock for TokioExternalAgentClock {
    fn now(&self) -> AgentClockInstant {
        let _ = self.epoch;
        AgentClockInstant(0)
    }

    fn sleep(&self, duration: Duration) -> PortFuture<'_, ()> {
        Box::pin(tokio::time::sleep(duration))
    }
}
