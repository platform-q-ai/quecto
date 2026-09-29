//! The coordination board a [`super::SwarmContext`] or a
//! [`super::HostedStore`] reaches (#2278, epic #2265): composition's
//! handles builder, the handles it built for the last board file called,
//! and the event log each call is recorded in once the session has one.
//!
//! Every board call is a direct call of the Rust dispatcher
//! ([`swarm_board_dispatch::call`]) on the caller's thread: the store's
//! calls are blocking (a contended transaction waits up to its 500 ms busy
//! timeout), so every production caller is already on a blocking thread,
//! as it was for the Python interpreter this replaced. Nothing here
//! constructs a repository, a clock, an id source or a use case: the
//! builder is composition's (`composition::swarm::build_swarm_board_handles`).
use std::sync::{Arc, OnceLock};

use serde_json::Value;

use crate::application::swarm::dto::BoardLocation;
use crate::application::swarm::ports::BoardOpLog;
use crate::domain::error::DomainError;
use crate::infrastructure::persistence::audit_log::AuditLog;
use crate::infrastructure::tools::swarm_board_dispatch::SwarmBoardHandles;

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
    session_log: Option<SwarmBoardOpLogBuilder>,
    event_log: OnceLock<Arc<dyn BoardOpLog>>,
}

impl std::fmt::Debug for SwarmBoard {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SwarmBoard")
            .field("logged", &self.shared.event_log.get().is_some())
            .finish_non_exhaustive()
    }
}

impl SwarmBoard {
    /// The board over composition's `build`.
    pub fn new(build: SwarmBoardHandlesBuilder) -> Self {
        Self {
            shared: Arc::new(Shared {
                build,
                session_log: None,
                event_log: OnceLock::new(),
            }),
        }
    }

    /// The board over composition's `build`, recording in a session's
    /// event log through `session_log` ([`Self::record_in_session`]).
    pub fn with_session_log(
        build: SwarmBoardHandlesBuilder,
        session_log: SwarmBoardOpLogBuilder,
    ) -> Self {
        Self {
            shared: Arc::new(Shared {
                build,
                session_log: Some(session_log),
                event_log: OnceLock::new(),
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

    /// Composition's builder (red stub, #2278).
    pub fn handles_builder(&self) -> SwarmBoardHandlesBuilder {
        self.shared.build
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
        // Red stub (#2278): still the Python interpreter.
        super::super::swarm_board_worker::call(
            &super::super::swarm_board_worker::Board {
                checkout: &location.checkout,
                database: &location.database,
                member,
            },
            &super::bootstrap_source(&location.database, &location.checkout, member),
            method,
            args,
        )
    }
}

#[cfg(test)]
#[path = "swarm_bridge_board_tests.rs"]
mod tests;
