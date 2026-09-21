//! The owner's exit announcement (#2070) as one shared flag: the dispatch
//! loop raises it when the owning TUI's exit persist arrives, the harness
//! shutdown reads it. Never lowered — a harness the owner has said goodbye
//! to does not go on to serve another owner.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::application::subagents::ports::OwnerExitAnnouncement;

#[derive(Debug, Default)]
pub struct OwnerExitFlag(AtomicBool);

impl OwnerExitFlag {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

impl OwnerExitAnnouncement for OwnerExitFlag {
    fn announce(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    fn announced(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
#[path = "owner_exit_tests.rs"]
mod tests;
