//! Where the agent `board` tool's calls are recorded (epic #2481 T3): the
//! session's event log, through the recording composition builds for it
//! ([`SessionRecordingBuilder`]). A session binds its log when it opens it,
//! a session switch moves the binding to the arriving session's log, and a
//! session that keeps no event log binds nothing, so its calls record
//! nothing. Each call takes the recording bound when it starts.
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::application::project_board::telemetry::BoardRecording;
use crate::infrastructure::config::telemetry::TelemetryConfig;
use crate::infrastructure::persistence::audit_log::AuditLog;

/// Composition's builder of a session's recording: into the event log
/// given, only while the configuration given (or the owner's global file
/// under the base directory given) switches it on; `None` otherwise.
#[derive(Debug, Clone, Copy)]
pub struct SessionRecordingBuilder(
    pub fn(&TelemetryConfig, &Path, &AuditLog) -> Option<BoardRecording>,
);

/// The recording bound, and how to build it again for another session.
struct Bound {
    recording: Arc<BoardRecording>,
    build: SessionRecordingBuilder,
    telemetry: TelemetryConfig,
    base_dir: PathBuf,
}

/// The board tool's recording, bound to the current session's event log.
#[derive(Default)]
pub struct SessionRecording {
    bound: RwLock<Option<Bound>>,
}

impl SessionRecording {
    /// The process's own: the one session the process runs at a time.
    pub fn process() -> Arc<Self> {
        static PROCESS: OnceLock<Arc<SessionRecording>> = OnceLock::new();
        Arc::clone(PROCESS.get_or_init(Arc::default))
    }

    fn read(&self) -> RwLockReadGuard<'_, Option<Bound>> {
        self.bound.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, Option<Bound>> {
        self.bound.write().unwrap_or_else(PoisonError::into_inner)
    }

    /// Records later calls in `log`, the session's event log, as `build`
    /// builds the recording from `telemetry` and `base_dir`; nothing when
    /// it builds none. Whether calls are now recorded.
    pub fn bind(
        &self,
        build: SessionRecordingBuilder,
        telemetry: &TelemetryConfig,
        base_dir: &Path,
        log: &AuditLog,
    ) -> bool {
        let bound = (build.0)(telemetry, base_dir, log).map(|recording| Bound {
            recording: Arc::new(recording),
            build,
            telemetry: telemetry.clone(),
            base_dir: base_dir.to_path_buf(),
        });
        let recorded = bound.is_some();
        *self.write() = bound;
        recorded
    }

    /// Records nothing from now on: the session keeps no event log.
    pub fn stop(&self) {
        *self.write() = None;
    }

    /// The session switched to the one whose event log is `log`: calls
    /// recorded in the departing session's log are recorded in `log` from
    /// now on; a binding that records nothing still records nothing.
    /// Whether calls are now recorded.
    pub fn follow(&self, log: &AuditLog) -> bool {
        let again = self.read().as_ref().map(|bound| {
            let Bound {
                build,
                telemetry,
                base_dir,
                ..
            } = bound;
            (*build, telemetry.clone(), base_dir.clone())
        });
        match again {
            Some((build, telemetry, base_dir)) => self.bind(build, &telemetry, &base_dir, log),
            None => false,
        }
    }

    /// The recording a call starting now writes through, if any.
    pub fn current(&self) -> Option<Arc<BoardRecording>> {
        self.read()
            .as_ref()
            .map(|bound| Arc::clone(&bound.recording))
    }
}

#[cfg(test)]
#[path = "session_recording_tests.rs"]
mod tests;
