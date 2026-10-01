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
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde_json::Value;

use crate::application::swarm::dto::BoardLocation;
use crate::application::swarm::ports::{BoardOpLog, SessionOpLog};
use crate::domain::error::DomainError;
use crate::domain::swarm::RefusalKind;
use crate::domain::swarm::watch::Nudge;
use crate::infrastructure::persistence::audit_log::AuditLog;
use crate::infrastructure::tools::swarm_board_dispatch::{
    self, BindingFaults, BoardWire, CallOrigin, SwarmBoardHandles, TELEMETRY_TARGET,
};

/// Builds the board handles over one board file, recording each call in
/// the event log given (composition's `board_op_log`, only while the
/// event log is on, #2303).
pub type SwarmBoardHandlesBuilder =
    fn(BoardLocation, Option<Arc<dyn BoardOpLog>>) -> SwarmBoardHandles;

/// The event log a session's board calls are recorded in, from its audit
/// log: composition's `board_op_log`, `None` unless the event log is on
/// (`telemetry.event_log.enabled`, owner decision T1, #2303).
pub type SwarmBoardOpLogBuilder = fn(bool, &AuditLog) -> Option<Arc<dyn SessionOpLog>>;

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
    /// The run watches' latches, one per board file a watch here waits on
    /// (#2390 review L2), by the file's path.
    watches: Mutex<Vec<(std::path::PathBuf, Arc<WatchNudges>)>>,
}

/// The run watch's nudge latch (#2390): one per board file a watch waits
/// on, shared by every context over the board, so this process's watch is
/// nudged by its own control changes to that file ([`SwarmBoard::call_as`])
/// and by the pushes its UDS handler receives. A latched flag, never a bare
/// notify: a nudge that comes before the watch waits is taken by that wait,
/// so none is lost. It also counts this process's ops in flight that may
/// change the run's control state (review M1), so a watch about to end can
/// wait for the local nudge of an op whose change it already read.
#[derive(Debug, Default)]
pub struct WatchNudges {
    state: Mutex<Latched>,
    nudged: Condvar,
}

#[derive(Debug, Default)]
struct Latched {
    pending: Option<Nudge>,
    in_flight: u32,
}

impl WatchNudges {
    fn latched(&self) -> MutexGuard<'_, Latched> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Latches `nudge` (a local one outranks a remote one held with it)
    /// and wakes the watch.
    pub fn nudge(&self, nudge: Nudge) {
        let mut latched = self.latched();
        latched.pending = Some(latched.pending.map_or(nudge, |held| held.merge(nudge)));
        self.nudged.notify_all();
    }

    /// Waits up to `timeout` for a nudge and takes it; at once when one is
    /// latched already, `None` when none came.
    pub fn wait(&self, timeout: Duration) -> Option<Nudge> {
        let (mut latched, waited) = self
            .nudged
            .wait_timeout_while(self.latched(), timeout, |latched| latched.pending.is_none())
            .unwrap_or_else(PoisonError::into_inner);
        let taken = latched.pending.take();
        debug_assert!(
            taken.is_some() || waited.timed_out(),
            "a wait that took no nudge ran its whole timeout"
        );
        taken
    }

    /// The rest of the rate floor after a tick a remote nudge woke (review
    /// round 2, L1): waits up to `left`, holding every remote nudge that
    /// comes meanwhile, and returns at once on a local one. Answers what it
    /// took, `None` when nothing came.
    pub fn wait_floor(&self, left: Duration) -> Option<Nudge> {
        let (mut latched, waited) = self
            .nudged
            .wait_timeout_while(self.latched(), left, |latched| match latched.pending {
                Some(Nudge::Local) => false,
                Some(Nudge::Remote) | None => true,
            })
            .unwrap_or_else(PoisonError::into_inner);
        let taken = latched.pending.take();
        debug_assert!(
            taken == Some(Nudge::Local) || waited.timed_out(),
            "the floor gives way only to a local nudge"
        );
        taken
    }

    /// A board op of this process that may end the run begins; it is
    /// counted until it finishes (or is dropped).
    pub fn control_op(self: &Arc<Self>) -> ControlOp {
        self.latched().in_flight += 1;
        ControlOp(Some(self.clone()))
    }

    /// The watch is about to end (review M1): waits up to `bound` for this
    /// process's control ops in flight to finish, then answers whether a
    /// local nudge was latched, taking it.
    pub fn take_late_local(&self, bound: Duration) -> bool {
        let (mut latched, waited) = self
            .nudged
            .wait_timeout_while(self.latched(), bound, |latched| latched.in_flight > 0)
            .unwrap_or_else(PoisonError::into_inner);
        debug_assert!(
            latched.in_flight == 0 || waited.timed_out(),
            "the watch stops waiting only once no op is in flight, or at the bound"
        );
        match latched.pending {
            Some(Nudge::Local) => {
                latched.pending = None;
                true
            }
            Some(Nudge::Remote) | None => false,
        }
    }

    fn finished(&self, changed: bool) {
        let mut latched = self.latched();
        debug_assert!(latched.in_flight > 0, "an op finishes once it began");
        latched.in_flight = latched.in_flight.saturating_sub(1);
        if changed {
            latched.pending = Some(Nudge::Local);
        }
        self.nudged.notify_all();
    }
}

/// A board op of this process that may change the run's control state, in
/// flight (review M1). Dropped unfinished (the call panicked), it changed
/// nothing.
#[derive(Debug)]
pub struct ControlOp(Option<Arc<WatchNudges>>);

impl ControlOp {
    /// The op ended, having changed the run's control state or not: a
    /// change latches the local nudge.
    pub fn finish(mut self, changed: bool) {
        if let Some(nudges) = self.0.take() {
            nudges.finished(changed);
        }
    }
}

impl Drop for ControlOp {
    fn drop(&mut self) {
        if let Some(nudges) = self.0.take() {
            nudges.finished(false);
        }
    }
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
                watches: Mutex::new(Vec::new()),
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
                watches: Mutex::new(Vec::new()),
            }),
        }
    }

    /// Measures and records every call from now on, the admission's
    /// included, when the event log was decided on before admission
    /// (`event_log_on`, #2313): the records are held until the session's
    /// log is open ([`Self::record_in_session`]), and written there first.
    /// With the event log off nothing is measured (owner decision T1).
    pub fn record_from_admission(&self, event_log_on: bool) {
        tracing::debug!(
            target: TELEMETRY_TARGET,
            event_log_on,
            "swarm board recording decided before admission"
        );
        if event_log_on {
            self.recording_or_pending();
        }
    }

    /// Records every later call in the session's audit `log` when the
    /// event log is on (`enabled`) and this board was given composition's
    /// `session_log`; whether it now records there. Otherwise the board
    /// records nothing from now on, and what it held is dropped: a board
    /// without `session_log` can never record in a session's log, so it
    /// stops too (#2313 review L6), rather than measure and hold forever.
    pub fn record_in_session(&self, enabled: bool, log: &AuditLog) -> bool {
        let Some(build) = self.shared.session_log else {
            self.stop_recording();
            return false;
        };
        match build(enabled, log) {
            Some(log) => self.record_in(log),
            None => {
                self.stop_recording();
                false
            }
        }
    }

    /// Records nothing from now on (#2313: the session keeps no event
    /// log), and drops what it held since admission.
    pub fn stop_recording(&self) {
        // The watch's ticks held so far were made while the board
        // recorded (#2338 review round 1): written before it stops.
        self.flush_all_watch_polls();
        *self.recording_lock() = None;
    }

    /// Writes the run watch's ticks every file's handles hold (#2338
    /// review round 1): the harness is exiting, or recording stops. Only
    /// an exit nobody can intercept (SIGKILL, the OOM killer) loses them.
    pub fn flush_all_watch_polls(&self) {
        let handles: Vec<_> = self
            .built()
            .iter()
            .map(|file| file.handles.clone())
            .collect();
        for handles in handles {
            swarm_board_dispatch::flush_watch_polls(&handles);
        }
    }

    /// Records every later call in `log`, the current session's event log
    /// (#2313: a later session's replaces an earlier one's); what the board
    /// held until now is written there first.
    pub fn record_in(&self, log: Arc<dyn SessionOpLog>) -> bool {
        self.recording_or_pending().bind(log);
        true
    }

    /// The session switched to the one whose audit log is `log` (#2313):
    /// a board recording in the departing session's log records in `log`
    /// from now on; one that records nothing still does not.
    pub fn follow_session(&self, log: &AuditLog) -> bool {
        let followed = match self.recording() {
            Some(_) => self.record_in_session(true, log),
            None => false,
        };
        tracing::debug!(
            target: TELEMETRY_TARGET,
            followed,
            "swarm board records follow the session"
        );
        followed
    }

    /// Writes the summary of the run on the file at `location` (#2313),
    /// once: this process's records the board folded for it, and the run's
    /// own totals `member`'s harness reads from the board now
    /// (`_run_totals`, recorded as the harness's own, #2313 review M1).
    /// `false` when the board records nothing, folded no run there, or
    /// wrote it already. A totals read that fails leaves the run-wide
    /// section `null`, with a warning, and the process's counts are still
    /// written.
    pub(super) fn summarize_run(&self, location: &BoardLocation, member: &str) -> bool {
        let file = self
            .built()
            .iter()
            .find(|file| file.location == *location)
            .and_then(|file| Some((file.runs.clone()?, file.handles.clone())));
        let Some((runs, handles)) = file.filter(|(runs, _)| runs.open()) else {
            return false;
        };
        let totals = self.call_as(
            location.clone(),
            member,
            "_run_totals",
            Value::Array(Vec::new()),
            CallOrigin::Harness,
        );
        let totals = match totals {
            Ok(totals) => Some(totals),
            Err(error) => {
                tracing::warn!(
                    target: TELEMETRY_TARGET,
                    %error,
                    "swarm run totals were not read for the run summary"
                );
                None
            }
        };
        // The watch's ticks still held are the run's calls too (#2338):
        // written, and so folded, before the summary is taken, and no later
        // tick is written until it is (review round 1).
        swarm_board_dispatch::with_watch_polls_flushed(&handles, || {
            runs.summarize_run(totals.as_ref())
        })
    }

    /// Writes the run watch's polls held for the file at `location`
    /// (#2338), if its handles hold any.
    pub(super) fn flush_watch_polls(&self, location: &BoardLocation) {
        let handles = self
            .built()
            .iter()
            .find(|file| file.location == *location)
            .map(|file| file.handles.clone());
        if let Some(handles) = handles {
            swarm_board_dispatch::flush_watch_polls(&handles);
        }
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

    /// The run watch's latch, shared by every clone of this board (#2390).
    /// Whether a watch here holds the latch of the board file `database`.
    pub fn watches(&self, database: &std::path::Path) -> bool {
        self.watched(database).is_some()
    }

    /// The run watch's latch for the board file `database` (#2390), made
    /// when none is held: the watch's, and the UDS handler's that nudges
    /// it. Every clone of this board shares it.
    pub fn watch_nudges(&self, database: &std::path::Path) -> Arc<WatchNudges> {
        let mut watches = self.watches_lock();
        if let Some((_, nudges)) = watches.iter().find(|(file, _)| file == database) {
            return nudges.clone();
        }
        let nudges = Arc::new(WatchNudges::default());
        watches.push((database.to_path_buf(), nudges.clone()));
        nudges
    }

    /// The latch a watch here holds for the board file `database`, if any:
    /// a control change to a file nobody here watches nudges nothing.
    fn watched(&self, database: &std::path::Path) -> Option<Arc<WatchNudges>> {
        self.watches_lock()
            .iter()
            .find(|(file, _)| file == database)
            .map(|(_, nudges)| nudges.clone())
    }

    fn watches_lock(&self) -> MutexGuard<'_, Vec<(std::path::PathBuf, Arc<WatchNudges>)>> {
        self.shared
            .watches
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
        // #2390: a control change to a board file a watch here waits on
        // nudges that watch, which reads it at once and tells the other
        // members. An op that may end the run is counted in flight first
        // (review M1), so a watch about to end waits for its nudge; the
        // ops every model request and tool call make are not (round 2, L2).
        let watched = self.watched(&location.database);
        let in_flight = match (swarm_board_dispatch::may_end_run(method), &watched) {
            (true, Some(nudges)) => Some(nudges.control_op()),
            (true, None) | (false, _) => None,
        };
        let handles = self.handles(location);
        let (answer, controls_run) =
            swarm_board_dispatch::call_deciding(&handles, member, method, args, origin);
        match (in_flight, watched, controls_run) {
            (Some(op), _, changed) => op.finish(changed),
            (None, Some(nudges), true) => nudges.nudge(Nudge::Local),
            (None, Some(_), false) | (None, None, _) => {}
        }
        answer.map_err(|refusal| {
            DomainError::Tool(format!(
                "swarm: {}",
                Value::String(refusal.message().to_owned())
            ))
        })
    }

    /// Records `method`, called by `member` on the file at `location`, as
    /// refused with `kind` before it reached the board (#2279: a structured
    /// op's argument text or its running gate), with what it found wrong in
    /// the arguments (#2341), `elapsed` after it began.
    pub(super) fn refused(
        &self,
        location: BoardLocation,
        member: &str,
        method: &str,
        (kind, arguments): (RefusalKind, BindingFaults),
        elapsed: Duration,
    ) {
        let handles = self.handles(location);
        swarm_board_dispatch::refused(&handles, member, method, kind, arguments, elapsed);
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
        let found = built
            .iter()
            .position(|file| file.location == location)
            .map(|index| built.remove(index));
        let kept = match found {
            Some(file) if same_log(file.log.as_ref(), log.as_ref()) => Some(file),
            Some(replaced) => {
                // Its watch polls were made while its log was current
                // (#2338): written there before its handles go.
                swarm_board_dispatch::flush_watch_polls(&replaced.handles);
                None
            }
            None => None,
        };
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
            let dropped = built.remove(0);
            swarm_board_dispatch::flush_watch_polls(&dropped.handles);
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
