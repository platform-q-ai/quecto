//! A swarm member's pruning ceiling (#2342), split from `context.rs`.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

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
