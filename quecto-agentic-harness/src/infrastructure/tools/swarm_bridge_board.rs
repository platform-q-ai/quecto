//! The coordination board a [`super::SwarmContext`] or a
//! [`super::HostedStore`] reaches (#2278, epic #2265): composition's
//! handles builder, the handles it built for each board file recently
//! called, and the event log each call is recorded in once the session has
//! one: the current session's, followed across a switch (#2313), and from
//! admission on when the event log was decided on before it.
//!
//! Every board call is a direct call of the Rust dispatcher
//! ([`swarm_board_dispatch::call`]) on the caller's thread: the store's
//! calls are blocking (a contended transaction waits up to its 500 ms busy
//! timeout), so every caller makes it on the blocking pool or outside any
//! runtime, as it did for the Python interpreter this replaced; a debug
//! build asserts it on every call ([`SwarmBoard::call`]). Nothing here
//! constructs a repository, a clock, an id source or a use case: the
//! builder is composition's (`composition::swarm::build_swarm_board_handles`).
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde_json::Value;

use crate::application::swarm::dto::BoardLocation;
use crate::application::swarm::ports::BoardOpLog;
use crate::domain::error::DomainError;
use crate::domain::swarm::RefusalKind;
use crate::infrastructure::persistence::audit_log::AuditLog;
use crate::infrastructure::tools::swarm_board_dispatch::{
    self, BoardWire, CallOrigin, SwarmBoardHandles,
};

/// Builds the board handles over one board file, recording each call in
/// the event log given (composition's `board_op_log`, only while the
/// event log is on, #2303).
pub type SwarmBoardHandlesBuilder =
    fn(BoardLocation, Option<Arc<dyn BoardOpLog>>) -> SwarmBoardHandles;

/// The event log a session's board calls are recorded in, from its audit
/// log: composition's `board_op_log`, `None` unless the event log is on
/// (`telemetry.event_log.enabled`, owner decision T1, #2303).
pub type SwarmBoardOpLogBuilder = fn(bool, &AuditLog) -> Option<Arc<dyn BoardOpLog>>;

/// The board handles of every context that shares it: clones share the
/// built handles and the event log.
#[derive(Clone)]
pub struct SwarmBoard {
    shared: Arc<Shared>,
}

struct Shared {
    build: SwarmBoardHandlesBuilder,
    wire: BoardWire,
    session_log: Option<SwarmBoardOpLogBuilder>,
    /// The log every call is recorded in, while the board records: `None`
    /// measures and writes nothing (owner decision T1).
    recording: Mutex<Option<Arc<SessionLog>>>,
    /// The handles built per file, the most recently called last; at
    /// most [`BUILT_FILES`].
    built: Mutex<Vec<Built>>,
}

/// The handles built for one board file, recording in the session log
/// given (and folding the file's run for its summary) or not at all.
struct Built {
    location: BoardLocation,
    log: Option<Arc<SessionLog>>,
    runs: Option<Arc<RunFold>>,
    handles: Arc<SwarmBoardHandles>,
}

/// The most board files whose handles a board keeps (#2278 final review
/// nit): a context calls one file, and a host reads the few its
/// environments hold; a board called for more drops the least recently
/// called file's handles.
pub(super) const BUILT_FILES: usize = 16;

impl std::fmt::Debug for SwarmBoard {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SwarmBoard")
            .field("logged", &self.recording().is_some())
            .finish_non_exhaustive()
    }
}

impl SwarmBoard {
    /// The board over composition's `build`, reading and writing member
    /// text with composition's `wire`.
    pub fn new(build: SwarmBoardHandlesBuilder, wire: BoardWire) -> Self {
        Self {
            shared: Arc::new(Shared {
                build,
                wire,
                session_log: None,
                recording: Mutex::new(None),
                built: Mutex::new(Vec::new()),
            }),
        }
    }

    /// The board over composition's `build`, recording in a session's
    /// event log through `session_log` ([`Self::record_in_session`]).
    pub fn with_session_log(
        build: SwarmBoardHandlesBuilder,
        wire: BoardWire,
        session_log: SwarmBoardOpLogBuilder,
    ) -> Self {
        Self {
            shared: Arc::new(Shared {
                build,
                wire,
                session_log: Some(session_log),
                recording: Mutex::new(None),
                built: Mutex::new(Vec::new()),
            }),
        }
    }

    /// Measures and records every call from now on, the admission's
    /// included, when the event log was decided on before admission
    /// (`event_log_on`, #2313): the records are held until the session's
    /// log is open ([`Self::record_in_session`]), and written there first.
    /// With the event log off nothing is measured (owner decision T1).
    pub fn record_from_admission(&self, event_log_on: bool) {
        // RED stub (#2313): the admission's calls are not recorded.
        let _ = event_log_on;
    }

    /// Records every later call in the session's audit `log` when the
    /// event log is on (`enabled`) and this board was given composition's
    /// `session_log`; whether it now records there. Otherwise the board
    /// records nothing from now on, and what it held is dropped.
    pub fn record_in_session(&self, enabled: bool, log: &AuditLog) -> bool {
        match self
            .shared
            .session_log
            .and_then(|build| build(enabled, log))
        {
            Some(log) => self.record_in(log),
            None => false,
        }
    }

    /// Records nothing from now on (#2313: the session keeps no event
    /// log), and drops what it held since admission.
    pub fn stop_recording(&self) {
        // RED stub (#2313).
    }

    /// Records every later call in `log`, the current session's event log
    /// (#2313: a later session's replaces an earlier one's); what the board
    /// held until now is written there first.
    pub fn record_in(&self, log: Arc<dyn BoardOpLog>) -> bool {
        // RED stub (#2313): the first log only.
        if self.recording().is_some() {
            return false;
        }
        self.recording_or_pending().bind(log);
        true
    }

    /// The session switched to the one whose audit log is `log` (#2313):
    /// a board recording in the departing session's log records in `log`
    /// from now on; one that records nothing still does not.
    pub fn follow_session(&self, log: &AuditLog) -> bool {
        // RED stub (#2313): the records stay in the first session's log.
        let _ = log;
        false
    }

    /// Writes the summary of the run on the file at `location` (#2313),
    /// once, from the records this board folded for it: `false` when the
    /// board records nothing, folded no run there, or wrote it already.
    pub(super) fn summarize_run(&self, location: &BoardLocation) -> bool {
        let runs = self
            .built()
            .iter()
            .find(|file| file.location == *location)
            .and_then(|file| file.runs.clone());
        runs.is_some_and(|runs| runs.summarize_run())
    }

    fn recording_lock(&self) -> MutexGuard<'_, Option<Arc<SessionLog>>> {
        self.shared
            .recording
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn recording(&self) -> Option<Arc<SessionLog>> {
        self.recording_lock().clone()
    }

    /// The log the board records in, made (not yet bound) if it had none.
    fn recording_or_pending(&self) -> Arc<SessionLog> {
        self.recording_lock()
            .get_or_insert_with(|| Arc::new(SessionLog::pending()))
            .clone()
    }

    fn built(&self) -> MutexGuard<'_, Vec<Built>> {
        self.shared
            .built
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether `other` is this board (a clone of it), not merely one over
    /// the same builder.
    #[cfg(any(test, feature = "test-support"))]
    pub fn is_the_same_board(&self, other: &SwarmBoard) -> bool {
        Arc::ptr_eq(&self.shared, &other.shared)
    }

    /// One board call against the file at `location`, as `member`: the
    /// dispatcher's answer, or its refusal as the tool boundary has always
    /// carried a board error, `swarm: "<text>"` (the text as a JSON
    /// string).
    pub(super) fn call(
        &self,
        location: BoardLocation,
        member: &str,
        method: &str,
        args: Value,
    ) -> Result<Value, DomainError> {
        self.call_as(location, member, method, args, CallOrigin::Member)
    }

    /// [`Self::call`], made for `origin` (#2279 S15 final review: the
    /// harness's own reads on a member's behalf are recorded as `host`).
    pub(super) fn call_as(
        &self,
        location: BoardLocation,
        member: &str,
        method: &str,
        args: Value,
        origin: CallOrigin,
    ) -> Result<Value, DomainError> {
        // A board call blocks (a contended transaction waits up to the
        // store's busy timeout): every caller makes it on the blocking pool
        // (`call_work::spawn_blocking_in_call`) or outside any runtime,
        // never on an async worker (#2278 review L6).
        debug_assert!(
            crate::infrastructure::tools::call_work::may_block(),
            "a board call ({method}) is made off the async workers (#2278)"
        );
        let handles = self.handles(location);
        swarm_board_dispatch::call_as(&handles, member, method, args, origin).map_err(|refusal| {
            DomainError::Tool(format!(
                "swarm: {}",
                Value::String(refusal.message().to_owned())
            ))
        })
    }

    /// Records `method`, called by `member` on the file at `location`, as
    /// refused with `kind` before it reached the board (#2279: a structured
    /// op's argument text or its running gate), `elapsed` after it began.
    pub(super) fn refused(
        &self,
        location: BoardLocation,
        member: &str,
        method: &str,
        kind: RefusalKind,
        elapsed: Duration,
    ) {
        let handles = self.handles(location);
        swarm_board_dispatch::refused(&handles, member, method, kind, elapsed);
    }

    /// How structured ops read and write member text (#2279):
    /// composition's codec, the same for every board file, so reading a
    /// request resolves no file and builds no handles (#2279 review L5).
    pub(super) fn wire(&self) -> BoardWire {
        self.shared.wire
    }

    /// The handles for `location`: the ones built for the same file and
    /// the same log when the board kept them, else newly built by
    /// composition's builder (a context calls one board file; a hosted
    /// store may call another each time). The file becomes the most
    /// recently called; past [`BUILT_FILES`] files, the least recently
    /// called file's handles are dropped.
    ///
    /// The builder runs under the lock: a caller wanting another file waits
    /// while the handles are built, so two callers never build them twice.
    /// Building composes the graph and opens no connection (the repository
    /// opens one per call), so the wait is short.
    fn handles(&self, location: BoardLocation) -> Arc<SwarmBoardHandles> {
        let log = self.recording();
        let mut built = self.built();
        let kept = built
            .iter()
            .position(|file| file.location == location)
            .map(|index| built.remove(index))
            .filter(|file| same_log(file.log.as_ref(), log.as_ref()));
        let file = match kept {
            Some(file) => file,
            None => {
                let runs = log.clone().map(|log| Arc::new(RunFold::new(log)));
                let event_log = runs.clone().map(|runs| runs as Arc<dyn BoardOpLog>);
                let handles = Arc::new((self.shared.build)(location.clone(), event_log));
                debug_assert_eq!(
                    handles.telemetry.is_some(),
                    log.is_some(),
                    "composition's handles record in the log they were given"
                );
                Built {
                    location,
                    log,
                    runs,
                    handles,
                }
            }
        };
        let handles = file.handles.clone();
        built.push(file);
        if built.len() > BUILT_FILES {
            built.remove(0);
        }
        debug_assert!(
            built.len() <= BUILT_FILES,
            "a board keeps at most BUILT_FILES files"
        );
        handles
    }
}

/// Whether `kept` is the log `now` is: both none, or the same one.
fn same_log(kept: Option<&Arc<SessionLog>>, now: Option<&Arc<SessionLog>>) -> bool {
    match (kept, now) {
        (Some(kept), Some(now)) => Arc::ptr_eq(kept, now),
        (None, None) => true,
        (Some(_), None) | (None, Some(_)) => false,
    }
}

#[path = "swarm_bridge_board_log.rs"]
mod log;
use log::{RunFold, SessionLog};

#[cfg(test)]
#[path = "swarm_bridge_board_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "swarm_bridge_board_log_tests.rs"]
mod log_tests;
