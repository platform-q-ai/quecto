//! Where the agent `board` tool's calls are recorded (epic #2481 T3): the
//! session's event log, through the recording composition built for it. A
//! session binds its recording when its log opens, a session switch
//! rebuilds it into the arriving session's log, and a session that keeps
//! no event log binds nothing, so its calls record nothing. Each call
//! takes the recording bound when it starts. This slot holds a recording
//! only, never a log or a configuration: the session's front end builds
//! the recording, as composition's builder builds it.
use std::sync::{Arc, OnceLock, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::application::project_board::telemetry::BoardRecording;

/// The board tool's recording, bound to the current session's event log.
#[derive(Default)]
pub struct SessionRecording {
    bound: RwLock<Option<Arc<BoardRecording>>>,
}

impl SessionRecording {
    /// The process's own: the one session the process runs at a time.
    pub fn process() -> Arc<Self> {
        static PROCESS: OnceLock<Arc<SessionRecording>> = OnceLock::new();
        Arc::clone(PROCESS.get_or_init(Arc::default))
    }

    fn read(&self) -> RwLockReadGuard<'_, Option<Arc<BoardRecording>>> {
        self.bound.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, Option<Arc<BoardRecording>>> {
        self.bound.write().unwrap_or_else(PoisonError::into_inner)
    }

    /// Records later calls through `recording` (`None`: records nothing,
    /// the switch being off). Whether calls are now recorded.
    pub fn bind(&self, recording: Option<BoardRecording>) -> bool {
        let bound = recording.map(Arc::new);
        let recorded = bound.is_some();
        *self.write() = bound;
        recorded
    }

    /// Records nothing from now on: the session keeps no event log.
    pub fn stop(&self) {
        *self.write() = None;
    }

    /// The session switched: while calls are recorded, the recording
    /// `rebuild` builds (into the arriving session's log, with the switch
    /// decided when the session started) replaces the bound one; while
    /// nothing is recorded, nothing is built and nothing starts. One write
    /// lock covers the check and the swap, so a concurrent [`Self::stop`]
    /// is never undone. Whether calls are now recorded.
    pub fn follow(&self, rebuild: impl FnOnce() -> Option<BoardRecording>) -> bool {
        let mut bound = self.write();
        match bound.is_some() {
            true => {
                *bound = rebuild().map(Arc::new);
                bound.is_some()
            }
            false => false,
        }
    }

    /// The recording a call starting now writes through, if any.
    pub fn current(&self) -> Option<Arc<BoardRecording>> {
        self.read().clone()
    }
}

#[cfg(test)]
#[path = "session_recording_tests.rs"]
mod tests;
