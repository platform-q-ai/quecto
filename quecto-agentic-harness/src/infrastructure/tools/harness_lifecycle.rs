//! The one lifecycle cell of a harness (#1938, epic #1929).
//!
//! The harness shutdown admits itself by freezing this state
//! (`Accepting → Frozen`, through the teardown graph's lifecycle
//! repository); the spawn tool reads it **under the subagent registry lock**
//! in the same critical section that inserts a new row. Because the fleet
//! teardown claims the direct children under that same registry lock and
//! only after the freeze, every registration is either inserted before the
//! claim (and therefore included) or observes `Frozen` and is refused: a
//! spawn can never slip past the shutdown unowned.
use std::sync::{Arc, Mutex};

use crate::domain::subagent_teardown::HarnessLifecycleState;

/// Shared between the spawn tool and the teardown graph of one harness.
pub type SharedHarnessLifecycle = Arc<Mutex<HarnessLifecycleState>>;

pub fn new_shared_harness_lifecycle() -> SharedHarnessLifecycle {
    Arc::new(Mutex::new(HarnessLifecycleState::Accepting))
}

/// Read the current state; a poisoned cell reads what it held.
pub fn current(lifecycle: &SharedHarnessLifecycle) -> HarnessLifecycleState {
    *lifecycle.lock().unwrap_or_else(|e| e.into_inner())
}

/// Why a registration was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpawnRefused {
    pub state: HarnessLifecycleState,
}

impl std::fmt::Display for SpawnRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "spawn refused: the harness is {:?} and admits no new subagent",
            self.state
        )
    }
}

/// Affirmative admission: only an `Accepting` harness registers a child.
pub fn admit_spawn(lifecycle: &SharedHarnessLifecycle) -> Result<(), SpawnRefused> {
    let state = current(lifecycle);
    if state.accepts_new_work() {
        Ok(())
    } else {
        Err(SpawnRefused { state })
    }
}

#[cfg(test)]
#[path = "harness_lifecycle_tests.rs"]
mod tests;
