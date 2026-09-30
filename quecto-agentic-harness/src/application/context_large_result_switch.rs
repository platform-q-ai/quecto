//! A swarm member's size-aware collapse (#2348 review M1), which the
//! composition engages once the process joins a swarm; split from
//! `context.rs` for its decrease-only line ceiling.

use super::ContextManager;
use crate::domain::large_result_collapse::LargeResultCollapse;
use std::sync::{Arc, Mutex};

/// The size-aware collapse in force (#2348 review M1): the configured one
/// (off by default outside a swarm) until the composition engages a swarm
/// member's. Shared between the composition and the context manager.
#[derive(Clone, Debug)]
pub struct LargeResultSwitch(Arc<Mutex<LargeResultCollapse>>);

impl LargeResultSwitch {
    pub fn new(dial: LargeResultCollapse) -> Self {
        Self(Arc::new(Mutex::new(dial)))
    }

    /// Put `dial` in force from now on.
    pub fn engage(&self, dial: LargeResultCollapse) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = dial;
        debug_assert_eq!(self.dial(), dial, "the engaged dial is in force");
    }

    /// The dial in force.
    pub fn dial(&self) -> LargeResultCollapse {
        *self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl ContextManager {
    /// The handle the composition engages a swarm member's rule through.
    pub fn large_result_switch(&self) -> LargeResultSwitch {
        self.large_result_collapse.clone()
    }
}
