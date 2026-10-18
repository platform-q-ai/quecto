//! The durable-prefix dirty latch of an agent loop (#1072), shared with the
//! sessions capability's save transaction (#1860, D5 #1972).
//!
//! The pruning pass latches it whenever a run mutated history that existed
//! before the run (in-place stub demotion, tool-result collapse, a physical
//! drop). It is sticky and outcome-independent — it stays set across an
//! Error or Cancelled turn — and only the save transaction consumes it, so
//! a persist that fails keeps its observation retryable.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::application::sessions::ports::DurablePrefixObservation;

#[derive(Debug, Default)]
pub struct DurablePrefixLatch(AtomicBool);

impl DurablePrefixLatch {
    pub fn shared() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Record that pre-existing history changed.
    pub fn latch(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// Read-and-clear: true when any pass since the last take mutated
    /// already-existing history.
    pub fn take(&self) -> bool {
        self.0.swap(false, Ordering::Relaxed)
    }
}

impl DurablePrefixObservation for DurablePrefixLatch {
    fn take_durable_prefix_dirty(&self) -> bool {
        self.take()
    }
}

#[cfg(test)]
#[path = "durable_prefix_tests.rs"]
mod tests;
