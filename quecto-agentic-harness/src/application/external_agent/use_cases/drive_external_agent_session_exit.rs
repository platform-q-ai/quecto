//! An ended member's exit (#2304): waited for apart from the caller that
//! ended it, through the spawner port, and recorded as the member's last
//! lifecycle record. However that work ends — done, panicked, or dropped
//! with or without having run — the end is recorded (as an unknown exit
//! when the work did not get to observe one) and marked recorded, so
//! `close` and `finish` are released; and they wait at most [`EXIT_GRACE`] plus
//! [`END_RECORD_MARGIN`] even for work the spawner never runs.

use std::sync::Arc;

use super::{DriveExternalAgentSession, PendingExit};
use crate::application::external_agent::dto::{
    AgentClockInstant, END_RECORD_MARGIN, EXIT_GRACE, ExternalAgentExit, SessionRecord,
};
use crate::application::external_agent::ports::{ExternalAgentClock, ExternalAgentTelemetry};
use crate::application::external_agent::session_telemetry::wall_ms_since;

/// Records the member's end, then marks it recorded, when dropped: the
/// exit's work owns it, so it is dropped however that work ends, even
/// unpolled. Work that panics or is dropped before recording the end
/// leaves it to this guard, which records it as an unknown exit (no exit
/// observed: `clean: false`, no code, no signal) (#2304 review round 4).
struct EndRecordedOnDrop {
    recorded: Arc<tokio::sync::watch::Sender<bool>>,
    /// What records the end, with the member's start; `None` once the
    /// work has recorded it itself.
    unrecorded: Option<UnrecordedEnd>,
}

/// An end still to record: the telemetry port, the clock and the
/// member's start, for its wall time.
struct UnrecordedEnd {
    telemetry: Arc<dyn ExternalAgentTelemetry>,
    clock: Arc<dyn ExternalAgentClock>,
    started_at: Option<AgentClockInstant>,
}

impl UnrecordedEnd {
    /// Record the member's end with the exit `exit` (`None`: none observed).
    fn record(self, exit: Option<&ExternalAgentExit>) {
        let wall_ms = wall_ms_since(self.started_at, self.clock.now());
        self.telemetry.record(&ended_record(exit, wall_ms));
    }
}

impl EndRecordedOnDrop {
    /// Record the end with the exit the work observed; the guard then
    /// records nothing more.
    fn record(&mut self, exit: Option<&ExternalAgentExit>) {
        if let Some(unrecorded) = self.unrecorded.take() {
            unrecorded.record(exit);
        }
    }
}

impl Drop for EndRecordedOnDrop {
    fn drop(&mut self) {
        // The telemetry port never panics (it counts what it cannot keep),
        // so recording here is safe while a panic unwinds.
        self.record(None);
        self.recorded.send_replace(true);
    }
}

impl DriveExternalAgentSession {
    /// Record an ended member's exit, apart from the caller (through the
    /// spawner port): its input is closed and its process waited for,
    /// both within [`EXIT_GRACE`], then let go and its end recorded.
    /// Called with no write gate held, after every other record of the
    /// end.
    pub(super) fn record_exit(&self, exit: Option<PendingExit>) {
        let Some(PendingExit {
            process,
            started_at,
        }) = exit
        else {
            return;
        };
        assert!(
            self.core().ended(),
            "only an ended member's exit is recorded"
        );
        let clock = self.clock.clone();
        // Moved into the work when it is made, not made in it: dropped
        // with the work even when it never runs, recording the end then.
        let mut recorded = EndRecordedOnDrop {
            recorded: self.end_recorded.clone(),
            unrecorded: Some(UnrecordedEnd {
                telemetry: self.telemetry.clone(),
                clock: clock.clone(),
                started_at,
            }),
        };
        self.spawner.spawn(Box::pin(async move {
            let exit = match process {
                Some(process) => {
                    let exited = async {
                        process.close_input().await;
                        process.exited_discarding_output().await
                    };
                    tokio::select! {
                        biased;
                        exit = exited => Some(exit),
                        () = clock.sleep(EXIT_GRACE) => None,
                    }
                }
                None => None,
            };
            recorded.record(exit.as_ref());
            drop(recorded);
        }));
    }

    /// Resolves once the member's end is recorded (or it never started, or
    /// the exit's work ended without recording it), or once [`EXIT_GRACE`]
    /// plus [`END_RECORD_MARGIN`] has passed: never later.
    pub(super) async fn until_end_recorded(&self) {
        let mut recorded = self.end_recorded.subscribe();
        tokio::select! {
            biased;
            // The sender lives as long as `self`: never an error.
            _ = recorded.wait_for(|recorded| *recorded) => {}
            // Given up: an end recorded later may miss `finish`.
            () = self.clock.sleep(EXIT_GRACE + END_RECORD_MARGIN) => {}
        }
    }
}

/// The member's end: its process's exit (`None`: not observed within
/// [`EXIT_GRACE`]) and its wall time.
pub(super) fn ended_record(
    exit: Option<&ExternalAgentExit>,
    wall_ms: Option<u64>,
) -> SessionRecord {
    let (exit_code, signal) = match exit {
        Some(ExternalAgentExit::Code(code)) => (Some(*code), None),
        Some(ExternalAgentExit::Signal(signal)) => (None, Some(*signal)),
        Some(ExternalAgentExit::Unobservable(_)) | None => (None, None),
    };
    SessionRecord::Ended {
        clean: exit.is_some_and(ExternalAgentExit::is_clean),
        exit_code,
        signal,
        wall_ms,
    }
}
