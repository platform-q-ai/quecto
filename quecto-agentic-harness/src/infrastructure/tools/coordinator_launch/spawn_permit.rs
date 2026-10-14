//! Whether this session may spawn (S4c), as its tool registry's policy
//! says now, and its registry's guards: the registry sets both each time
//! they change, and the board's coordinator launcher reads them before
//! each launch, so `start` never launches what the session's own `spawn`
//! could not. The registry depends on this, never the other way.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use crate::application::tools::ports::ToolGuard;

/// The session's spawn permission, shared: closed, with no guards, until a
/// registry watching it says otherwise.
#[derive(Clone, Default)]
pub struct SpawnPermit {
    allowed: Arc<AtomicBool>,
    guards: Arc<RwLock<Vec<Arc<dyn ToolGuard>>>>,
}

impl std::fmt::Debug for SpawnPermit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpawnPermit")
            .field("allowed", &self.allowed())
            .finish_non_exhaustive()
    }
}

impl SpawnPermit {
    /// Whether the session's policy allows spawn now.
    pub fn allowed(&self) -> bool {
        self.allowed.load(Ordering::SeqCst)
    }

    /// The registry's guards' verdict on a `spawn` call with `arguments`,
    /// in their order: the first refusal's reason.
    ///
    /// # Errors
    /// A guard that refuses the call.
    pub fn check(&self, arguments: &str) -> Result<(), String> {
        let guards = self
            .guards
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guards
            .iter()
            .try_for_each(|guard| guard.check("spawn", arguments))
    }

    /// Set by the registry it follows: whether its policy allows spawn,
    /// and its guards.
    pub fn set(&self, allowed: bool, guards: &[Arc<dyn ToolGuard>]) {
        *self
            .guards
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = guards.to_vec();
        self.allowed.store(allowed, Ordering::SeqCst);
    }
}

#[cfg(test)]
#[path = "spawn_permit_tests.rs"]
mod tests;
