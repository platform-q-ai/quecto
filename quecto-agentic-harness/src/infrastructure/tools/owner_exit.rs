//! The owner's exit announcement (#2070) as one shared cell: the reader task
//! of the announcing connection raises it, the harness shutdown reads it,
//! and the same connection's close withdraws it — so a raised announcement
//! never outlives the client that made it.

use std::sync::{Arc, Mutex};

use crate::application::subagents::ports::OwnerExitAnnouncement;

#[derive(Debug, Default)]
pub struct OwnerExitFlag(Mutex<Option<u64>>);

impl OwnerExitFlag {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

impl OwnerExitAnnouncement for OwnerExitFlag {
    fn announce(&self, client: u64) {
        *self.0.lock().unwrap() = Some(client);
    }

    fn withdraw(&self, client: u64) {
        let mut held = self.0.lock().unwrap();
        if *held == Some(client) {
            *held = None;
        }
    }

    fn announced(&self) -> bool {
        self.0.lock().unwrap().is_some()
    }
}

#[cfg(test)]
#[path = "owner_exit_tests.rs"]
mod tests;
