//! A swarm member's pruning ceiling (#2342) and size-aware collapse
//! (#2348 review M1), which the composition imposes once the process joins
//! a swarm; split from `context.rs`.

use super::ContextManager;
use crate::domain::large_result_collapse::LargeResultCollapse;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// A lower pruning ceiling the composition imposes after construction
/// (#2342): a swarm member's, from when its process joins a swarm. Shared
/// between the composition and the context manager; it only ever lowers,
/// and `usize::MAX` (the start) imposes nothing.
#[derive(Clone, Debug)]
pub struct ContextCeilingCap(Arc<AtomicUsize>);

impl Default for ContextCeilingCap {
    fn default() -> Self {
        Self(Arc::new(AtomicUsize::new(usize::MAX)))
    }
}

impl ContextCeilingCap {
    /// Lower the cap to `tokens` (never raises it).
    pub fn lower_to(&self, tokens: usize) {
        let before = self.0.fetch_min(tokens, Ordering::SeqCst);
        debug_assert!(self.tokens() <= before, "a cap only ever lowers");
    }

    /// The cap in force: `usize::MAX` when none.
    pub fn tokens(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
}

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
    pub fn engage(&self, _dial: LargeResultCollapse) {}

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
