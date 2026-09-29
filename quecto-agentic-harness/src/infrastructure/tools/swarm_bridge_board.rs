//! The coordination board a [`super::SwarmContext`] or a
//! [`super::HostedStore`] reaches (#2278, epic #2265): composition's
//! handles builder, the handles it built for each board file recently
//! called, and the event log each call is recorded in once the session has one.
//!
//! Every board call is a direct call of the Rust dispatcher
//! ([`swarm_board_dispatch::call`]) on the caller's thread: the store's
//! calls are blocking (a contended transaction waits up to its 500 ms busy
//! timeout), so every caller makes it on the blocking pool or outside any
//! runtime, as it did for the Python interpreter this replaced; a debug
//! build asserts it on every call ([`SwarmBoard::call`]). Nothing here
//! constructs a repository, a clock, an id source or a use case: the
//! builder is composition's (`composition::swarm::build_swarm_board_handles`).
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde_json::Value;

use crate::application::swarm::dto::BoardLocation;
use crate::application::swarm::ports::BoardOpLog;
use crate::domain::error::DomainError;
use crate::domain::swarm::RefusalKind;
use crate::infrastructure::persistence::audit_log::AuditLog;
use crate::infrastructure::tools::swarm_board_dispatch::{self, BoardWire, SwarmBoardHandles};

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
    event_log: OnceLock<Arc<dyn BoardOpLog>>,
    /// The handles built per file, the most recently called last; at
    /// most [`BUILT_FILES`].
    built: Mutex<Vec<Built>>,
}

/// The handles built for one board file, with the event log or without.
struct Built {
    location: BoardLocation,
    logged: bool,
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
            .field("logged", &self.shared.event_log.get().is_some())
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
                event_log: OnceLock::new(),
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
                event_log: OnceLock::new(),
                built: Mutex::new(Vec::new()),
            }),
        }
    }

    /// Records every later call in the session's audit `log` when the
    /// event log is on (`enabled`) and this board was given composition's
    /// `session_log`; whether it now records there.
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

    /// Records every later call in `log` (the session's event log, once
    /// it is open). A board records in one log only: `false` when it
    /// already had one, which it keeps.
    pub fn record_in(&self, log: Arc<dyn BoardOpLog>) -> bool {
        self.shared.event_log.set(log).is_ok()
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
        // A board call blocks (a contended transaction waits up to the
        // store's busy timeout): every caller makes it on the blocking pool
        // (`call_work::spawn_blocking_in_call`) or outside any runtime,
        // never on an async worker (#2278 review L6).
        debug_assert!(
            crate::infrastructure::tools::call_work::may_block(),
            "a board call ({method}) is made off the async workers (#2278)"
        );
        let handles = self.handles(location);
        swarm_board_dispatch::call(&handles, member, method, args).map_err(|refusal| {
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
        let event_log = self.shared.event_log.get().cloned();
        let logged = event_log.is_some();
        let mut built = self
            .shared
            .built
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let kept = built
            .iter()
            .position(|file| file.location == location)
            .map(|index| built.remove(index))
            .filter(|file| file.logged == logged);
        let file = match kept {
            Some(file) => file,
            None => {
                let handles = Arc::new((self.shared.build)(location.clone(), event_log));
                debug_assert_eq!(
                    handles.telemetry.is_some(),
                    logged,
                    "composition's handles record in the log they were given"
                );
                Built {
                    location,
                    logged,
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

#[cfg(test)]
#[path = "swarm_bridge_board_tests.rs"]
mod tests;
