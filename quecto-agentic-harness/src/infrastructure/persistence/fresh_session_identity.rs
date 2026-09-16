//! Adapter of the `FreshSessionIdentityGenerator` port (D7 #1976): the
//! impure inputs of a fresh user-chat identity. The domain owns the key
//! *shape* (`chat-<secs>-<uniq>`, [`SessionIdentity::fresh_chat`]); this
//! adapter owns the wall clock plus a uniqueness token combining the
//! process id with a per-process counter, so two launches started in the
//! same second (or two fresh sessions within one process) never collide
//! on a key. The counter is process-global: the startup identity and every
//! later fresh session of one process draw from the same sequence.
use std::sync::atomic::{AtomicU64, Ordering};

use crate::application::sessions::ports::FreshSessionIdentityGenerator;
use crate::domain::session_identity::SessionIdentity;

static SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Default, Clone, Copy)]
pub struct ProcessClockIdentityGenerator;

impl ProcessClockIdentityGenerator {
    pub fn new() -> Self {
        Self
    }
}

impl FreshSessionIdentityGenerator for ProcessClockIdentityGenerator {
    fn fresh_identity(&self) -> SessionIdentity {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        // PID disambiguates separate launches; the counter disambiguates
        // within one.
        let uniq = ((std::process::id() as u64) << 24) ^ seq;
        SessionIdentity::fresh_chat(secs, uniq)
    }
}

#[cfg(test)]
#[path = "fresh_session_identity_tests.rs"]
mod tests;
